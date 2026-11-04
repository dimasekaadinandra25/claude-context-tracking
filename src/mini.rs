use crate::session::{Session, format_tokens};
use crate::theme::{self, ACCENT, LIVE, MIN_WINDOW_SIZE, WINDOW_SIZE};
use eframe::egui::{
    Align, Context, Key, Label, Layout, PointerButton, RichText, Ui, Vec2, ViewportCommand,
    WindowLevel, vec2,
};
use std::time::SystemTime;

const SIZE: Vec2 = vec2(340.0, 110.0);
const MAX_LINES: usize = 2;
const LINE_HEIGHT: f32 = 22.0;
const LINE_GAP: f32 = 6.0;
const CARD_MARGIN: i8 = 10;
const DOT_RADIUS: f32 = 3.0;
const NAME_SHARE: f32 = 0.45;
const BAR_HEIGHT: f32 = 7.0;
const PERCENT_WIDTH: f32 = 34.0;

pub fn enter(ctx: &Context) -> Vec2 {
    let zoom = ctx.zoom_factor();
    let restore = ctx
        .input(|i| i.viewport().inner_rect)
        .map_or(Vec2::from(WINDOW_SIZE), |rect| rect.size() * zoom);
    ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop));
    ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(Vec2::ZERO));
    resize(ctx, SIZE);
    restore
}

pub fn leave(ctx: &Context, restore: Vec2) {
    ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::Normal));
    ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(
        Vec2::from(MIN_WINDOW_SIZE) / ctx.zoom_factor(),
    ));
    resize(ctx, restore);
}

fn resize(ctx: &Context, logical: Vec2) {
    ctx.send_viewport_cmd(ViewportCommand::InnerSize(logical / ctx.zoom_factor()));
}

pub fn show(ui: &mut Ui, sessions: &[Session], today: u64) -> bool {
    let active = active(sessions, SystemTime::now());
    let lines = if active.is_empty() { 2 } else { active.len() } as f32;
    theme::glass(ui)
        .inner_margin(CARD_MARGIN)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.spacing_mut().item_spacing.y = LINE_GAP;
            let height = lines * LINE_HEIGHT + (lines - 1.0) * LINE_GAP;
            ui.add_space(((ui.available_height() - height) / 2.0).max(0.0));
            if active.is_empty() {
                line(ui, |ui| {
                    ui.add(Label::new(RichText::new("No active session").weak()).truncate())
                });
                line(ui, |ui| {
                    ui.add(
                        Label::new(
                            RichText::new(format!("+{} new context today", format_tokens(today)))
                                .strong()
                                .color(ACCENT),
                        )
                        .truncate(),
                    )
                });
            }
            for session in active {
                line(ui, |ui| session_line(ui, session));
            }
        })
        .response
        .on_hover_text("Double-click or press Esc to return");
    ui.input(|i| {
        i.pointer.button_double_clicked(PointerButton::Primary) || i.key_pressed(Key::Escape)
    })
}

fn line<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), LINE_HEIGHT),
        Layout::left_to_right(Align::Center),
        add,
    );
}

fn session_line(ui: &mut Ui, session: &Session) {
    let (name, bar) = split_line(ui.available_width(), ui.spacing().item_spacing.x);
    theme::dot(ui, DOT_RADIUS, LIVE);
    ui.allocate_ui_with_layout(
        vec2(name, LINE_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_width(name);
            ui.add(Label::new(RichText::new(session.name()).strong()).truncate());
        },
    );
    let fraction = session.usage_fraction();
    theme::gradient_bar(ui, fraction, vec2(bar, BAR_HEIGHT));
    ui.allocate_ui_with_layout(
        vec2(PERCENT_WIDTH, LINE_HEIGHT),
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.set_min_width(PERCENT_WIDTH);
            ui.label(RichText::new(percent(fraction)).small());
        },
    );
}

fn split_line(width: f32, gap: f32) -> (f32, f32) {
    let rest = (width - DOT_RADIUS * 2.0 - PERCENT_WIDTH - 3.0 * gap).max(0.0);
    let name = rest * NAME_SHARE;
    (name, rest - name)
}

fn active(sessions: &[Session], now: SystemTime) -> Vec<&Session> {
    sessions
        .iter()
        .filter(|s| s.is_active(now))
        .take(MAX_LINES)
        .collect()
}

fn percent(fraction: f32) -> String {
    format!("{:.0}%", fraction * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn session(id: &str, new_context: u64, latest_at: &str) -> Session {
        Session {
            id: id.into(),
            new_context,
            latest_at: latest_at.into(),
            ..Session::default()
        }
    }

    #[test]
    fn shows_first_two_active_sessions() {
        let now = UNIX_EPOCH + Duration::from_secs(1_793_324_739);
        let sessions = [
            session("idle", 10, "2026-10-30T01:00:00Z"),
            session("a", 0, "2026-10-30T01:45:00Z"),
            session("old", 10, "2026-10-30T01:00:00Z"),
            session("b", 10, "2026-10-30T01:44:00Z"),
            session("c", 10, "2026-10-30T01:43:00Z"),
        ];
        let ids: Vec<&str> = active(&sessions, now)
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids, ["a", "b"]);
        assert!(active(&sessions[..1], now).is_empty());
    }

    #[test]
    fn line_splits_width_and_never_goes_negative() {
        let (name, bar) = split_line(302.0, 8.0);
        assert!((name - 107.1).abs() < 0.01);
        assert!((bar - 130.9).abs() < 0.01);
        assert_eq!(split_line(60.0, 8.0), (0.0, 0.0));
        let (wide_name, wide_bar) = split_line(800.0, 8.0);
        assert!(wide_name > name && wide_bar > bar);
    }

    #[test]
    fn percent_rounds_to_whole_number() {
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.456), "46%");
        assert_eq!(percent(1.0), "100%");
    }
}
