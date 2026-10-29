#[derive(Debug, Default, Clone)]
pub struct Session {
    pub id: String,
    pub project: String,
    pub title: String,
    pub model: String,
    pub latest_at: String,
    pub start_context: u64,
    pub latest_context: u64,
    pub new_context: u64,
    pub compactions: u32,
    pub context_window: u64,
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
        !self.latest_at.is_empty()
    }

    pub fn record_request(&mut self, model: String, context: u64, at: String) {
        if !self.has_requests() {
            self.start_context = context;
        } else if context >= self.latest_context {
            self.new_context += context - self.latest_context;
        } else {
            self.compactions += 1;
        }
        self.latest_context = context;
        self.latest_at = at;
        self.context_window = context_window(&model);
        self.model = model;
    }

    pub fn reset_baseline(&mut self) {
        self.start_context = self.latest_context;
        self.new_context = 0;
        self.compactions = 0;
    }

    pub fn usage_fraction(&self) -> f32 {
        self.latest_context as f32 / self.context_window.max(1) as f32
    }

    pub fn latest_label(&self) -> String {
        format!(
            "{} / {} ({:.0}%)",
            format_tokens(self.latest_context),
            format_tokens(self.context_window),
            self.usage_fraction() * 100.0
        )
    }

    pub fn new_context_label(&self) -> String {
        format!("+{}", format_tokens(self.new_context))
    }
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
mod tests {
    use super::*;

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
        let mut request = |context: u64, at: &str| {
            session.record_request("claude-opus-5-5".into(), context, at.into())
        };
        request(4_000, "09:00:00");
        request(50_000, "09:00:01");
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
            session.record_request("claude-opus-5-5".into(), context, format!("10:{i:02}"));
            assert_eq!(session.new_context_label(), expected, "step {i}");
            assert_eq!(session.compactions, compactions, "step {i}");
        }
        assert_eq!(session.start_context, 50_000);
        assert_eq!(session.latest_label(), "70.0K / 1M (7%)");
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
