use crate::config;
use crate::export::write_atomic;
use crate::plan_limits::Window;
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "day_budget.json";
const DAY_SECONDS: f64 = 86_400.0;
const SAME_WINDOW_SECONDS: u64 = 3_600;
const FULL_PERCENT: f64 = 100.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DayBudget {
    pub day: u64,
    pub resets_at: u64,
    pub start_used: f64,
    pub budget: f64,
    pub notified: bool,
}

pub struct OverBudget {
    pub used_today: f64,
    pub budget: f64,
}

fn start(day: u64, window: Window, now: u64) -> DayBudget {
    let days_left = window.resets_at.saturating_sub(now) as f64 / DAY_SECONDS;
    DayBudget {
        day,
        resets_at: window.resets_at,
        start_used: window.used_percentage,
        budget: (FULL_PERCENT - window.used_percentage).max(0.0) / days_left.max(1.0),
        notified: false,
    }
}

pub fn step(
    saved: Option<DayBudget>,
    day: u64,
    window: Window,
    now: u64,
) -> (DayBudget, Option<OverBudget>) {
    let same = saved.filter(|state| {
        state.day == day && state.resets_at.abs_diff(window.resets_at) < SAME_WINDOW_SECONDS
    });
    let mut state = same.unwrap_or_else(|| start(day, window, now));
    let used_today = window.used_percentage - state.start_used;
    if state.notified || used_today <= state.budget {
        return (state, None);
    }
    state.notified = true;
    let budget = state.budget;
    (state, Some(OverBudget { used_today, budget }))
}

pub fn load() -> Option<DayBudget> {
    serde_json::from_str(&std::fs::read_to_string(config::dir().join(FILE_NAME)).ok()?).ok()
}

pub fn save(state: &DayBudget) {
    if let Ok(text) = serde_json::to_string(state) {
        let _ = write_atomic(&config::dir().join(FILE_NAME), &text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;

    fn window(used: f64) -> Window {
        Window {
            used_percentage: used,
            resets_at: 10 * DAY,
        }
    }

    #[test]
    fn warns_once_when_today_goes_over_the_budget_set_at_the_start_of_the_day() {
        let now = 5 * DAY;
        let (state, over) = step(None, 5, window(20.0), now);
        assert!(over.is_none());
        assert_eq!(state.budget, 16.0);
        let (state, over) = step(Some(state), 5, window(30.0), now + 3_600);
        assert!(over.is_none());
        let (state, over) = step(Some(state), 5, window(37.0), now + 7_200);
        let over = over.expect("over budget");
        assert_eq!((over.used_today, over.budget), (17.0, 16.0));
        let (state, over) = step(Some(state), 5, window(40.0), now + 9_000);
        assert!(over.is_none() && state.notified);
        let (next_day, over) = step(Some(state), 6, window(40.0), now + DAY);
        assert!(over.is_none() && !next_day.notified && next_day.start_used == 40.0);
    }

    #[test]
    fn a_new_window_starts_a_new_budget_the_same_day() {
        let (state, _) = step(None, 5, window(90.0), 5 * DAY);
        let fresh = Window {
            used_percentage: 2.0,
            resets_at: 17 * DAY,
        };
        let (state, over) = step(Some(state), 5, fresh, 5 * DAY + 60);
        assert!(over.is_none());
        assert_eq!(state.start_used, 2.0);
    }
}
