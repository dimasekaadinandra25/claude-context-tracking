use crate::config;
use crate::export::write_atomic;
use crate::platform;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const STATUS_LINE_FLAG: &str = "--statusline";
const FILE_NAME: &str = "plan_limits.json";
const SETTINGS_FILE: &str = "settings.json";
const BACKUP_FILE: &str = "settings.json.usage-monitor.bak";
const STATUS_LINE_KEY: &str = "statusLine";
const COMMAND_KEY: &str = "command";
const TYPE_KEY: &str = "type";
const COMMAND_TYPE: &str = "command";
const FIVE_HOUR_LABEL: &str = "5h";
const SEVEN_DAY_LABEL: &str = "7d";
const ACCOUNT_FILE: &str = ".claude.json";
const CREDENTIALS_FILE: &str = ".credentials.json";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const USAGE_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const USAGE_TIMEOUT: Duration = Duration::from_secs(15);
const TOKEN_MARGIN_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub used_percentage: f64,
    pub resets_at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelLimit {
    pub name: String,
    pub used_percentage: f64,
    pub resets_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanLimits {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelLimit>,
    pub captured_at: u64,
}

impl PlanLimits {
    pub fn live(&self, now: u64) -> Vec<(&'static str, Window)> {
        [
            (FIVE_HOUR_LABEL, self.five_hour),
            (SEVEN_DAY_LABEL, self.seven_day),
        ]
        .into_iter()
        .filter_map(|(label, window)| Some((label, window?)))
        .filter(|(_, window)| window.resets_at > now)
        .collect()
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct StatusInput {
    rate_limits: Option<RateLimits>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct RateLimits {
    five_hour: Option<Window>,
    seven_day: Option<Window>,
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn path() -> PathBuf {
    config::dir().join(FILE_NAME)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Account {
    pub email: String,
    pub plan: String,
    pub tier: String,
}

#[derive(Deserialize)]
struct ClaudeJson {
    #[serde(rename = "oauthAccount")]
    oauth_account: Option<OAuthAccount>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OAuthAccount {
    email_address: Option<String>,
    plan_display_name: Option<String>,
    user_rate_limit_tier: Option<String>,
    organization_rate_limit_tier: Option<String>,
}

pub fn account() -> Option<Account> {
    let file = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir).join(ACCOUNT_FILE),
        None => dirs::home_dir()?.join(ACCOUNT_FILE),
    };
    account_from(&std::fs::read_to_string(file).ok()?)
}

fn account_from(text: &str) -> Option<Account> {
    let oauth = serde_json::from_str::<ClaudeJson>(text.trim_start_matches('\u{feff}'))
        .ok()?
        .oauth_account?;
    let account = Account {
        email: oauth.email_address.unwrap_or_default(),
        plan: oauth.plan_display_name.unwrap_or_default(),
        tier: oauth
            .user_rate_limit_tier
            .filter(|tier| !tier.is_empty())
            .or(oauth.organization_rate_limit_tier)
            .unwrap_or_default(),
    };
    (!account.email.is_empty() || !account.tier.is_empty()).then_some(account)
}

#[derive(Deserialize)]
struct Credentials {
    #[serde(rename = "claudeAiOauth")]
    oauth: Option<OAuthToken>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthToken {
    access_token: String,
    expires_at: u64,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct UsageResponse {
    five_hour: Option<UsageWindow>,
    seven_day: Option<UsageWindow>,
    limits: Vec<serde_json::Value>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct UsageLimit {
    kind: String,
    percent: Option<f64>,
    resets_at: Option<String>,
    scope: Option<LimitScope>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct LimitScope {
    model: Option<ScopeModel>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ScopeModel {
    display_name: Option<String>,
}

const WEEKLY_SCOPED: &str = "weekly_scoped";

impl UsageLimit {
    fn model_limit(self) -> Option<ModelLimit> {
        if self.kind != WEEKLY_SCOPED {
            return None;
        }
        Some(ModelLimit {
            name: self
                .scope?
                .model?
                .display_name
                .filter(|name| !name.is_empty())?,
            used_percentage: self.percent?,
            resets_at: crate::session::unix_seconds(&self.resets_at?)?,
        })
    }
}

#[derive(Deserialize)]
struct UsageWindow {
    utilization: f64,
    resets_at: String,
}

impl UsageWindow {
    fn window(&self) -> Option<Window> {
        Some(Window {
            used_percentage: self.utilization,
            resets_at: crate::session::unix_seconds(&self.resets_at)?,
        })
    }
}

fn access_token(credentials: &str, now: u64) -> Option<String> {
    let token = serde_json::from_str::<Credentials>(credentials.trim_start_matches('\u{feff}'))
        .ok()?
        .oauth?;
    (token.expires_at > now * 1_000 + TOKEN_MARGIN_MS && !token.access_token.is_empty())
        .then_some(token.access_token)
}

fn limits_from(body: &str, now: u64) -> Option<PlanLimits> {
    let usage = serde_json::from_str::<UsageResponse>(body).ok()?;
    let five_hour = usage.five_hour.as_ref().and_then(UsageWindow::window);
    let seven_day = usage.seven_day.as_ref().and_then(UsageWindow::window);
    let models = usage
        .limits
        .into_iter()
        .filter_map(|limit| {
            serde_json::from_value::<UsageLimit>(limit)
                .ok()?
                .model_limit()
        })
        .collect();
    (five_hour.is_some() || seven_day.is_some()).then_some(PlanLimits {
        five_hour,
        seven_day,
        models,
        captured_at: now,
    })
}

fn fetch(now: u64) -> Option<PlanLimits> {
    let credentials =
        std::fs::read_to_string(platform::claude_dir().join(CREDENTIALS_FILE)).ok()?;
    let token = access_token(&credentials, now)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(USAGE_TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .get(USAGE_URL)
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", OAUTH_BETA)
        .call()
        .ok()?;
    limits_from(&response.body_mut().read_to_string().ok()?, now)
}

#[derive(Default)]
pub struct UsagePoller {
    last_run: Option<Instant>,
    running: Option<JoinHandle<()>>,
}

impl UsagePoller {
    pub fn tick(&mut self) {
        if self.running.as_ref().is_some_and(|run| !run.is_finished())
            || self
                .last_run
                .is_some_and(|at| at.elapsed() < USAGE_POLL_INTERVAL)
        {
            return;
        }
        self.last_run = Some(Instant::now());
        self.running = Some(std::thread::spawn(|| {
            if let Some(limits) = fetch(unix_now())
                && let Ok(text) = serde_json::to_string(&limits)
            {
                let _ = write_atomic(&path(), &text);
            }
        }));
    }
}

pub fn load() -> Option<PlanLimits> {
    serde_json::from_str(&std::fs::read_to_string(path()).ok()?).ok()
}

fn captured(input: &str, now: u64) -> Option<PlanLimits> {
    let limits = serde_json::from_str::<StatusInput>(input)
        .ok()?
        .rate_limits?;
    (limits.five_hour.is_some() || limits.seven_day.is_some()).then_some(PlanLimits {
        five_hour: limits.five_hour,
        seven_day: limits.seven_day,
        models: Vec::new(),
        captured_at: now,
    })
}

pub fn run_status_line() -> i32 {
    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);
    if let Some(mut limits) = captured(&input, unix_now()) {
        limits.models = load().map(|saved| saved.models).unwrap_or_default();
        if let Ok(text) = serde_json::to_string(&limits) {
            let _ = write_atomic(&path(), &text);
        }
    }
    chain(&config::saved().chained_status_line, &input)
}

fn chain(command: &str, input: &str) -> i32 {
    if command.trim().is_empty() {
        return 0;
    }
    let Ok(mut child) = shell(command).stdin(Stdio::piped()).spawn() else {
        return 1;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    child
        .wait()
        .ok()
        .and_then(|status| status.code())
        .unwrap_or(1)
}

#[cfg(windows)]
fn shell(command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut shell = match platform::git_bash() {
        Some(bash) => {
            let mut shell = Command::new(bash);
            shell.arg("-c");
            shell
        }
        None => {
            let mut shell = Command::new("powershell");
            shell.args(["-NoProfile", "-Command"]);
            shell
        }
    };
    shell.arg(command).creation_flags(CREATE_NO_WINDOW);
    shell
}

#[cfg(not(windows))]
fn shell(command: &str) -> Command {
    let mut shell = Command::new("sh");
    shell.args(["-c", command]);
    shell
}

fn settings_path() -> PathBuf {
    platform::claude_dir().join(SETTINGS_FILE)
}

fn read_settings(path: &Path) -> io::Result<Map<String, Value>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(error),
    };
    serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Claude Code's settings.json can't be read, so it was left unchanged",
        )
    })
}

fn write_settings(path: &Path, settings: &Map<String, Value>) -> io::Result<()> {
    write_atomic(path, &(serde_json::to_string_pretty(settings)? + "\n"))
}

fn status_command(settings: &Map<String, Value>) -> Option<&str> {
    settings.get(STATUS_LINE_KEY)?.get(COMMAND_KEY)?.as_str()
}

fn is_ours(command: &str) -> bool {
    command.contains(STATUS_LINE_FLAG) && command.contains(env!("CARGO_PKG_NAME"))
}

fn set_status_command(settings: &mut Map<String, Value>, command: &str) {
    let entry = settings
        .entry(STATUS_LINE_KEY)
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    if let Some(status_line) = entry.as_object_mut() {
        status_line.insert(TYPE_KEY.to_owned(), COMMAND_TYPE.into());
        status_line.insert(COMMAND_KEY.to_owned(), command.into());
    }
}

fn our_command(exe: &Path, git_bash: bool) -> String {
    let exe = exe.to_string_lossy().replace('\\', "/");
    let call = if git_bash { "" } else { "& " };
    format!("{call}\"{exe}\" {STATUS_LINE_FLAG}")
}

#[cfg(windows)]
fn runs_in_git_bash() -> bool {
    platform::git_bash().is_some()
}

#[cfg(not(windows))]
fn runs_in_git_bash() -> bool {
    true
}

fn current_command() -> io::Result<String> {
    Ok(our_command(&std::env::current_exe()?, runs_in_git_bash()))
}

pub fn is_enabled() -> bool {
    read_settings(&settings_path())
        .is_ok_and(|settings| status_command(&settings).is_some_and(is_ours))
}

pub fn set(enabled: bool) -> io::Result<()> {
    apply(&settings_path(), enabled, &current_command()?)
}

fn apply(settings_path: &Path, enabled: bool, command: &str) -> io::Result<()> {
    let mut settings = read_settings(settings_path)?;
    let current = status_command(&settings).map(str::to_owned);
    let ours = current.as_deref().is_some_and(is_ours);
    if !enabled {
        let _ = std::fs::remove_file(path());
    }
    if enabled == ours {
        return Ok(());
    }
    if settings_path.exists() {
        std::fs::copy(settings_path, settings_path.with_file_name(BACKUP_FILE))?;
    }
    if enabled {
        config::save_chained_status_line(current.as_deref().unwrap_or_default())?;
        set_status_command(&mut settings, command);
    } else {
        match config::saved().chained_status_line.as_str() {
            "" => {
                settings.remove(STATUS_LINE_KEY);
            }
            previous => set_status_command(&mut settings, previous),
        }
    }
    write_settings(settings_path, &settings)?;
    if !enabled {
        config::save_chained_status_line("")?;
    }
    Ok(())
}

pub fn keep_path_current() -> io::Result<()> {
    let path = settings_path();
    let mut settings = read_settings(&path)?;
    let command = current_command()?;
    match status_command(&settings) {
        Some(current) if is_ours(current) && current != command => {
            set_status_command(&mut settings, &command);
            write_settings(&path, &settings)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{TempDir, lock_env};

    const OURS: &str = "\"C:/Apps/claude-usage-monitor.exe\" --statusline";

    #[test]
    fn usage_endpoint_reply_becomes_plan_limits() {
        let body = r#"{"five_hour":null,"seven_day":{"utilization":19.5,"resets_at":"2026-11-18T15:00:00.033648+00:00"},"limits":[
            {"kind":"weekly_all","percent":19,"resets_at":"2026-11-18T15:00:00Z","scope":null},
            {"kind":"weekly_scoped","percent":4,"resets_at":"2026-11-18T15:00:00+00:00","scope":{"model":{"id":null,"display_name":"Fable"},"surface":null},"is_active":false},
            {"kind":"weekly_scoped","percent":"bad","scope":{"model":{"display_name":"Odd"}}},
            {"kind":"weekly_scoped","percent":1,"resets_at":"2026-11-18T15:00:00Z","scope":{"surface":"web"}}
        ]}"#;
        let limits = limits_from(body, 7).unwrap();
        assert_eq!(
            limits.models,
            [ModelLimit {
                name: "Fable".into(),
                used_percentage: 4.0,
                resets_at: 1_795_014_000
            }]
        );
        assert_eq!(limits.five_hour, None);
        assert_eq!(
            limits.seven_day,
            Some(Window {
                used_percentage: 19.5,
                resets_at: 1_795_014_000
            })
        );
        assert_eq!(limits.captured_at, 7);
        assert_eq!(
            limits_from(r#"{"five_hour":null,"seven_day":null}"#, 7),
            None
        );
        assert_eq!(limits_from("<html>", 7), None);
    }

    #[test]
    fn only_a_token_that_is_still_valid_is_used() {
        let credentials = |expires: u64| {
            format!(
                r#"{{"claudeAiOauth":{{"accessToken":"tok","refreshToken":"r","expiresAt":{expires}}}}}"#
            )
        };
        let now = 1_000;
        assert_eq!(
            access_token(&credentials(now * 1_000 + 3_600_000), now).as_deref(),
            Some("tok")
        );
        assert_eq!(access_token(&credentials(now * 1_000 + 30_000), now), None);
        assert_eq!(access_token("{}", now), None);
    }
    const THEIRS: &str = "powershell -File \"C:\\Users\\me\\line.ps1\"";

    fn settings_in(dir: &TempDir) -> PathBuf {
        unsafe { std::env::set_var("CLAUDE_USAGE_CONFIG_DIR", dir.path()) };
        dir.path().join(SETTINGS_FILE)
    }

    #[test]
    fn reads_the_signed_in_account_and_prefers_the_user_tier() {
        let text = "\u{feff}{\"numStartups\":3,\"oauthAccount\":{\"emailAddress\":\"a@b.c\",\"planDisplayName\":\"Max\",\"userRateLimitTier\":null,\"organizationRateLimitTier\":\"default_claude_max_20x\",\"accountUuid\":\"x\"}}";
        assert_eq!(
            account_from(text),
            Some(Account {
                email: "a@b.c".into(),
                plan: "Max".into(),
                tier: "default_claude_max_20x".into(),
            })
        );
        let user = r#"{"oauthAccount":{"emailAddress":"a@b.c","userRateLimitTier":"default_claude_pro","organizationRateLimitTier":"org"}}"#;
        assert_eq!(account_from(user).unwrap().tier, "default_claude_pro");
        assert_eq!(account_from(r#"{"numStartups":3}"#), None);
        assert_eq!(account_from(r#"{"oauthAccount":{}}"#), None);
        assert_eq!(account_from("not json"), None);
    }

    #[test]
    fn captures_rate_limits_from_status_line_input() {
        let input = r#"{"model":{"id":"x"},"rate_limits":{
            "five_hour":{"used_percentage":23.5,"resets_at":1000},
            "seven_day":{"used_percentage":41.2,"resets_at":2000},
            "spend_limit":{"used_percentage":1.0,"resets_at":3000}}}"#;
        let limits = captured(input, 500).unwrap();
        assert_eq!(limits.captured_at, 500);
        assert_eq!(limits.five_hour.unwrap().used_percentage, 23.5);
        assert_eq!(limits.seven_day.unwrap().resets_at, 2000);
        assert_eq!(captured(r#"{"model":{"id":"x"}}"#, 500), None);
        assert_eq!(captured(r#"{"rate_limits":{}}"#, 500), None);
        assert_eq!(captured("not json", 500), None);
    }

    #[test]
    fn expired_windows_are_not_live() {
        let limits = PlanLimits {
            five_hour: Some(Window {
                used_percentage: 10.0,
                resets_at: 100,
            }),
            seven_day: Some(Window {
                used_percentage: 20.0,
                resets_at: 300,
            }),
            models: Vec::new(),
            captured_at: 50,
        };
        assert_eq!(limits.live(50).len(), 2);
        let later = limits.live(200);
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].0, "7d");
    }

    #[test]
    fn command_fits_the_shell_claude_code_uses() {
        let exe = Path::new(r"C:\Program Files\App\claude-usage-monitor.exe");
        assert_eq!(
            our_command(exe, true),
            "\"C:/Program Files/App/claude-usage-monitor.exe\" --statusline"
        );
        assert_eq!(
            our_command(exe, false),
            "& \"C:/Program Files/App/claude-usage-monitor.exe\" --statusline"
        );
    }

    #[test]
    fn enabling_keeps_other_settings_and_disabling_restores_the_old_line() {
        let _env = lock_env();
        let dir = TempDir::new("limits-toggle");
        let settings = settings_in(&dir);
        let original = format!(
            "{{\n  \"zeta\": 1,\n  \"statusLine\": {{\n    \"type\": \"command\",\n    \"command\": {},\n    \"refreshInterval\": 5\n  }},\n  \"alpha\": {{\n    \"on\": true\n  }}\n}}\n",
            serde_json::to_string(THEIRS).unwrap()
        );
        std::fs::write(&settings, &original).unwrap();

        apply(&settings, true, OURS).unwrap();
        let enabled = read_settings(&settings).unwrap();
        assert_eq!(status_command(&enabled), Some(OURS));
        assert_eq!(enabled[STATUS_LINE_KEY]["refreshInterval"], 5);
        assert_eq!(
            enabled.keys().collect::<Vec<_>>(),
            ["zeta", "statusLine", "alpha"]
        );
        assert_eq!(config::saved().chained_status_line, THEIRS);
        assert_eq!(
            std::fs::read_to_string(settings.with_file_name(BACKUP_FILE)).unwrap(),
            original
        );

        apply(&settings, true, OURS).unwrap();
        assert_eq!(config::saved().chained_status_line, THEIRS);

        std::fs::write(path(), "{}").unwrap();
        apply(&settings, false, OURS).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), original);
        assert_eq!(config::saved().chained_status_line, "");
        assert!(!path().exists());
    }

    #[test]
    fn enabling_without_a_status_line_removes_it_again_when_disabled() {
        let _env = lock_env();
        let dir = TempDir::new("limits-empty");
        let settings = settings_in(&dir);
        apply(&settings, true, OURS).unwrap();
        assert_eq!(
            status_command(&read_settings(&settings).unwrap()),
            Some(OURS)
        );
        apply(&settings, false, OURS).unwrap();
        assert!(
            !read_settings(&settings)
                .unwrap()
                .contains_key(STATUS_LINE_KEY)
        );
    }

    #[test]
    fn disabling_leaves_a_status_line_someone_else_set_later() {
        let _env = lock_env();
        let dir = TempDir::new("limits-other");
        let settings = settings_in(&dir);
        let mut other = Map::new();
        set_status_command(&mut other, THEIRS);
        write_settings(&settings, &other).unwrap();
        apply(&settings, false, OURS).unwrap();
        assert_eq!(
            status_command(&read_settings(&settings).unwrap()),
            Some(THEIRS)
        );
    }

    #[test]
    fn unreadable_settings_are_never_overwritten() {
        let _env = lock_env();
        let dir = TempDir::new("limits-broken");
        let settings = settings_in(&dir);
        std::fs::write(&settings, "{ \"statusLine\": ").unwrap();
        assert!(apply(&settings, true, OURS).is_err());
        assert_eq!(
            std::fs::read_to_string(&settings).unwrap(),
            "{ \"statusLine\": "
        );
    }

    #[test]
    fn settings_with_byte_order_mark_are_read() {
        let _env = lock_env();
        let dir = TempDir::new("limits-bom");
        let settings = settings_in(&dir);
        std::fs::write(&settings, "\u{feff}{\"alpha\": 1}").unwrap();
        apply(&settings, true, OURS).unwrap();
        assert_eq!(read_settings(&settings).unwrap()["alpha"], 1);
    }
}
