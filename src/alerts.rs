use crate::plan_limits::Window;
use crate::platform::APP_NAME;
use crate::session::{Session, format_tokens, unix_seconds};
use crate::theme::{FULL_AT, WARN_AT};
use std::collections::HashSet;

const DAY_SECONDS: i64 = 86_400;
const LIMIT_STEPS: [f64; 2] = [80.0, 90.0];
const HOUR_SECONDS: u64 = 3_600;

pub fn day_start(now: u64, utc_offset: i64) -> u64 {
    let local = i64::try_from(now).unwrap_or_default() + utc_offset;
    u64::try_from(local - local.rem_euclid(DAY_SECONDS) - utc_offset).unwrap_or_default()
}

pub fn new_context_since<'a>(sessions: impl IntoIterator<Item = &'a Session>, since: u64) -> u64 {
    sessions
        .into_iter()
        .flat_map(|s| &s.hours)
        .filter(|((hour, _), _)| unix_seconds(hour).is_some_and(|at| at >= since))
        .map(|(_, usage)| usage.new_context)
        .sum()
}

pub fn tray_text(today: u64, active: usize, sync: &str) -> String {
    format!(
        "{APP_NAME}\n+{} new context today\n{active} active · {sync}",
        format_tokens(today)
    )
}

#[derive(Default)]
pub struct FullWarnings {
    warned: HashSet<String>,
}

impl FullWarnings {
    pub fn newly_full<'a>(
        &mut self,
        active: impl IntoIterator<Item = &'a Session>,
    ) -> Vec<&'a Session> {
        let mut fresh = Vec::new();
        for session in active {
            let fraction = session.usage_fraction();
            if fraction < WARN_AT {
                self.warned.remove(&session.id);
            } else if fraction >= FULL_AT && self.warned.insert(session.id.clone()) {
                fresh.push(session);
            }
        }
        fresh
    }
}

pub fn notify_full(session: &Session) {
    let body = format!(
        "{} is at {:.0}% of its context window. Auto-compact is near.",
        session.name(),
        session.usage_fraction() * 100.0
    );
    notify("Context almost full", body);
}

#[derive(Default)]
pub struct LimitWarnings {
    seen: HashSet<(String, u64, u8)>,
    primed: bool,
}

impl LimitWarnings {
    pub fn newly_crossed(&mut self, windows: &[(String, Window)]) -> Vec<(String, Window)> {
        let mut fresh = Vec::new();
        for (label, window) in windows {
            let mut crossed = false;
            for step in LIMIT_STEPS
                .iter()
                .filter(|step| window.used_percentage >= **step)
            {
                let key = (label.clone(), window.resets_at / HOUR_SECONDS, *step as u8);
                crossed |= self.seen.insert(key);
            }
            if crossed && self.primed {
                fresh.push((label.clone(), *window));
            }
        }
        self.primed = true;
        fresh
    }
}

pub fn notify_limit(name: &str, window: Window, resets_in: &str) {
    notify(
        "Claude limit almost used",
        format!(
            "{name} is {:.0}% used. It resets in {resets_in}.",
            window.used_percentage
        ),
    );
}

pub fn notify_key_reminder() {
    notify(
        "Usage sync is off",
        "Add your API key to send usage to the team dashboard".to_owned(),
    );
}

fn notify(summary: &'static str, body: String) {
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new()
            .appname(APP_NAME)
            .summary(summary)
            .body(&body)
            .show();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::HourUsage;

    fn session(id: &str, latest_context: u64) -> Session {
        Session {
            id: id.into(),
            latest_context,
            context_window: 1_000,
            ..Session::default()
        }
    }

    #[test]
    fn limit_warnings_fire_once_per_step_and_skip_what_was_already_crossed() {
        let window = |used| Window {
            used_percentage: used,
            resets_at: 1_000_000,
        };
        let mut warnings = LimitWarnings::default();
        assert!(
            warnings
                .newly_crossed(&[("7d".into(), window(85.0))])
                .is_empty()
        );
        assert!(
            warnings
                .newly_crossed(&[("7d".into(), window(86.0))])
                .is_empty()
        );
        let ninety =
            warnings.newly_crossed(&[("7d".into(), window(91.0)), ("Fable".into(), window(10.0))]);
        assert_eq!(ninety.len(), 1);
        assert_eq!(ninety[0].0, "7d");
        assert!(
            warnings
                .newly_crossed(&[("7d".into(), window(95.0))])
                .is_empty()
        );
        let fable = warnings.newly_crossed(&[("Fable".into(), window(80.0))]);
        assert_eq!(fable.len(), 1);
        let next_week = Window {
            used_percentage: 82.0,
            resets_at: 1_000_000 + 7 * 86_400,
        };
        assert_eq!(warnings.newly_crossed(&[("7d".into(), next_week)]).len(), 1);
    }

    #[test]
    fn day_start_uses_local_midnight() {
        let noon_utc = 1_793_620_800;
        assert_eq!(day_start(noon_utc, 0), 1_793_577_600);
        assert_eq!(day_start(noon_utc, 7 * 3_600), 1_793_552_400);
        assert_eq!(day_start(noon_utc, -5 * 3_600), 1_793_595_600);
    }

    #[test]
    fn new_context_since_counts_hours_from_start() {
        let mut s = session("a", 0);
        for (hour, new_context) in [
            ("2026-11-01T23:00:00Z", 5),
            ("2026-11-02T00:00:00Z", 7),
            ("2026-11-02T09:00:00Z", 11),
        ] {
            s.hours.insert(
                (hour.into(), "m".into()),
                HourUsage {
                    new_context,
                    ..HourUsage::default()
                },
            );
        }
        let midnight = unix_seconds("2026-11-02T00:00:00Z").unwrap();
        assert_eq!(new_context_since([&s], midnight), 18);
    }

    #[test]
    fn full_warning_fires_once_until_context_drops() {
        let mut warnings = FullWarnings::default();
        let full = session("a", 950);
        assert_eq!(warnings.newly_full([&full]).len(), 1);
        assert!(warnings.newly_full([&full]).is_empty());
        assert!(warnings.newly_full([&session("a", 850)]).is_empty());
        assert!(warnings.newly_full([&full]).is_empty());
        assert!(warnings.newly_full([&session("a", 300)]).is_empty());
        assert_eq!(warnings.newly_full([&full]).len(), 1);
    }

    #[test]
    fn full_warning_survives_idle_pauses() {
        let mut warnings = FullWarnings::default();
        let full = session("a", 950);
        assert_eq!(warnings.newly_full([&full]).len(), 1);
        assert!(warnings.newly_full([]).is_empty());
        assert!(warnings.newly_full([&full]).is_empty());
    }

    #[test]
    fn tray_text_summarizes_today() {
        assert_eq!(
            tray_text(1_240_000, 1, "Synced 10:30"),
            format!("{APP_NAME}\n+1.2M new context today\n1 active · Synced 10:30")
        );
    }
}
