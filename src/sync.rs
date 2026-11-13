use crate::config::{self, Config, SyncIdentity};
use crate::index::Index;
use crate::overhead::{self, Overhead};
use crate::plan_limits::{self, Account, PlanLimits};
use crate::platform::{self, APP_VERSION};
use crate::session::{Session, unix_seconds};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

const SYNC_INTERVAL: Duration = Duration::from_secs(30 * 60);
const PLAN_SYNC_GAP: Duration = Duration::from_secs(5 * 60);
const STARTUP_DELAY: Duration = Duration::from_secs(60);
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_SESSIONS: usize = 200;
const MAX_HOUR_ROWS: usize = 20_000;
const INGEST_PATH: &str = "/usage/v1/sessions";
const WATERMARK_OVERLAP: Duration = Duration::from_secs(60 * 60);

#[derive(Serialize)]
struct Payload<'a> {
    machine: &'a str,
    app_version: &'static str,
    sessions: Vec<SessionPayload<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan_limits: Option<PlanLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<Account>,
}

#[derive(Serialize)]
struct SessionPayload<'a> {
    session_id: &'a str,
    project: &'a str,
    title: &'a str,
    model: &'a str,
    context_window: u64,
    start_context: u64,
    latest_context: u64,
    peak_context: u64,
    new_context: u64,
    requests: u32,
    compactions: u32,
    first_at: &'a str,
    latest_at: &'a str,
    hours: Vec<HourPayload<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    overhead: Option<OverheadPayload>,
    tools: Vec<ToolPayload<'a>>,
}

#[derive(Serialize)]
struct ToolPayload<'a> {
    hour: &'a str,
    tool: &'a str,
    calls: u32,
    lines_added: u64,
    lines_removed: u64,
    commits: u32,
}

#[derive(Serialize)]
struct OverheadPayload {
    system_tokens: u64,
    memory_tokens: u64,
    agents_tokens: u64,
    skills_tokens: u64,
}

#[derive(Serialize)]
struct HourPayload<'a> {
    hour: &'a str,
    model: &'a str,
    new_context: u64,
    requests: u32,
    output_tokens: u64,
    subagent_requests: u32,
    subagent_output_tokens: u64,
    input_tokens: u64,
    cache_write_tokens: u64,
    cache_read_tokens: u64,
}

