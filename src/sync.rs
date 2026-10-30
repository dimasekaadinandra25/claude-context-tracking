use crate::config::{self, Config};
use crate::index::Index;
use crate::session::{Session, unix_seconds};
use serde::Serialize;
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
}

impl<'a> Payload<'a> {
    fn new(machine: &'a str, sessions: &[&'a Session]) -> Self {
        Self {
            machine,
            app_version: env!("CARGO_PKG_VERSION"),
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
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SyncStatus {
    Off,
    Synced(SystemTime),
    Failed(String),
}

pub struct Syncer {
    started: Instant,
    last_run: Option<Instant>,
    last_config: Config,
}

impl Syncer {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            last_run: None,
            last_config: config::load(),
        }
    }

    pub fn tick(&mut self, index: &Index, machine: &str) -> Option<SyncStatus> {
        let config = config::load();
        let due = match self.last_run {
            Some(at) => at.elapsed() >= SYNC_INTERVAL,
            None => self.started.elapsed() >= STARTUP_DELAY,
        };
        if !due && config == self.last_config {
            return None;
        }
        self.last_run = Some(Instant::now());
        self.last_config = config;
        Some(run_with(
            post,
            &self.last_config,
            &index.snapshot(),
            machine,
        ))
    }
}

pub fn chunks<'a>(sessions: &'a [Session], watermark: Option<&str>) -> Vec<Vec<&'a Session>> {
    let mut pending: Vec<&Session> = sessions
        .iter()
        .filter(|s| watermark.is_none_or(|w| after_overlap(&s.latest_at, w)))
        .filter(|s| s.lifetime_new_context() > 0)
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
    mut post: impl FnMut(&str, &str, &str) -> Result<(), String>,
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
    for chunk in chunks(sessions, config::load_watermark().as_deref()) {
        let body = match serde_json::to_string(&Payload::new(machine, &chunk)) {
            Ok(body) => body,
            Err(error) => return SyncStatus::Failed(error.to_string()),
        };
        if let Err(reason) = post(&url, config.api_key.trim(), &body) {
            return SyncStatus::Failed(reason);
        }
        newest = chunk.last().map(|s| s.latest_at.as_str());
    }
    match newest.map(config::save_watermark) {
        Some(Err(error)) => SyncStatus::Failed(error.to_string()),
        _ => SyncStatus::Synced(SystemTime::now()),
    }
}

fn post(url: &str, key: &str, body: &str) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .post(url)
        .header("X-API-Key", key)
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        return Ok(());
    }
    let text = response.body_mut().read_to_string().unwrap_or_default();
    Err(format!(
        "HTTP {}: {}",
        response.status().as_u16(),
        error_message(&text)
    ))
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
    use crate::session::{HourUsage, cached};
    use serde_json::Value;

    const MODEL: &str = "claude-opus-5-5";

    fn session_with_hours(id: usize, hours: usize, growth: u64) -> Session {
        let mut session = Session::new(format!("s{id}"));
        session.latest_at = format!("2026-10-30T02:00:00.{id:03}Z");
        for hour in 0..hours {
            session.hours.insert(
                (format!("h{hour:05}"), MODEL.to_owned()),
                HourUsage {
                    new_context: growth,
                    requests: 1,
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
        }
    }

    #[test]
    fn payload_matches_server_contract() {
        let mut session = Session::new("abc".into());
        session.project = r"C:\Code\RAG".into();
        session.record_request(MODEL.into(), cached(20_000), "2026-10-30T02:10:00Z".into());
        session.record_request(MODEL.into(), cached(60_000), "2026-10-30T02:50:00Z".into());
        session.reset_baseline();
        session.record_request(MODEL.into(), cached(30_000), "2026-10-30T03:05:00Z".into());

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
    fn failed_send_keeps_watermark() {
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
                Ok(())
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

        let status = run_with(|_, _, _| Ok(()), &enabled(), &sessions, "PC");
        assert!(matches!(status, SyncStatus::Synced(_)));
        assert_eq!(
            config::load_watermark().as_deref(),
            Some("2026-10-30T02:00:00.249Z")
        );
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
                Ok(())
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
