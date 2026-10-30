use crate::overhead::{Item, Overhead};
use crate::session::{Session, format_tokens, share};
use crate::theme::ACCENT;
use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Ui, vec2};

const PANEL_WIDTH: f32 = 360.0;
const BAR_HEIGHT: f32 = 8.0;
const SWATCH_SIZE: f32 = 10.0;
const SYSTEM_COLOR: Color32 = ACCENT;
const AGENTS_COLOR: Color32 = Color32::from_rgb(170, 130, 220);
const MEMORY_COLOR: Color32 = Color32::from_rgb(230, 80, 80);
const SKILLS_COLOR: Color32 = Color32::from_rgb(110, 210, 190);
const MESSAGES_COLOR: Color32 = Color32::from_rgb(120, 130, 150);
const ESTIMATE_HINT: &str = "Estimated from files on disk";

struct Category {
    name: &'static str,
    tokens: u64,
    color: Color32,
    estimated: bool,
    hint: &'static str,
}

pub fn show(ui: &mut Ui, session: &Session, overhead: &Overhead) {
    ui.set_width(PANEL_WIDTH);
    ui.label(RichText::new(&session.model).strong());
    ui.label(format!(
        "{} / {} tokens ({:.0}%)",
        session.latest_tokens_label(),
        format_tokens(session.context_window),
        session.usage_fraction() * 100.0
    ));

    let categories = categories(ui, session, overhead);
    let used = &categories[..categories.len() - 1];
    usage_bar(ui, used, session.context_window);
    ui.add_space(6.0);
    categories_table(ui, &categories, session.context_window);

    item_list(ui, "MEMORY FILES", &overhead.memory_files);
    item_list(ui, "CUSTOM AGENTS", &overhead.agents);

    let usage = session.last_usage;
    section(ui, "LATEST REQUEST");
    key_values(
        ui,
        "latest_request",
        &[
            ("Cache read", format_tokens(usage.cache_read_input_tokens)),
            (
                "Cache write",
                format_tokens(usage.cache_creation_input_tokens),
            ),
            ("Uncached input", format_tokens(usage.input_tokens)),
            ("Output", format_tokens(usage.output_tokens)),
        ],
    );

    section(ui, "SESSION");
    key_values(
        ui,
        "session",
        &[
            ("Start context", format_tokens(session.start_context)),
            ("New context", session.new_context_label()),
            ("Peak context", format_tokens(session.peak_context)),
            ("Compactions", session.compactions.to_string()),
            ("Requests", session.requests.to_string()),
            ("First request", format_timestamp(&session.first_at)),
            ("Last request", format_timestamp(&session.latest_at)),
        ],
    );
    ui.add_space(4.0);
    ui.label(RichText::new(&session.project).small().weak());
    ui.label(RichText::new(&session.id).small().weak().monospace());
}

fn categories(ui: &Ui, session: &Session, overhead: &Overhead) -> [Category; 6] {
    [
        Category {
            name: "System prompt & tools",
            tokens: session.system_context().saturating_sub(overhead.total()),
            color: SYSTEM_COLOR,
            estimated: true,
            hint: "First request of the session minus the estimated parts below: system prompt, built-in and MCP tools, MCP instructions and the first message",
        },
        Category {
            name: "Custom agents",
            tokens: overhead.agent_tokens(),
            color: AGENTS_COLOR,
            estimated: true,
            hint: ESTIMATE_HINT,
        },
        Category {
            name: "Memory files",
            tokens: overhead.memory_tokens(),
            color: MEMORY_COLOR,
            estimated: true,
            hint: ESTIMATE_HINT,
        },
        Category {
            name: "Skills",
            tokens: overhead.skills,
            color: SKILLS_COLOR,
            estimated: true,
            hint: "Estimated from skill and command files of enabled plugins and your skill folders",
        },
        Category {
            name: "Messages",
            tokens: session.conversation_context(),
            color: MESSAGES_COLOR,
            estimated: session.estimated,
            hint: "Measured: messages, tool results and thinking added after the first request. Estimated after /compact or /model until the next request",
        },
        Category {
            name: "Free space",
            tokens: session.free_space(),
            color: ui.visuals().widgets.inactive.bg_fill,
            estimated: false,
            hint: "Room left in the context window",
        },
    ]
}