impl<'a> Payload<'a> {
    fn new(
        machine: &'a str,
        sessions: &[&'a Session],
        hide_titles: bool,
        overheads: &HashMap<String, Overhead>,
    ) -> Self {
        Self {
            machine,
            app_version: APP_VERSION,
            sessions: sessions
                .iter()
                .map(|s| SessionPayload::new(s, hide_titles, overheads.get(&s.project)))
                .collect(),
            plan_limits: plan_limits::load(),
            account: plan_limits::account(),
        }
    }
}

impl<'a> SessionPayload<'a> {
    fn new(s: &'a Session, hide_titles: bool, overhead: Option<&Overhead>) -> Self {
        Self {
            session_id: &s.id,
            project: if hide_titles {
                ""
            } else {
                folder_name(&s.project)
            },
            title: if hide_titles { "" } else { s.name() },
            model: &s.model,
            context_window: s.context_window,
            start_context: s.base_context,
            latest_context: s.latest_context,
            peak_context: s.peak_context,
            new_context: s.lifetime_new_context(),
            requests: s.requests,
            compactions: s.compactions_lifetime,
            first_at: &s.first_at,
            latest_at: &s.latest_at,
            hours: s
                .hours
                .iter()
                .map(|((hour, model), usage)| HourPayload {
                    hour,
                    model,
                    new_context: usage.new_context,
                    requests: usage.requests,
                    output_tokens: usage.output_tokens,
                    subagent_requests: usage.subagent_requests,
                    subagent_output_tokens: usage.subagent_output_tokens,
                    input_tokens: usage.input_tokens,
                    cache_write_tokens: usage.cache_write_tokens,
                    cache_read_tokens: usage.cache_read_tokens,
                })
                .collect(),
            overhead: overhead
                .filter(|_| s.has_requests())
                .map(|o| OverheadPayload {
                    system_tokens: o.system_tokens(s),
                    memory_tokens: o.memory_tokens(),
                    agents_tokens: o.agent_tokens(),
                    skills_tokens: o.skills,
                }),
            tools: s
                .tools
                .iter()
                .map(|((hour, tool), usage)| ToolPayload {
                    hour,
                    tool,
                    calls: usage.calls,
                    lines_added: usage.lines_added,
                    lines_removed: usage.lines_removed,
                    commits: usage.commits,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Rejection {
    pub session_id: String,
    pub reason: String,
}

#[derive(Deserialize)]
struct Envelope {
    data: IngestResult,
}

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
struct IngestResult {
    rejected: Vec<Rejection>,
    latest_app_version: Option<String>,
}

type PlanReading = Option<Vec<Option<f64>>>;

fn plan_reading() -> PlanReading {
    plan_limits::load().map(|limits| {
        [limits.five_hour, limits.seven_day]
            .map(|window| window.map(|w| w.used_percentage))
            .into_iter()
            .chain(limits.models.iter().map(|m| Some(m.used_percentage)))
            .collect()
    })
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse().ok())
        .collect()
}

pub fn is_newer(version: &str, than: &str) -> bool {
    version_parts(version) > version_parts(than)
}

#[derive(Clone, Debug, PartialEq)]
pub enum SyncStatus {
    Off,
    Waiting,
    Syncing,
    Synced {
        at: SystemTime,
        rejected: Vec<Rejection>,
        update: Option<String>,
    },
    KeyRejected,
    Failed(String),
}

pub struct Syncer {
    started: Instant,
    last_run: Option<Instant>,
    last_identity: SyncIdentity,
    last_plan: PlanReading,
    requested: bool,
    running: Option<JoinHandle<()>>,
}

impl Syncer {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            last_run: None,
            last_identity: config::load().sync_identity(),
            last_plan: plan_reading(),
            requested: false,
            running: None,
        }
    }

    pub fn request(&mut self) {
        self.requested = true;
    }

    pub fn tick(
        &mut self,
        index: &Index,
        machine: &str,
        report: impl Fn(SyncStatus) + Send + 'static,
    ) {
        if self.running.as_ref().is_some_and(|run| !run.is_finished()) {
            return;
        }
        let config = config::load();
        let plan = plan_reading();
        let due = is_due(
            self.last_run.map(|at| at.elapsed()),
            self.started.elapsed(),
            plan != self.last_plan,
        );
        if !due && !self.requested && config.sync_identity() == self.last_identity {
            return;
        }
        self.requested = false;
        self.last_plan = plan;
        self.last_run = Some(Instant::now());
        self.last_identity = config.sync_identity();
        if !config.is_enabled() {
            report(SyncStatus::Off);
            return;
        }
        report(SyncStatus::Syncing);
        let (sessions, machine) = (index.snapshot(), machine.to_owned());
        self.running = Some(std::thread::spawn(move || {
            report(run_with(post, estimate, &config, &sessions, &machine));
        }));
    }

    pub fn finish(&mut self, index: &Index, machine: &str) {
        if let Some(run) = self.running.take() {
            let _ = run.join();
        }
        run_with(post, estimate, &config::load(), &index.snapshot(), machine);
    }
}

fn is_due(since_last: Option<Duration>, since_start: Duration, plan_changed: bool) -> bool {
    match since_last {
        Some(elapsed) => elapsed >= SYNC_INTERVAL || (plan_changed && elapsed >= PLAN_SYNC_GAP),
        None => since_start >= STARTUP_DELAY,
    }
}

pub fn chunks<'a>(sessions: &'a [Session], watermark: Option<&str>) -> Vec<Vec<&'a Session>> {
    let mut pending: Vec<&Session> = sessions
        .iter()
        .filter(|s| watermark.is_none_or(|w| after_overlap(s.last_activity(), w)))
        .filter(|s| has_usage(s))
        .collect();
    pending.sort_by(|a, b| a.last_activity().cmp(b.last_activity()));
    let mut chunks: Vec<Vec<&Session>> = Vec::new();
    let mut rows = 0;
    for session in pending {
        let full = chunks.last().is_none_or(|chunk| {
            chunk.len() >= MAX_SESSIONS || rows + row_count(session) > MAX_HOUR_ROWS
        });
        if full {
            chunks.push(Vec::new());
            rows = 0;
        }
        rows += row_count(session);
        chunks.last_mut().unwrap().push(session);
    }
    chunks
}

fn row_count(session: &Session) -> usize {
    session.hours.len() + session.tools.len()
}

fn has_usage(session: &Session) -> bool {
    session
        .hours
        .values()
        .any(|h| h.new_context > 0 || h.output_tokens > 0 || h.subagent_requests > 0)
}

fn folder_name(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
}

fn after_overlap(active_at: &str, watermark: &str) -> bool {
    match (unix_seconds(active_at), unix_seconds(watermark)) {
        (Some(at), Some(mark)) => at + WATERMARK_OVERLAP.as_secs() >= mark,
        _ => true,
    }
}

fn estimate(project: &str) -> Overhead {
    overhead::estimate(&platform::claude_dir(), project)
}

fn run_with(
    mut post: impl FnMut(&str, &str, &str) -> Result<String, SyncStatus>,
    mut estimate: impl FnMut(&str) -> Overhead,
    config: &Config,
    sessions: &[Session],
    machine: &str,
) -> SyncStatus {
    if !config.is_enabled() {
        return SyncStatus::Off;
    }
    let url = format!(
        "{}{INGEST_PATH}",
        config.server_url.trim().trim_end_matches('/')
    );
    let mut rejected = Vec::new();
    let mut latest = None;
    let mut overheads = HashMap::new();
    for chunk in chunks(sessions, config::load_watermark(config).as_deref()) {
        for session in &chunk {
            overheads
                .entry(session.project.clone())
                .or_insert_with(|| estimate(&session.project));
        }
        let payload = Payload::new(machine, &chunk, config.hide_titles, &overheads);
        let body = match serde_json::to_string(&payload) {
            Ok(body) => body,
            Err(error) => return SyncStatus::Failed(error.to_string()),
        };
        match post(&url, config.api_key.trim(), &body) {
            Ok(response) => {
                let result = ingest_result(&response);
                rejected.extend(result.rejected);
                latest = result.latest_app_version.or(latest);
            }
            Err(status) => return status,
        }
        let newest = chunk.last().map_or("", |s| s.last_activity());
        if let Err(error) = config::save_watermark(config, newest) {
            return SyncStatus::Failed(error.to_string());
        }
    }
    SyncStatus::Synced {
        at: SystemTime::now(),
        rejected,
        update: latest.filter(|version| is_newer(version, APP_VERSION)),
    }
}

fn ingest_result(body: &str) -> IngestResult {
    serde_json::from_str::<Envelope>(body)
        .map(|envelope| envelope.data)
        .unwrap_or_default()
}

fn post(url: &str, key: &str, body: &str) -> Result<String, SyncStatus> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut request = agent.post(url);
    for (name, value) in headers(key) {
        request = request.header(name, value);
    }
    let mut response = request
        .send(body)
        .map_err(|error| SyncStatus::Failed(error.to_string()))?;
    let text = response.body_mut().read_to_string().unwrap_or_default();
    if response.status().is_success() {
        return Ok(text);
    }
    Err(failure(response.status().as_u16(), &text))
}

