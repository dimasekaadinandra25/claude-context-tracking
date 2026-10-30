use crate::details;
use crate::overhead::{self, Overhead};
use crate::platform::{self, APP_NAME};
use crate::session::{Session, format_age, format_tokens, total_new_context};
use crate::theme::{self, ACCENT};
use crate::tray::{self, Tray};
use crate::watcher::{self, Update};
use eframe::egui::{self, Align, Color32, Layout, ProgressBar, RichText, Ui, vec2};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

const LIVE_COLOR: Color32 = Color32::from_rgb(60, 180, 90);
const ACTIVE_TEXT_SIZE: f32 = 20.0;
const ACTIVE_ROW_HEIGHT: f32 = 30.0;
const ACTIVE_BAR_HEIGHT: f32 = 8.0;
const ROW_HEIGHT: f32 = 22.0;
const INFO_RADIUS: f32 = 7.0;
const IDLE_BAR_WIDTH: f32 = 200.0;
const IDLE_VALUE_WIDTH: f32 = 64.0;
const IDLE_MODEL_WIDTH: f32 = 110.0;
const IDLE_INSET: i8 = 16;
const CARD_MARGIN: i8 = 14;
const CARD_SPACING: f32 = 8.0;
const SEARCH_WIDTH: f32 = 220.0;
const ACTIVITY_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const OVERHEAD_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const NEW_CONTEXT_HINT: &str = "Context added since start. Keeps counting through /compact: a compaction adds nothing, growth after it adds on top.";
static NO_OVERHEAD: Overhead = Overhead::EMPTY;

