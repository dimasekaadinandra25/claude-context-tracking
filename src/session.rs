use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ACTIVE_WINDOW: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RequestUsage {
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
}

impl RequestUsage {
    pub fn context(&self) -> u64 {
        self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens
    }
}

#[derive(Debug, Default, Clone)]
pub struct Session {
    pub id: String,
    pub project: String,
    pub title: String,
    pub model: String,
    pub first_at: String,
    pub latest_at: String,
    pub base_context: u64,
    pub start_context: u64,
    pub latest_context: u64,
    pub new_context: u64,
    pub new_context_by_model: BTreeMap<String, u64>,
    pub peak_context: u64,
    pub compactions: u32,
    pub requests: u32,
    pub last_usage: RequestUsage,
    pub context_window: u64,
    pub estimated: bool,
}

impl Session {
    pub fn new(id: String) -> Self {
        Self {
            id,
            ..Self::default()
        }
    }

    pub fn name(&self) -> &str {
        if !self.title.is_empty() {
            return &self.title;
        }
        match self.project.rsplit(['\\', '/']).next() {
            Some(folder) if !folder.is_empty() => folder,
            _ => &self.id,
        }
    }

    pub fn has_requests(&self) -> bool {
        self.requests > 0
    }

    pub fn record_request(&mut self, model: String, usage: RequestUsage, at: String) {
        let context = usage.context();
        if !self.has_requests() {
            self.start_context = context;
            self.base_context = context;
            self.first_at.clone_from(&at);
        } else if self.estimated || model != self.model {
            self.estimated = false;
        } else if context > self.latest_context {
            let growth = context - self.latest_context;
            self.new_context += growth;
            *self.new_context_by_model.entry(model.clone()).or_default() += growth;
        } else if context < self.latest_context {
            self.compactions += 1;
        }
        self.latest_context = context;
        self.peak_context = self.peak_context.max(context);
        self.requests += 1;
        self.last_usage = usage;
        self.latest_at = at;
        self.context_window = context_window(&model);
        self.model = model;
    }

    pub fn record_compaction(&mut self, summary_tokens: u64, at: String) {
        if !self.has_requests() {
            return;
        }
        self.latest_context = self.system_context() + summary_tokens;
        self.compactions += 1;
        self.estimated = true;
        self.latest_at = at;
    }

    pub fn record_model_switch(&mut self, model: String) {
        if !self.has_requests() || model == self.model {
            return;
        }
        self.context_window = context_window(&model);
        self.model = model;
        self.estimated = true;
    }

    pub fn reset_baseline(&mut self) {
        self.start_context = self.latest_context;
        self.new_context = 0;
        self.new_context_by_model.clear();
        self.compactions = 0;
    }

    pub fn system_context(&self) -> u64 {
        self.base_context.min(self.latest_context)
    }

    pub fn conversation_context(&self) -> u64 {
        self.latest_context - self.system_context()
    }

    pub fn free_space(&self) -> u64 {
        self.context_window.saturating_sub(self.latest_context)
    }

    pub fn usage_fraction(&self) -> f32 {
        share(self.latest_context, self.context_window)
    }

    pub fn latest_tokens_label(&self) -> String {
        let prefix = if self.estimated { "≈ " } else { "" };
        format!("{prefix}{}", format_tokens(self.latest_context))
    }

    pub fn latest_label(&self) -> String {
        format!(
            "{} / {} ({:.0}%)",
            self.latest_tokens_label(),
            format_tokens(self.context_window),
            self.usage_fraction() * 100.0
        )
    }

    pub fn new_context_label(&self) -> String {
        format!("+{}", format_tokens(self.new_context))
    }

    pub fn idle_for(&self, now: SystemTime) -> Option<Duration> {
        let latest = UNIX_EPOCH + Duration::from_secs(unix_seconds(&self.latest_at)?);
        Some(now.duration_since(latest).unwrap_or_default())
    }

    pub fn is_active(&self, now: SystemTime) -> bool {
        self.new_context > 0 && self.idle_for(now).is_some_and(|idle| idle < ACTIVE_WINDOW)
    }
}

fn unix_seconds(at: &str) -> Option<u64> {
    let field = |start: usize, len: usize| at.get(start..start + len)?.parse::<i64>().ok();
    let (year, month, day) = (field(0, 4)?, field(5, 2)?, field(8, 2)?);
    let seconds = field(11, 2)? * 3_600 + field(14, 2)? * 60 + field(17, 2)?;
    let year = if month <= 2 { year - 1 } else { year };
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let days = year * 365 + year / 4 - year / 100 + year / 400 + day_of_year - 719_468;
    u64::try_from(days * 86_400 + seconds).ok()
}

