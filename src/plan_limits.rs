use crate::config;
use crate::export::write_atomic;
use crate::platform;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub used_percentage: f64,
    pub resets_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanLimits {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
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
        captured_at: now,
    })
}

pub fn run_status_line() -> i32 {
    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);
    if let Some(limits) = captured(&input, unix_now())
        && let Ok(text) = serde_json::to_string(&limits)
    {
        let _ = write_atomic(&path(), &text);
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
    const THEIRS: &str = "powershell -File \"C:\\Users\\me\\line.ps1\"";

    fn settings_in(dir: &TempDir) -> PathBuf {
        unsafe { std::env::set_var("CLAUDE_USAGE_CONFIG_DIR", dir.path()) };
        dir.path().join(SETTINGS_FILE)
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