pub struct App {
    updates: Receiver<Update>,
    indexing: Option<(usize, usize)>,
    machine: String,
    sessions: Vec<Session>,
    filter: String,
    details_for: Option<String>,
    overheads: HashMap<String, Overhead>,
    overheads_refreshed: Option<Instant>,
    tray: Option<Tray>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        Self {
            updates: watcher::spawn(cc.egui_ctx.clone()),
            indexing: Some((0, 0)),
            machine: platform::machine_name(),
            sessions: Vec::new(),
            filter: String::new(),
            details_for: None,
            overheads: HashMap::new(),
            overheads_refreshed: None,
            tray: tray::spawn(cc.egui_ctx.clone()),
        }
    }

    fn hide_to_tray_on_close(&self, ctx: &egui::Context) {
        if self.tray.is_some() && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::set_window_visible(ctx, false);
        }
    }

    fn apply_updates(&mut self) {
        let mut latest = None;
        for update in self.updates.try_iter() {
            match update {
                Update::Indexing { done, total } => self.indexing = Some((done, total)),
                Update::Sessions(sessions) => latest = Some(sessions),
            }
        }
        if let Some(mut sessions) = latest {
            sessions.sort_by(|a, b| b.latest_at.cmp(&a.latest_at));
            self.sessions = sessions;
            self.indexing = None;
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading(RichText::new(APP_NAME).strong());
            ui.label(RichText::new(&self.machine).weak());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.indexing.is_none() {
                    ui.colored_label(LIVE_COLOR, "● Live").on_hover_text(
                        "Watching session files; updates within ~2s of each request",
                    );
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("Search sessions")
                        .desired_width(SEARCH_WIDTH),
                );
            });
        });
        if let Some((done, total)) = self.indexing {
            let fraction = if total == 0 {
                0.0
            } else {
                done as f32 / total as f32
            };
            ui.add(
                ProgressBar::new(fraction).text(format!("Indexing sessions… {done}/{total} files")),
            );
        }
        ui.add_space(6.0);
    }

    fn refresh_overheads(&mut self) {
        let missing = self
            .sessions
            .iter()
            .any(|s| !self.overheads.contains_key(&s.project));
        let stale = self
            .overheads_refreshed
            .is_none_or(|at| at.elapsed() >= OVERHEAD_REFRESH_INTERVAL);
        if !missing && !stale {
            return;
        }
        let claude_dir = platform::claude_dir();
        self.overheads = self
            .sessions
            .iter()
            .map(|s| s.project.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .map(|project| {
                let estimate = overhead::estimate(&claude_dir, &project);
                (project, estimate)
            })
            .collect();
        self.overheads_refreshed = Some(Instant::now());
    }

    fn overhead(&self, session: &Session) -> &Overhead {
        self.overheads.get(&session.project).unwrap_or(&NO_OVERHEAD)
    }

    fn visible_sessions(&self) -> impl Iterator<Item = &Session> {
        let query = self.filter.to_lowercase();
        self.sessions
            .iter()
            .filter(move |s| matches_query(s, &query))
    }

    fn totals_bar(&self, ui: &mut Ui) {
        let now = SystemTime::now();
        let (count, active) = self.visible_sessions().fold((0, 0), |(count, active), s| {
            (count + 1, active + usize::from(s.is_active(now)))
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Total new context").weak());
            ui.label(
                RichText::new(format!(
                    "+{}",
                    format_tokens(total_new_context(self.visible_sessions()))
                ))
                .strong()
                .color(ACCENT),
            );
            ui.label(RichText::new(format!("·  {active} active  ·  {count} sessions")).weak());
        });
        ui.add_space(4.0);
    }

    fn details_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.details_for.clone() else {
            return;
        };
        let Some(session) = self.sessions.iter().find(|s| s.id == id) else {
            self.details_for = None;
            return;
        };
        let mut open = true;
        egui::Window::new("Context usage")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| details::show(ui, session, self.overhead(session)));
        if !open {
            self.details_for = None;
        }
    }

    fn sessions_list(&self, ui: &mut Ui) -> Option<String> {
        let now = SystemTime::now();
        let (active, idle): (Vec<_>, Vec<_>) =
            self.visible_sessions().partition(|s| s.is_active(now));
        let mut opened = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for session in active {
                if self.active_card(ui, session) {
                    opened = Some(session.id.clone());
                }
                ui.add_space(CARD_SPACING);
            }
            if !idle.is_empty() {
                ui.add_space(CARD_SPACING);
                idle_frame().show(ui, |ui| ui.label(RichText::new("IDLE").small().weak()));
            }
            for session in idle {
                if self.idle_row(ui, session, now) {
                    opened = Some(session.id.clone());
                }
            }
        });
        opened
    }

    fn active_card(&self, ui: &mut Ui, session: &Session) -> bool {
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke((1.5, ACCENT))
            .corner_radius(10)
            .inner_margin(CARD_MARGIN)
            .show(ui, |ui| {
                split_row(
                    ui,
                    ACTIVE_ROW_HEIGHT,
                    |ui| {
                        ui.label(
                            RichText::new(session.new_context_label())
                                .size(ACTIVE_TEXT_SIZE)
                                .strong()
                                .color(ACCENT),
                        )
                        .on_hover_text(NEW_CONTEXT_HINT);
                    },
                    |ui| {
                        ui.label(RichText::new("●").size(ACTIVE_TEXT_SIZE).color(LIVE_COLOR));
                        name_label(
                            ui,
                            session,
                            RichText::new(session.name())
                                .size(ACTIVE_TEXT_SIZE)
                                .strong(),
                        );
                    },
                );
                let (info, ()) = split_row(
                    ui,
                    ROW_HEIGHT,
                    |ui| {
                        let info = self.info_button(ui, session);
                        ui.label(RichText::new(session.latest_label()).strong());
                        info
                    },
                    |ui| {
                        ui.label(RichText::new(summary(session)).weak());
                    },
                );
                let bar = self.context_bar(
                    ui,
                    session,
                    ProgressBar::new(session.usage_fraction())
                        .fill(ACCENT)
                        .desired_height(ACTIVE_BAR_HEIGHT),
                );
                info || bar
            })
            .inner
    }

    fn idle_row(&self, ui: &mut Ui, session: &Session, now: SystemTime) -> bool {
        idle_frame()
            .show(ui, |ui| {
                split_row(
                    ui,
                    ROW_HEIGHT,
                    |ui| {
                        let age = session
                            .idle_for(now)
                            .map(|idle| format!("{} ago", format_age(idle)))
                            .unwrap_or_default();
                        fixed_width_label(ui, IDLE_VALUE_WIDTH, RichText::new(age).small().weak());
                        fixed_width_label(
                            ui,
                            IDLE_VALUE_WIDTH,
                            RichText::new(session.new_context_label()).small().weak(),
                        )
                        .on_hover_text(NEW_CONTEXT_HINT);
                        let info = self.info_button(ui, session);
                        let bar = self.context_bar(
                            ui,
                            session,
                            ProgressBar::new(session.usage_fraction())
                                .fill(ui.visuals().widgets.noninteractive.bg_stroke.color)
                                .desired_width(IDLE_BAR_WIDTH)
                                .text(
                                    RichText::new(session.latest_label())
                                        .small()
                                        .color(ui.visuals().text_color()),
                                ),
                        );
                        fixed_width_label(
                            ui,
                            IDLE_MODEL_WIDTH,
                            RichText::new(&session.model).small().weak(),
                        );
                        info || bar
                    },
                    |ui| name_label(ui, session, RichText::new(session.name()).weak()),
                )
                .0
            })
            .inner
    }

    fn context_bar(&self, ui: &mut Ui, session: &Session, bar: ProgressBar) -> bool {
        let response = ui.add(bar).interact(egui::Sense::click());
        self.details_trigger(response, session)
    }

    fn info_button(&self, ui: &mut Ui, session: &Session) -> bool {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ROW_HEIGHT, ROW_HEIGHT), egui::Sense::click());
        let color = if response.hovered() {
            ACCENT
        } else {
            ui.visuals().text_color()
        };
        let painter = ui.painter();
        painter.circle_stroke(rect.center(), INFO_RADIUS, (1.0, color));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "i",
            egui::FontId::proportional(INFO_RADIUS * 1.5),
            color,
        );
        self.details_trigger(response, session)
    }

    fn details_trigger(&self, response: egui::Response, session: &Session) -> bool {
        let clicked = response.clicked();
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_ui(|ui| details::show(ui, session, self.overhead(session)));
        clicked
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.hide_to_tray_on_close(ui.ctx());
        self.apply_updates();
        self.refresh_overheads();
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("totals").show(ui, |ui| self.totals_bar(ui));
        let opened = egui::CentralPanel::default()
            .show(ui, |ui| self.sessions_list(ui))
            .inner;
        if opened.is_some() {
            self.details_for = opened;
        }
        self.details_window(ui.ctx());
        ui.ctx().request_repaint_after(ACTIVITY_REFRESH_INTERVAL);
    }
}