fn categories_table(ui: &mut Ui, categories: &[Category], window: u64) {
    egui::Grid::new("categories")
        .num_columns(3)
        .spacing([24.0, 6.0])
        .show(ui, |ui| {
            header_row(ui, ["CATEGORY", "TOKENS", "USAGE"]);
            for category in categories {
                ui.horizontal(|ui| {
                    swatch(ui, category.color);
                    ui.label(category.name);
                })
                .response
                .on_hover_text(category.hint);
                let prefix = if category.estimated { "≈ " } else { "" };
                right_aligned(ui, format!("{prefix}{}", format_tokens(category.tokens)));
                right_aligned(ui, percent(category.tokens, window));
                ui.end_row();
            }
        });
}

fn item_list(ui: &mut Ui, title: &str, items: &[Item]) {
    if items.is_empty() {
        return;
    }
    section(ui, title);
    for item in items {
        let row_height = ui.spacing().interact_size.y;
        ui.allocate_ui_with_layout(
            vec2(PANEL_WIDTH, row_height),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.label(RichText::new(format!("≈ {}", item.tokens)).small());
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&item.name).small()).truncate())
                        .on_hover_text(&item.name);
                });
            },
        );
    }
}

fn usage_bar(ui: &mut Ui, segments: &[Category], total: u64) {
    let (rect, _) = ui.allocate_exact_size(vec2(PANEL_WIDTH, BAR_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, BAR_HEIGHT / 2.0, ui.visuals().extreme_bg_color);
    let mut left = rect.left();
    for segment in segments {
        let width = rect.width() * share(segment.tokens, total);
        let segment_rect =
            egui::Rect::from_min_size(egui::pos2(left, rect.top()), vec2(width, BAR_HEIGHT));
        painter.rect_filled(segment_rect, 0.0, segment.color);
        left += width;
    }
}

fn header_row(ui: &mut Ui, headers: [&str; 3]) {
    for (i, header) in headers.into_iter().enumerate() {
        let text = RichText::new(header).small().weak();
        if i == 0 {
            ui.label(text);
        } else {
            right_aligned(ui, text);
        }
    }
    ui.end_row();
}

fn key_values(ui: &mut Ui, id: &str, rows: &[(&str, String)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([24.0, 4.0])
        .show(ui, |ui| {
            for (key, value) in rows {
                ui.label(*key);
                right_aligned(ui, value.as_str());
                ui.end_row();
            }
        });
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).small().weak());
}

fn swatch(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(SWATCH_SIZE, SWATCH_SIZE), Sense::hover());
    ui.painter().rect_filled(rect, 2.0, color);
}

fn right_aligned(ui: &mut Ui, text: impl Into<egui::WidgetText>) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(text));
}

fn percent(part: u64, whole: u64) -> String {
    let value = share(part, whole) * 100.0;
    if part > 0 && value < 0.1 {
        "<0.1%".to_owned()
    } else {
        format!("{value:.1}%")
    }
}

fn format_timestamp(at: &str) -> String {
    match at.get(..16) {
        Some(minutes) if at.contains('T') => format!("{} UTC", minutes.replace('T', " ")),
        _ => at.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_formats_small_and_regular_shares() {
        assert_eq!(percent(0, 1_000_000), "0.0%");
        assert_eq!(percent(651, 1_000_000), "<0.1%");
        assert_eq!(percent(346_200, 1_000_000), "34.6%");
    }

    #[test]
    fn timestamps_show_minutes_in_utc() {
        assert_eq!(
            format_timestamp("2026-10-29T07:06:12.345Z"),
            "2026-10-29 07:06 UTC"
        );
        assert_eq!(format_timestamp(""), "");
    }
}
