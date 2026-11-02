use crate::config::{self, Config};
use crate::index::Index;
use crate::platform::APP_VERSION;
use crate::session::{Session, unix_seconds};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime};

const SYNC_INTERVAL: Duration = Duration::from_secs(30 * 60);
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
    fn new(machine: &'a str, sessions: &[&'a Session]) -> Self {
        Self {
            machine,
            app_version: APP_VERSION,
            sessions: sessions.iter().map(|s| SessionPayload::from(*s)).collect(),
        }
    }
}

impl<'a> From<&'a Session> for SessionPayload<'a> {
    fn from(s: &'a Session) -> Self {
        Self {
            session_id: &s.id,
            project: folder_name(&s.project),
            title: s.name(),
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

#[derive(Deserialize)]
struct IngestResult {
    #[serde(default)]
    rejected: Vec<Rejection>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SyncStatus {
    Off,
    Synced {
        at: SystemTime,
        rejected: Vec<Rejection>,
    },
    Failed(String),
}

pub struct Syncer {
    started: Instant,
    last_run: Option<Instant>,
    last_config: Config,
    requested: bool,
}

impl Syncer {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            last_run: None,
            last_config: config::load(),
            requested: false,
        }
    }

    pub fn request(&mut self) {
        self.requested = true;
    }

    pub fn tick(&mut self, index: &Index, machine: &str) -> Option<SyncStatus> {
        let due = match self.last_run {
            Some(at) => at.elapsed() >= SYNC_INTERVAL,
            None => self.started.elapsed() >= STARTUP_DELAY,
        };
        if !due && !self.requested && config::load() == self.last_config {
            return None;
        }
        Some(self.run(index, machine))
    }

    pub fn run(&mut self, index: &Index, machine: &str) -> SyncStatus {
        self.requested = false;
        self.last_run = Some(Instant::now());
        self.last_config = config::load();
        run_with(post, &self.last_config, &index.snapshot(), machine)
    }
}

pub fn chunks<'a>(sessions: &'a [Session], watermark: Option<&str>) -> Vec<Vec<&'a Session>> {
    let mut pending: Vec<&Session> = sessions
        .iter()
        .filter(|s| watermark.is_none_or(|w| after_overlap(&s.latest_at, w)))
        .filter(|s| has_usage(s))
        .collect();
    pending.sort_by(|a, b| a.latest_at.cmp(&b.latest_at));
    let mut chunks: Vec<Vec<&Session>> = Vec::new();
    let mut rows = 0;
    for session in pending {
        let full = chunks.last().is_none_or(|chunk| {
            chunk.len() >= MAX_SESSIONS || rows + session.hours.len() > MAX_HOUR_ROWS
        });
        if full {
            chunks.push(Vec::new());
            rows = 0;
        }
        rows += session.hours.len();
        chunks.last_mut().unwrap().push(session);
    }
    chunks
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

fn after_overlap(latest_at: &str, watermark: &str) -> bool {
    match (unix_seconds(latest_at), unix_seconds(watermark)) {
        (Some(at), Some(mark)) => at + WATERMARK_OVERLAP.as_secs() >= mark,
        _ => true,
    }
}

fn run_with(
    mut post: impl FnMut(&str, &str, &str) -> Result<String, String>,
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
    let mut newest = None;
    let mut rejected = Vec::new();
    for chunk in chunks(sessions, config::load_watermark().as_deref()) {
        let body = match serde_json::to_string(&Payload::new(machine, &chunk)) {
            Ok(body) => body,
            Err(error) => return SyncStatus::Failed(error.to_string()),
        };
        match post(&url, config.api_key.trim(), &body) {
            Ok(response) => rejected.extend(rejections(&response)),
            Err(reason) => return SyncStatus::Failed(reason),
        }
        newest = chunk.last().map(|s| s.latest_at.as_str());
    }
    match newest.map(config::save_watermark) {
        Some(Err(error)) => SyncStatus::Failed(error.to_string()),
        _ => SyncStatus::Synced {
            at: SystemTime::now(),
            rejected,
        },
    }
}

fn rejections(body: &str) -> Vec<Rejection> {
    serde_json::from_str::<Envelope>(body)
        .map(|envelope| envelope.data.rejected)
        .unwrap_or_default()
}

fn post(url: &str, key: &str, body: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut request = agent.post(url);
    for (name, value) in headers(key) {
        request = request.header(name, value);
    }
    let mut response = request.send(body).map_err(|error| error.to_string())?;
    let text = response.body_mut().read_to_string().unwrap_or_default();
    if response.status().is_success() {
        return Ok(text);
    }
    Err(format!(
        "HTTP {}: {}",
        response.status().as_u16(),
        error_message(&text)
    ))
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
    use crate::platform::TempDir;
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

    fn enabled() -> Config {
        Config {
            server_url: "http://127.0.0.1:1/".into(),
            api_key: "key".into(),
            ..Config::default()
        }
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

        let json = serde_json::to_value(SessionPayload::from(&session)).unwrap();
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
                "peak_context",
                "project",
                "requests",
                "session_id",
                "start_context",
                "title",
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

        let payload = Payload::new("PC", &[&session]);
        let json: Value = serde_json::to_value(payload).unwrap();
        assert_eq!(json["machine"], "PC");
        assert_eq!(json["app_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(json["sessions"].as_array().unwrap().len(), 1);
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
                ("X-App-Version", "0.2.0"),
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
    fn watermark_holds_on_failure_and_advances_past_rejections() {
        let dir = TempDir::new("sync");
        unsafe { std::env::set_var("CLAUDE_USAGE_CONFIG_DIR", dir.path()) };
        let start = "2026-10-30T01:00:00Z";
        config::save_watermark(start).unwrap();
        let sessions: Vec<Session> = (0..250).map(|i| session_with_hours(i, 1, 5)).collect();

        let mut calls = 0;
        let status = run_with(
            |url, key, body| {
                calls += 1;
                assert_eq!(url, "http://127.0.0.1:1/usage/v1/sessions");
                assert_eq!(key, "key");
                assert!(body.starts_with(r#"{"machine":"PC""#));
                if calls == 2 {
                    return Err("HTTP 500: Internal server error".into());
                }
                Ok(String::new())
            },
            &enabled(),
            &sessions,
            "PC",
        );
        assert_eq!(calls, 2);
        assert_eq!(
            status,
            SyncStatus::Failed("HTTP 500: Internal server error".into())
        );
        assert_eq!(config::load_watermark().as_deref(), Some(start));

        let status = run_with(
            |_, _, _| Ok(REJECTED_RESPONSE.to_owned()),
            &enabled(),
            &sessions,
            "PC",
        );
        let SyncStatus::Synced { rejected, .. } = status else {
            panic!("expected Synced, got {status:?}");
        };
        assert_eq!(rejected.len(), 2);
        assert_eq!(
            config::load_watermark().as_deref(),
            Some("2026-10-30T02:00:00.249Z")
        );
    }

    #[test]
    fn rejections_parse_from_response_envelope() {
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
            &config,
            &sessions,
            "PC",
        );
        assert_eq!(status, SyncStatus::Off);
        assert_eq!(calls, 0);
        assert!(!Config::default().is_enabled());
    }
}