pub fn format_age(age: Duration) -> String {
    match age.as_secs() {
        seconds @ ..3_600 => format!("{}m", seconds / 60),
        seconds @ ..86_400 => format!("{}h", seconds / 3_600),
        seconds => format!("{}d", seconds / 86_400),
    }
}

pub fn total_new_context<'a>(sessions: impl IntoIterator<Item = &'a Session>) -> u64 {
    sessions.into_iter().map(|s| s.new_context).sum()
}

pub fn share(part: u64, whole: u64) -> f32 {
    part as f32 / whole.max(1) as f32
}

pub fn context_window(model: &str) -> u64 {
    let version: Vec<u32> = model
        .split('-')
        .filter(|part| part.len() <= 2)
        .filter_map(|part| part.parse().ok())
        .collect();
    match version[..] {
        [major, ..] if major >= 5 => 1_000_000,
        [4, minor, ..] if minor >= 6 => 1_000_000,
        _ => 200_000,
    }
}

pub fn format_tokens(tokens: u64) -> String {
    match tokens {
        1_000_000.. => format!("{:.1}M", tokens as f64 / 1e6).replace(".0M", "M"),
        1_000.. => format!("{:.1}K", tokens as f64 / 1e3),
        _ => tokens.to_string(),
    }
}

#[cfg(test)]
pub fn cached(context: u64) -> RequestUsage {
    RequestUsage {
        cache_read_input_tokens: context,
        ..RequestUsage::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = "claude-opus-5-5";
    const SONNET: &str = "claude-sonnet-5-5";

    #[test]
    fn context_window_by_model() {
        for (model, window) in [
            ("claude-opus-5-5", 1_000_000),
            ("claude-opus-5", 1_000_000),
            ("claude-sonnet-4-6", 1_000_000),
            ("claude-haiku-4-5", 200_000),
            ("claude-opus-4-20250514", 200_000),
            ("claude-3-5-sonnet-20241022", 200_000),
        ] {
            assert_eq!(context_window(model), window, "{model}");
        }
    }

    #[test]
    fn format_tokens_scales() {
        assert_eq!(format_tokens(950), "950");
        assert_eq!(format_tokens(6_800), "6.8K");
        assert_eq!(format_tokens(1_000_000), "1M");
    }

    #[test]
    fn new_context_keeps_counting_through_compaction() {
        let mut session = Session::new("s1".into());
        session.record_request(MODEL.into(), cached(4_000), "09:00:00".into());
        session.record_request(MODEL.into(), cached(50_000), "09:00:01".into());
        session.reset_baseline();

        let steps = [
            (120_000, "+70.0K", 0),
            (210_000, "+160.0K", 0),
            (35_000, "+160.0K", 1),
            (90_000, "+215.0K", 1),
            (30_000, "+215.0K", 2),
            (70_000, "+255.0K", 2),
        ];
        for (i, (context, expected, compactions)) in steps.into_iter().enumerate() {
            session.record_request(MODEL.into(), cached(context), format!("10:{i:02}"));
            assert_eq!(session.new_context_label(), expected, "step {i}");
            assert_eq!(session.compactions, compactions, "step {i}");
        }
        assert_eq!(session.start_context, 50_000);
        assert_eq!(session.peak_context, 210_000);
        assert_eq!(session.requests, 8);
        assert_eq!(session.latest_label(), "70.0K / 1M (7%)");
    }

    #[test]
    fn compaction_boundary_estimates_until_next_request() {
        let mut session = Session::new("s1".into());
        session.record_request(MODEL.into(), cached(50_000), "09:00".into());
        session.record_request(MODEL.into(), cached(440_000), "09:10".into());
        session.reset_baseline();
        session.record_request(MODEL.into(), cached(450_000), "09:20".into());

        session.record_compaction(27_000, "09:30".into());
        assert_eq!(session.compactions, 1);
        assert_eq!(session.latest_label(), "≈ 77.0K / 1M (8%)");
        assert_eq!(session.new_context_label(), "+10.0K");

        session.record_request(MODEL.into(), cached(76_800), "09:31".into());
        assert_eq!(session.compactions, 1);
        assert_eq!(session.latest_label(), "76.8K / 1M (8%)");
        assert_eq!(session.new_context_label(), "+10.0K");

        session.record_request(MODEL.into(), cached(80_000), "09:40".into());
        assert_eq!(session.new_context_label(), "+13.2K");
    }

    #[test]
    fn model_switch_rebaselines_without_growth_or_compaction() {
        let mut session = Session::new("s1".into());
        session.record_request(MODEL.into(), cached(50_000), "09:00".into());
        session.record_request(MODEL.into(), cached(100_000), "09:10".into());

        session.record_model_switch("claude-haiku-4-5".into());
        assert_eq!(session.model, "claude-haiku-4-5");
        assert_eq!(session.latest_label(), "≈ 100.0K / 200.0K (50%)");

        session.record_request("claude-haiku-4-5".into(), cached(96_000), "09:20".into());
        assert_eq!(session.latest_label(), "96.0K / 200.0K (48%)");
        assert_eq!(session.compactions, 0);
        assert_eq!(session.new_context_label(), "+50.0K");

        session.record_request(MODEL.into(), cached(104_000), "09:30".into());
        assert_eq!(session.new_context_label(), "+50.0K");
        session.record_request(MODEL.into(), cached(110_000), "09:40".into());
        assert_eq!(session.new_context_label(), "+56.0K");

        session.record_request(SONNET.into(), cached(112_000), "09:50".into());
        session.record_request(SONNET.into(), cached(120_000), "10:00".into());
        assert_eq!(session.new_context_label(), "+64.0K");
        assert_eq!(
            session.new_context_by_model,
            BTreeMap::from([(MODEL.to_owned(), 56_000), (SONNET.to_owned(), 8_000)])
        );

        session.reset_baseline();
        assert!(session.new_context_by_model.is_empty());
    }

    #[test]
    fn breakdown_splits_system_conversation_and_free_space() {
        let mut session = Session::new("s1".into());
        session.record_request(MODEL.into(), cached(30_000), "09:00".into());
        let latest = RequestUsage {
            input_tokens: 2,
            cache_creation_input_tokens: 8_000,
            cache_read_input_tokens: 92_000,
            output_tokens: 500,
        };
        session.record_request(MODEL.into(), latest, "09:05".into());

        assert_eq!(session.first_at, "09:00");
        assert_eq!(session.last_usage, latest);
        assert_eq!(session.system_context(), 30_000);
        assert_eq!(session.conversation_context(), 70_002);
        assert_eq!(session.free_space(), 899_998);

        session.record_request(MODEL.into(), cached(20_000), "09:10".into());
        assert_eq!(session.system_context(), 20_000);
        assert_eq!(session.conversation_context(), 0);
    }

    #[test]
    fn total_new_context_sums_sessions() {
        let mut a = Session::new("a".into());
        let mut b = Session::new("b".into());
        for (session, contexts) in [(&mut a, [10_000, 100_000]), (&mut b, [5_000, 45_000])] {
            for (i, context) in contexts.into_iter().enumerate() {
                session.record_request(MODEL.into(), cached(context), format!("0{i}"));
            }
        }
        assert_eq!(total_new_context([&a, &b]), 130_000);
    }

    #[test]
    fn activity_follows_latest_request_time() {
        assert_eq!(unix_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_seconds("2000-03-01T00:00:00.000Z"), Some(951_868_800));
        assert_eq!(
            unix_seconds("2026-10-30T01:45:39.123Z"),
            Some(1_793_324_739)
        );
        assert_eq!(unix_seconds(""), None);

        let mut session = Session::new("s1".into());
        session.record_request(MODEL.into(), cached(1_000), "2026-10-30T01:40:00Z".into());
        session.record_request(MODEL.into(), cached(1_000), "2026-10-30T01:45:39Z".into());
        let at = |seconds| UNIX_EPOCH + Duration::from_secs(seconds);
        assert!(!session.is_active(at(1_793_324_739 + 60)));
        session.record_request(MODEL.into(), cached(2_000), "2026-10-30T01:45:39Z".into());
        assert!(session.is_active(at(1_793_324_739 + 9 * 60)));
        assert!(!session.is_active(at(1_793_324_739 + 10 * 60)));
        assert!(!Session::new("s2".into()).is_active(at(0)));

        assert_eq!(format_age(Duration::from_secs(14 * 60)), "14m");
        assert_eq!(format_age(Duration::from_secs(3 * 3_600)), "3h");
        assert_eq!(format_age(Duration::from_secs(50 * 3_600)), "2d");
    }

    #[test]
    fn name_falls_back_to_project_folder_then_id() {
        let mut session = Session::new("abc".into());
        assert_eq!(session.name(), "abc");
        session.project = r"C:\Code\RAG".into();
        assert_eq!(session.name(), "RAG");
        session.title = "Hello".into();
        assert_eq!(session.name(), "Hello");
    }
}