fn split_row<R, L>(
    ui: &mut Ui,
    height: f32,
    right: impl FnOnce(&mut Ui) -> R,
    left: impl FnOnce(&mut Ui) -> L,
) -> (R, L) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), height),
        Layout::right_to_left(Align::Center),
        |ui| {
            let right = right(ui);
            let left = ui.with_layout(Layout::left_to_right(Align::Center), left);
            (right, left.inner)
        },
    )
    .inner
}

fn idle_frame() -> egui::Frame {
    egui::Frame::new().inner_margin(egui::Margin::symmetric(IDLE_INSET, 2))
}

fn fixed_width_label(ui: &mut Ui, width: f32, text: RichText) -> egui::Response {
    ui.add_sized([width, ROW_HEIGHT], egui::Label::new(text).truncate())
}

fn name_label(ui: &mut Ui, session: &Session, text: RichText) {
    ui.add(egui::Label::new(text).truncate())
        .on_hover_text(format!("{}\n{}", session.project, session.id));
}

fn summary(session: &Session) -> String {
    let mut parts = vec![
        session.model.clone(),
        format!("started at {}", format_tokens(session.start_context)),
    ];
    if session.compactions > 0 {
        parts.push(format!("compacted {}×", session.compactions));
    }
    parts.join("  ·  ")
}

fn matches_query(session: &Session, query: &str) -> bool {
    query.is_empty()
        || [
            session.name(),
            session.model.as_str(),
            session.project.as_str(),
        ]
        .iter()
        .any(|field| field.to_lowercase().contains(query))
}
