use crate::alerts;
use crate::session::{Session, format_tokens, local_offset, unix_seconds};
use crate::theme;
use eframe::egui::{Align, Layout, RichText, Ui, vec2};
use std::collections::BTreeMap;

pub const TITLE: &str = "Models · last 7 days";
const PANEL_WIDTH: f32 = 340.0;
const BAR_HEIGHT: f32 = 6.0;
const OUTPUT_WEIGHT: u64 = 5;
const DAYS: u64 = 7;
const DAY_SECONDS: u64 = 86_400;
const TENTHS: u64 = 1_000;
const MODEL_PREFIX: &str = "claude-";
const NOTE: &str = "Share of tokens: input including cache, output counts 5×.";

#[derive(Debug, Default, PartialEq)]
pub struct ModelTotal {
    pub model: String,
    pub requests: u64,
    pub input: u64,
    pub output: u64,
    pub share: f64,
}

impl ModelTotal {
    fn weight(&self) -> u64 {
        self.input + OUTPUT_WEIGHT * self.output
    }
}

pub fn since(now: u64) -> u64 {
    alerts::day_start(now, local_offset()).saturating_sub((DAYS - 1) * DAY_SECONDS)
}

pub fn totals<'a>(sessions: impl IntoIterator<Item = &'a Session>, since: u64) -> Vec<ModelTotal> {
    let mut by_model: BTreeMap<&str, ModelTotal> = BTreeMap::new();
    for ((hour, model), usage) in sessions.into_iter().flat_map(|s| &s.hours) {
        if unix_seconds(hour).is_none_or(|at| at < since) {
            continue;
        }
        let total = by_model
            .entry(model.as_str())
            .or_insert_with(|| ModelTotal {
                model: model.clone(),
                ..ModelTotal::default()
            });
        total.requests += u64::from(usage.requests);
        total.input += usage.input_tokens + usage.cache_write_tokens + usage.cache_read_tokens;
        total.output += usage.output_tokens;
    }
    let mut rows: Vec<ModelTotal> = by_model
        .into_values()
        .filter(|row| row.weight() > 0)
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.weight()));
    let shares = shares(&rows.iter().map(ModelTotal::weight).collect::<Vec<_>>());
    for (row, share) in rows.iter_mut().zip(shares) {
        row.share = share;
    }
    rows
}

pub fn shares(weights: &[u64]) -> Vec<f64> {
    let sum: u64 = weights.iter().sum();
    if sum == 0 {
        return vec![0.0; weights.len()];
    }
    let exact: Vec<f64> = weights
        .iter()
        .map(|weight| *weight as f64 / sum as f64 * TENTHS as f64)
        .collect();
    let mut rounded: Vec<u64> = exact.iter().map(|value| value.floor() as u64).collect();
    let left = TENTHS - rounded.iter().sum::<u64>();
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by(|&a, &b| {
        (exact[b] - rounded[b] as f64).total_cmp(&(exact[a] - rounded[a] as f64))
    });
    for &index in order.iter().take(left as usize) {
        rounded[index] += 1;
    }
    rounded.iter().map(|tenths| *tenths as f64 / 10.0).collect()
}

pub fn label(model: &str) -> String {
    let short = model.strip_prefix(MODEL_PREFIX).unwrap_or(model);
    let mut parts: Vec<&str> = short.split('-').collect();
    if parts.len() > 1
        && parts
            .last()
            .is_some_and(|part| part.len() == 8 && part.bytes().all(|b| b.is_ascii_digit()))
    {
        parts.pop();
    }
    let Some((family, version)) = parts.split_first() else {
        return short.to_owned();
    };
    let mut name: String = family
        .chars()
        .take(1)
        .flat_map(char::to_uppercase)
        .collect();
    name.push_str(&family[family.chars().next().map_or(0, char::len_utf8)..]);
    if version.is_empty() {
        name
    } else {
        format!("{name} {}", version.join("."))
    }
}

pub fn show(ui: &mut Ui, rows: &[ModelTotal]) {
    ui.set_width(PANEL_WIDTH);
    if rows.is_empty() {
        ui.label(RichText::new("No usage in the last 7 days.").weak());
        return;
    }
    for row in rows {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label(&row.model)).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.1}%", row.share)).strong());
            });
        });
        let width = ui.available_width();
        theme::gradient_bar(ui, (row.share / 100.0) as f32, vec2(width, BAR_HEIGHT));
        ui.label(
            RichText::new(format!(
                "{} req · {} in · {} out",
                row.requests,
                format_tokens(row.input),
                format_tokens(row.output)
            ))
            .small()
            .weak(),
        );
        ui.add_space(6.0);
    }
    ui.label(RichText::new(NOTE).small().weak());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::HourUsage;

    fn usage(requests: u32, cache_read: u64, output: u64) -> HourUsage {
        HourUsage {
            requests,
            cache_read_tokens: cache_read,
            output_tokens: output,
            ..HourUsage::default()
        }
    }

    #[test]
    fn totals_add_up_per_model_inside_the_window_and_share_sums_to_100() {
        let mut a = Session::new("a".into());
        a.hours.insert(
            ("2026-11-11T02:00:00Z".into(), "claude-opus-5-5".into()),
            usage(3, 900, 10),
        );
        a.hours.insert(
            ("2026-11-11T03:00:00Z".into(), "claude-haiku-5-5".into()),
            usage(1, 50, 0),
        );
        a.hours.insert(
            ("2026-11-01T03:00:00Z".into(), "claude-fable-5-1".into()),
            usage(9, 9_000, 0),
        );
        let mut b = Session::new("b".into());
        b.hours.insert(
            ("2026-11-10T05:00:00Z".into(), "claude-opus-5-5".into()),
            usage(2, 50, 0),
        );

        let since = unix_seconds("2026-11-05T00:00:00Z").unwrap();
        let rows = totals([&a, &b], since);
        assert_eq!(
            rows.iter().map(|r| r.model.as_str()).collect::<Vec<_>>(),
            ["claude-opus-5-5", "claude-haiku-5-5"]
        );
        assert_eq!(
            (rows[0].requests, rows[0].input, rows[0].output),
            (5, 950, 10)
        );
        assert_eq!(rows[0].share + rows[1].share, 100.0);
        assert_eq!((rows[0].share, rows[1].share), (95.2, 4.8));
    }

    #[test]
    fn shares_are_rounded_together() {
        assert_eq!(shares(&[972_600, 13_700, 13_700]), [97.2, 1.4, 1.4]);
        assert_eq!(shares(&[1, 1, 1]), [33.4, 33.3, 33.3]);
        assert_eq!(shares(&[0, 0]), [0.0, 0.0]);
    }

    #[test]
    fn labels_read_like_claude_model_names() {
        assert_eq!(label("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(label("claude-fable-5-1"), "Fable 5.1");
        assert_eq!(label("claude-haiku-4-5-20251001"), "Haiku 4.5");
        assert_eq!(label("<synthetic>"), "<synthetic>");
    }
}