fn failure(status: u16, body: &str) -> SyncStatus {
    match status {
        401 | 403 => SyncStatus::KeyRejected,
        _ => SyncStatus::Failed(format!("HTTP {status}: {}", error_message(body))),
    }
}

fn headers(key: &str) -> [(&'static str, &str); 3] {
    [
        ("X-API-Key", key),
        ("X-App-Version", APP_VERSION),
        ("Content-Type", "application/json"),
    ]
}

fn error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| json["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overhead::Item;
    use crate::platform::{TempDir, lock_env};
    use crate::session::{HourUsage, RequestUsage, cached};
    use serde_json::Value;

    const MODEL: &str = "claude-opus-5-5";
    const REJECTED_RESPONSE: &str = r#"{"data":{"accepted":0,"rejected":[{"session_id":"s1","reason":"session belongs to another user"}]},"message":"Usage saved"}"#;

    fn session_with_hours(id: usize, hours: usize, growth: u64) -> Session {
        let mut session = Session::new(format!("s{id}"));
        session.latest_at = format!("2026-10-30T02:00:00.{id:03}Z");
        for hour in 0..hours {
            session.hours.insert(
                (format!("h{hour:05}"), MODEL.to_owned()),
                HourUsage {
                    new_context: growth,
                    requests: 1,
                    ..HourUsage::default()
                },
            );
        }
        session
    }

    fn sizes(chunks: &[Vec<&Session>]) -> Vec<usize> {
        chunks.iter().map(Vec::len).collect()
    }

    fn item(tokens: u64) -> Item {
        Item {
            name: "file.md".into(),
            tokens,
        }
    }

    fn overhead() -> Overhead {
        Overhead {
            memory_files: vec![item(1_000), item(200)],
            agents: vec![item(500)],
            skills: 300,
        }
    }

    fn with_request(mut session: Session, project: &str) -> Session {
        session.project = project.into();
        session.requests = 1;
        session.base_context = 20_000;
        session.latest_context = 25_000;
        session
    }

    fn enabled() -> Config {
        Config {
            server_url: "http://127.0.0.1:1/".into(),
            api_key: "key".into(),
            ..Config::default()
        }
    }

    #[test]
    fn tool_activity_is_sent_per_hour_without_any_content() {
        let mut session = Session::new("abc".into());
        let usage = session.tool_mut("2026-10-30T02:10:00Z", "Edit").unwrap();
        usage.calls = 3;
        usage.lines_added = 12;
        usage.lines_removed = 4;
        usage.commits = 1;
        let json = serde_json::to_value(SessionPayload::new(&session, false, None)).unwrap();
        assert_eq!(
            json["tools"],
            serde_json::json!([{
                "hour": "2026-10-30T02:00:00Z",
                "tool": "Edit",
                "calls": 3,
                "lines_added": 12,
                "lines_removed": 4,
                "commits": 1,
            }])
        );
    }

    #[test]
    fn payload_matches_server_contract() {
        let mut session = Session::new("abc".into());
        session.project = r"C:\Code\RAG".into();
        session.record_request(MODEL.into(), cached(20_000), "2026-10-30T02:10:00Z".into());
        session.record_request(MODEL.into(), cached(60_000), "2026-10-30T02:50:00Z".into());
        session.reset_baseline();
        let usage = RequestUsage {
            output_tokens: 250,
            ..cached(30_000)
        };
        session.record_request(MODEL.into(), usage, "2026-10-30T03:05:00Z".into());
        session.record_subagent_request(
            MODEL,
            RequestUsage {
                input_tokens: 4,
                cache_creation_input_tokens: 6_000,
                output_tokens: 900,
                ..RequestUsage::default()
            },
            "2026-10-30T03:20:00Z",
        );

        let json =
            serde_json::to_value(SessionPayload::new(&session, false, Some(&overhead()))).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "compactions",
                "context_window",
                "first_at",
                "hours",
                "latest_at",
                "latest_context",
                "model",
                "new_context",
                "overhead",
                "peak_context",
                "project",
                "requests",
                "session_id",
                "start_context",
                "title",
                "tools",
            ]
        );
        assert_eq!(json["session_id"], "abc");
        assert_eq!(json["title"], "RAG");
        assert_eq!(json["project"], "RAG");
        assert_eq!(json["start_context"], 20_000);
        assert_eq!(json["new_context"], 40_000);
        assert_eq!(json["compactions"], 1);
        assert_eq!(json["requests"], 3);
        assert_eq!(json["first_at"], "2026-10-30T02:10:00Z");
        assert_eq!(json["hours"][0]["hour"], "2026-10-30T02:00:00Z");
        assert_eq!(json["hours"][0]["model"], MODEL);
        assert_eq!(json["hours"][0]["new_context"], 40_000);
        assert_eq!(json["hours"][0]["requests"], 2);
        assert_eq!(json["hours"][1]["hour"], "2026-10-30T03:00:00Z");
        assert_eq!(json["hours"][1]["output_tokens"], 250);
        assert_eq!(json["hours"][1]["subagent_requests"], 1);
        assert_eq!(json["hours"][1]["subagent_output_tokens"], 900);
        assert_eq!(json["hours"][0]["cache_read_tokens"], 80_000);
        assert_eq!(json["hours"][1]["input_tokens"], 4);
        assert_eq!(json["hours"][1]["cache_write_tokens"], 6_000);
        assert_eq!(json["hours"][1]["cache_read_tokens"], 30_000);
        assert_eq!(
            json["overhead"],
            serde_json::json!({
                "system_tokens": 18_000,
                "memory_tokens": 1_200,
                "agents_tokens": 500,
                "skills_tokens": 300,
            })
        );
        let mut hour_keys: Vec<&str> = json["hours"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        hour_keys.sort_unstable();
        assert_eq!(
            hour_keys,
            [
                "cache_read_tokens",
                "cache_write_tokens",
                "hour",
                "input_tokens",
                "model",
                "new_context",
                "output_tokens",
                "requests",
                "subagent_output_tokens",
                "subagent_requests",
            ]
        );

        let payload = Payload::new("PC", &[&session], false, &HashMap::new());
        let json: Value = serde_json::to_value(payload).unwrap();
        assert_eq!(json["machine"], "PC");
        assert_eq!(json["app_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(json["sessions"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn hidden_titles_send_no_title_or_project() {
        let mut session = with_request(session_with_hours(0, 1, 1), r"C:\Code\client-x");
        session.title = "Client X pricing".into();
        let overheads = HashMap::from([(session.project.clone(), overhead())]);
        let json: Value =
            serde_json::to_value(Payload::new("PC", &[&session], true, &overheads)).unwrap();
        let sent = &json["sessions"][0];
        assert_eq!(sent["title"], "");
        assert_eq!(sent["project"], "");
        assert_eq!(sent["model"], session.model.as_str());
        assert_eq!(sent["overhead"]["system_tokens"], 18_000);
        assert_eq!(sent["overhead"]["memory_tokens"], 1_200);
    }

    #[test]
    fn overhead_is_estimated_once_per_project_and_omitted_without_requests() {
        let _env = lock_env();
        let dir = TempDir::new("sync-overhead");
        unsafe { std::env::set_var("CLAUDE_USAGE_CONFIG_DIR", dir.path()) };
        let mut sessions: Vec<Session> = (0..250)
            .map(|i| with_request(session_with_hours(i, 1, 1), ["a", "b"][i % 2]))
            .collect();
        sessions[0].requests = 0;
        let mut estimated = Vec::new();
        let mut bodies = Vec::new();
        let status = run_with(
            |_, _, body| {
                bodies.push(serde_json::from_str::<Value>(body).unwrap());
                Ok(String::new())
            },
            |project| {
                estimated.push(project.to_owned());
                overhead()
            },
            &enabled(),
            &sessions,
            "PC",
        );
        assert!(matches!(status, SyncStatus::Synced { .. }));
        assert_eq!(bodies.len(), 2);
        assert_eq!(estimated, ["a", "b"]);
        let sent: Vec<&Value> = bodies
            .iter()
            .flat_map(|body| body["sessions"].as_array().unwrap())
            .collect();
        assert_eq!(sent[0]["session_id"], "s0");
        assert!(sent[0].get("overhead").is_none());
        assert!(
            sent[1..]
                .iter()
                .all(|s| s["overhead"]["system_tokens"] == 18_000)
        );
    }

    #[test]
    fn unauthorized_responses_mean_the_key_was_rejected() {
        assert_eq!(failure(401, ""), SyncStatus::KeyRejected);
        assert_eq!(
            failure(403, r#"{"message":"forbidden"}"#),
            SyncStatus::KeyRejected
        );
        assert_eq!(
            failure(500, r#"{"message":"Internal server error"}"#),
            SyncStatus::Failed("HTTP 500: Internal server error".into())
        );
    }

    #[test]
    fn chunks_respect_session_and_hour_limits() {
        let many: Vec<Session> = (0..450)
            .rev()
            .map(|i| session_with_hours(i, 10, 1))
            .collect();
        let cut = chunks(&many, None);
        assert_eq!(sizes(&cut), [200, 200, 50]);
        assert_eq!(cut[0][0].id, "s0");
        assert_eq!(cut[2][49].id, "s449");

        let wide: Vec<Session> = (0..3).map(|i| session_with_hours(i, 15_000, 1)).collect();
        assert_eq!(sizes(&chunks(&wide, None)), [1, 1, 1]);

        let huge = [
            session_with_hours(0, 25_000, 1),
            session_with_hours(1, 10, 1),
        ];
        assert_eq!(sizes(&chunks(&huge, None)), [1, 1]);

        let mixed = [
            session_with_hours(1, 10, 1),
            session_with_hours(2, 10, 0),
            session_with_hours(3, 10, 1),
            session_with_hours(4, 10, 1),
        ];
        let cut = chunks(&mixed, None);
        let ids: Vec<&str> = cut.concat().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["s1", "s3", "s4"]);
    }

    #[test]
    fn chunks_send_sessions_with_output_or_subagent_usage() {
        let mut output_only = session_with_hours(1, 1, 0);
        output_only
            .hours
            .values_mut()
            .for_each(|h| h.output_tokens = 5);
        let mut subagent_only = session_with_hours(2, 1, 0);
        subagent_only.record_subagent_request(
            MODEL,
            RequestUsage::default(),
            "2026-10-30T02:00:00Z",
        );
        let idle = session_with_hours(3, 1, 0);

        let sessions = [output_only, subagent_only, idle];
        let cut = chunks(&sessions, None);
        let ids: Vec<&str> = cut.concat().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["s1", "s2"]);
    }

    #[test]
    fn post_sends_key_and_app_version_headers() {
        assert_eq!(
            headers("key"),
            [
                ("X-API-Key", "key"),
                ("X-App-Version", env!("CARGO_PKG_VERSION")),
                ("Content-Type", "application/json"),
            ]
        );
    }

    #[test]
    fn project_is_sent_as_folder_name() {
        assert_eq!(
            folder_name(r"C:\Code\Claude Usage Monitor"),
            "Claude Usage Monitor"
        );
        assert_eq!(folder_name("/home/dimas/my-site/"), "my-site");
        assert_eq!(folder_name("RAG"), "RAG");
        assert_eq!(folder_name(""), "");
    }

    #[test]
    fn chunks_resend_sessions_inside_watermark_overlap() {
        let at = |id: usize, latest_at: &str| {
            let mut session = session_with_hours(id, 1, 1);
            session.latest_at = latest_at.to_owned();
            session
        };
        let sessions = [
            at(1, "2026-10-30T01:59:59Z"),
            at(2, "2026-10-30T02:30:00Z"),
            at(3, "2026-10-30T03:10:00Z"),
        ];
        let cut = chunks(&sessions, Some("2026-10-30T03:00:00Z"));
        let ids: Vec<&str> = cut.concat().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["s2", "s3"]);
    }

    #[test]
    fn chunks_follow_subagent_activity() {
        let mut busy = session_with_hours(1, 1, 1);
        busy.latest_at = "2026-10-30T01:00:00Z".into();
        busy.record_subagent_request(MODEL, RequestUsage::default(), "2026-10-30T03:30:00Z");
        let mut quiet = session_with_hours(2, 1, 1);
        quiet.latest_at = "2026-10-30T03:20:00Z".into();
        let sessions = [busy, quiet];
        let cut = chunks(&sessions, Some("2026-10-30T03:00:00Z"));
        let ids: Vec<&str> = cut.concat().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["s2", "s1"]);
    }

    #[test]
    fn watermark_advances_per_chunk_and_past_rejections() {
        let _env = lock_env();
        let dir = TempDir::new("sync");
        unsafe { std::env::set_var("CLAUDE_USAGE_CONFIG_DIR", dir.path()) };
        config::save_watermark(&enabled(), "2026-10-30T01:00:00Z").unwrap();
        let sessions: Vec<Session> = (0..250).map(|i| session_with_hours(i, 1, 5)).collect();

        let mut calls = 0;
        let status = run_with(
            |url, key, body| {
                calls += 1;
                assert_eq!(url, "http://127.0.0.1:1/usage/v1/sessions");
                assert_eq!(key, "key");
                assert!(body.starts_with(r#"{"machine":"PC""#));
                if calls == 2 {
                    return Err(failure(500, "Internal server error"));
                }
                Ok(String::new())
            },
            |_| Overhead::default(),
            &enabled(),
            &sessions,
            "PC",
        );
        assert_eq!(calls, 2);
        assert_eq!(
            status,
            SyncStatus::Failed("HTTP 500: Internal server error".into())
        );
        assert_eq!(
            config::load_watermark(&enabled()).as_deref(),
            Some("2026-10-30T02:00:00.199Z")
        );

        let status = run_with(
            |_, _, _| Ok(REJECTED_RESPONSE.to_owned()),
            |_| Overhead::default(),
            &enabled(),
            &sessions,
            "PC",
        );
        let SyncStatus::Synced { rejected, .. } = status else {
            panic!("expected Synced, got {status:?}");
        };
        assert_eq!(rejected.len(), 2);
        assert_eq!(
            config::load_watermark(&enabled()).as_deref(),
            Some("2026-10-30T02:00:00.249Z")
        );
    }

    #[test]
    fn rejections_parse_from_response_envelope() {
        let rejections = |body| ingest_result(body).rejected;
        assert_eq!(
            rejections(REJECTED_RESPONSE),
            [Rejection {
                session_id: "s1".into(),
                reason: "session belongs to another user".into(),
            }]
        );
        assert_eq!(
            rejections(r#"{"data":{"accepted":3,"rejected":[]},"message":"Usage saved"}"#),
            []
        );
        assert_eq!(rejections(r#"{"data":{"accepted":3},"message":"ok"}"#), []);
        assert_eq!(rejections("not json"), []);
    }

    #[test]
    fn a_new_plan_reading_syncs_after_five_minutes_instead_of_thirty() {
        let minutes = |m: u64| Duration::from_secs(m * 60);
        assert!(!is_due(None, Duration::from_secs(30), false));
        assert!(is_due(None, minutes(1), false));
        assert!(!is_due(Some(minutes(4)), minutes(60), true));
        assert!(is_due(Some(minutes(5)), minutes(60), true));
        assert!(!is_due(Some(minutes(29)), minutes(60), false));
        assert!(is_due(Some(minutes(30)), minutes(60), false));
    }

    #[test]
    fn newer_team_version_is_offered_as_an_update() {
        let _env = lock_env();
        let dir = TempDir::new("sync-update");
        unsafe { std::env::set_var("CLAUDE_USAGE_CONFIG_DIR", dir.path()) };
        let sessions = [session_with_hours(0, 1, 1)];
        let update = |body: String| {
            let status = run_with(
                move |_, _, _| Ok(body.clone()),
                |_| Overhead::default(),
                &enabled(),
                &sessions,
                "PC",
            );
            match status {
                SyncStatus::Synced { update, .. } => update,
                other => panic!("expected Synced, got {other:?}"),
            }
        };
        let reply = |version: &str| {
            format!(r#"{{"data":{{"accepted":1,"rejected":[],"latest_app_version":"{version}"}}}}"#)
        };
        assert_eq!(update(reply("999.0.0")), Some("999.0.0".to_owned()));
        assert_eq!(update(reply(APP_VERSION)), None);
        assert_eq!(update(String::new()), None);
        assert!(is_newer("0.10.0", "0.9.1"));
        assert!(!is_newer("0.2.0", "0.2.0-dev"));
    }

    #[test]
    fn sync_disabled_without_settings() {
        let sessions = [session_with_hours(0, 1, 1)];
        let config = Config {
            api_key: "  ".into(),
            ..enabled()
        };
        let mut calls = 0;
        let status = run_with(
            |_, _, _| {
                calls += 1;
                Ok(String::new())
            },
            |_| Overhead::default(),
            &config,
            &sessions,
            "PC",
        );
        assert_eq!(status, SyncStatus::Off);
        assert_eq!(calls, 0);
        assert!(!Config::default().is_enabled());
    }
}
