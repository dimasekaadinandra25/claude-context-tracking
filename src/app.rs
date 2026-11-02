use crate::alerts::{self, FullWarnings};
use crate::autostart;
use crate::config;
use crate::details;
use crate::mini;
use crate::overhead::{self, Overhead};
use crate::platform::{self, APP_NAME, APP_VERSION};
use crate::session::{Session, format_age, format_tokens, total_new_context};
use crate::sync::SyncStatus;
use crate::theme::{self, ACCENT, FULL_AT, LIVE};
use crate::tray::{self, Tray};
use crate::watcher::{self, Command, Update};
use eframe::egui::{self, Align, Layout, ProgressBar, RichText, Ui, vec2};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ACTIVE_TEXT_SIZE: f32 = 18.0;
const NAME_TEXT_SIZE: f32 = 13.0;
const LOGO_SIZE: f32 = 18.0;
const LIVE_DOT_RADIUS: f32 = 3.0;
const ACTIVE_DOT_RADIUS: f32 = 4.0;
const DOT_GAP: f32 = 5.0;
const ICON_SIZE: f32 = 16.0;
const ACTIVE_ROW_HEIGHT: f32 = 24.0;
const ACTIVE_BAR_HEIGHT: f32 = 7.0;
const ROW_HEIGHT: f32 = 20.0;
const INFO_RADIUS: f32 = 6.0;
const IDLE_VALUE_WIDTH: f32 = 52.0;
const IDLE_CONTEXT_WIDTH: f32 = 80.0;
const IDLE_MODEL_WIDTH: f32 = 88.0;
const IDLE_INSET: i8 = 14;
const IDLE_PANEL_MARGIN: i8 = 6;
const SECTION_SPACING: f32 = 4.0;
const FOLD_INDENT: f32 = 22.0;
const CARD_MARGIN: i8 = 11;
const CARD_SPACING: f32 = 8.0;
const SEARCH_WIDTH: f32 = 100.0;
const TOP_ROW_HEIGHT: f32 = 28.0;
const LIVE_PILL_WIDTH: f32 = 64.0;
const PILL_PADDING: f32 = 28.0;
const SEARCH_ICON_SIZE: f32 = 13.0;
const SEARCH_HEIGHT: f32 = 20.0;
const SEARCH_ID: &str = "search";
const OLDER_THAN: Duration = Duration::from_secs(7 * 86_400);
const SETTINGS_FIELD_WIDTH: f32 = 320.0;
const SYNC_REASON_CHARS: usize = 60;
const ACTIVITY_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const OVERHEAD_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const NEW_CONTEXT_HINT: &str = "Context added since start. Keeps counting through /compact: a compaction adds nothing, growth after it adds on top.";
static NO_OVERHEAD: Overhead = Overhead::EMPTY;

pub struct App {
    updates: Receiver<Update>,
    commands: Sender<Command>,
    mini: Option<egui::Vec2>,
    indexing: Option<(usize, usize)>,
    machine: String,
    sessions: Vec<Session>,
    filter: String,
    details_for: Option<String>,
    overheads: HashMap<String, Overhead>,
    overheads_refreshed: Option<Instant>,
    tray: Option<Tray>,
    sync: Option<SyncStatus>,
    settings: Option<Settings>,
    idle_sort: IdleSort,
    warnings: FullWarnings,
    tray_tooltip: String,
}

#[derive(Clone, Copy, PartialEq)]
enum IdleSort {
    Recent,
    Largest,
}

struct Settings {
    api_key: String,
    start_with_windows: bool,
}

impl Settings {
    fn load() -> Self {
        Self {
            api_key: config::load().api_key,
            start_with_windows: autostart::is_enabled(),
        }
    }

    fn save(&self) -> bool {
        config::save_api_key(&self.api_key).is_ok()
            && autostart::set(self.start_with_windows).is_ok()
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        cc.egui_ctx.set_theme(if config::load().light_mode {
            egui::Theme::Light
        } else {
            egui::Theme::Dark
        });
        let (tx, updates) = channel();
        let commands = watcher::spawn(cc.egui_ctx.clone(), tx.clone());
        Self {
            updates,
            commands: commands.clone(),
            mini: None,
            indexing: Some((0, 0)),
            machine: platform::machine_name(),
            sessions: Vec::new(),
            filter: String::new(),
            details_for: None,
            overheads: HashMap::new(),
            overheads_refreshed: None,
            tray: tray::spawn(cc.egui_ctx.clone(), tx, commands),
            sync: None,
            settings: None,
            idle_sort: IdleSort::Recent,
            warnings: FullWarnings::default(),
            tray_tooltip: String::new(),
        }
    }

    fn hide_to_tray_on_close(&self, ctx: &egui::Context) {
        if self.tray.is_some() && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::set_window_visible(ctx, false);
        }
    }

    fn apply_updates(&mut self, ctx: &egui::Context) {
        let mut latest = None;
        let mut toggle_mini = false;
        for update in self.updates.try_iter() {
            match update {
                Update::Indexing { done, total } => self.indexing = Some((done, total)),
                Update::Sessions(sessions) => latest = Some(sessions),
                Update::Sync(status) => self.sync = Some(status),
                Update::ToggleMini => toggle_mini = !toggle_mini,
            }
        }
        if toggle_mini {
            self.toggle_mini(ctx);
        }
        if let Some(mut sessions) = latest {
            sessions.sort_by(|a, b| b.latest_at.cmp(&a.latest_at));
            self.sessions = sessions;
            self.indexing = None;
        }
    }

    fn toggle_mini(&mut self, ctx: &egui::Context) {
        self.mini = match self.mini.take() {
            Some(restore) => {
                mini::leave(ctx, restore);
                None
            }
            None => Some(mini::enter(ctx)),
        };
    }

    fn new_context_today(&self) -> u64 {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        alerts::new_context_since(&self.sessions, alerts::day_start(seconds, local_offset()))
    }

    fn refresh_alerts(&mut self) {
        let now = SystemTime::now();
        let active: Vec<&Session> = self.sessions.iter().filter(|s| s.is_active(now)).collect();
        for session in self.warnings.newly_full(active.iter().copied()) {
            alerts::notify_full(session);
        }
        let Some(tray) = &self.tray else {
            return;
        };
        let sync = self.sync.as_ref().map_or_else(
            || "Sync off".to_owned(),
            |status| sync_text(status, local_offset()).0,
        );
        let text = alerts::tray_text(self.new_context_today(), active.len(), &sync);
        if text != self.tray_tooltip {
            tray::set_tooltip(tray, &text);
            self.tray_tooltip = text;
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let (find, escape) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if find {
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique(SEARCH_ID)));
        }
        if escape && self.settings.take().is_none() && self.details_for.take().is_none() {
            self.filter.clear();
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        let filter = &mut self.filter;
        let (machine, live) = (self.machine.as_str(), self.indexing.is_none());
        let ((settings, mini), ()) = split_row(
            ui,
            TOP_ROW_HEIGHT,
            |ui| {
                let settings = glyph_button(ui, "⚙", "Settings");
                mode_button(ui);
                let mini = icon_button(
                    ui,
                    theme::Icon::Mini,
                    "Mini mode: small always-on-top widget",
                );
                search_box(ui, filter);
                (settings, mini)
            },
            |ui| title_area(ui, machine, live),
        );
        if settings {
            self.settings = Some(Settings::load());
        }
        if mini {
            self.toggle_mini(&ui.ctx().clone());
        }
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
        split_row(
            ui,
            ROW_HEIGHT,
            |ui| {
                ui.label(RichText::new(format!("{active} active  ·  {count} sessions")).weak());
                if let Some(status) = &self.sync {
                    sync_label(ui, status);
                }
                if self.sync != Some(SyncStatus::Off)
                    && icon_button(ui, theme::Icon::Refresh, "Sync now")
                {
                    let _ = self.commands.send(Command::SyncNow);
                }
            },
            |ui| {
                ui.label(RichText::new("Total new context").weak());
                ui.label(
                    RichText::new(format!(
                        "+{}",
                        format_tokens(total_new_context(self.visible_sessions()))
                    ))
                    .strong()
                    .color(ACCENT),
                );
            },
        );
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

    fn settings_window(&mut self, ctx: &egui::Context) {
        let Some(settings) = &mut self.settings else {
            return;
        };
        let mut open = true;
        let mut save = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("API key");
                    ui.add(
                        egui::TextEdit::singleline(&mut settings.api_key)
                            .password(true)
                            .desired_width(SETTINGS_FIELD_WIDTH),
                    );
                });
                if autostart::SUPPORTED {
                    ui.checkbox(&mut settings.start_with_windows, "Start with Windows");
                }
                save = ui.button("Save").clicked();
                ui.label(
                    RichText::new(format!("Version {APP_VERSION}"))
                        .small()
                        .weak(),
                );
            });
        if !open || (save && settings.save()) {
            self.settings = None;
        }
    }

    fn sessions_list(&self, ui: &mut Ui, sort: &mut IdleSort) -> Option<String> {
        let now = SystemTime::now();
        let (active, mut idle): (Vec<_>, Vec<_>) =
            self.visible_sessions().partition(|s| s.is_active(now));
        if *sort == IdleSort::Largest {
            idle.sort_by_key(|s| std::cmp::Reverse(s.latest_context));
        }
        let (recent, older): (Vec<_>, Vec<_>) = idle
            .into_iter()
            .partition(|s| s.idle_for(now).is_none_or(|age| age < OLDER_THAN));
        let mut opened = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            if !active.is_empty() {
                section_label(ui, "ACTIVE");
            }
            for session in active {
                if self.active_card(ui, session) {
                    opened = Some(session.id.clone());
                }
                ui.add_space(CARD_SPACING);
            }
            if recent.is_empty() && older.is_empty() {
                return;
            }
            ui.add_space(CARD_SPACING);
            idle_header(ui, sort);
            theme::glass(ui)
                .inner_margin(egui::Margin::symmetric(0, IDLE_PANEL_MARGIN))
                .show(ui, |ui| {
                    self.idle_rows(ui, &recent, now, &mut opened);
                    if older.is_empty() {
                        return;
                    }
                    if !recent.is_empty() {
                        ui.add(egui::Separator::default().spacing(0.0));
                    }
                    ui.spacing_mut().indent = FOLD_INDENT;
                    idle_frame().show(ui, |ui| {
                        egui::CollapsingHeader::new(
                            RichText::new(format!("Older than 7 days ({})", older.len()))
                                .small()
                                .weak(),
                        )
                        .id_salt("older")
                        .default_open(false)
                        .show_unindented(ui, |ui| {
                            ui.add(egui::Separator::default().spacing(0.0));
                            self.idle_rows(ui, &older, now, &mut opened);
                        });
                    });
                });
        });
        opened
    }

    fn idle_rows(
        &self,
        ui: &mut Ui,
        sessions: &[&Session],
        now: SystemTime,
        opened: &mut Option<String>,
    ) {
        for (i, session) in sessions.iter().enumerate() {
            if i > 0 {
                ui.add(egui::Separator::default().spacing(0.0));
            }
            if self.idle_row(ui, session, now) {
                *opened = Some(session.id.clone());
            }
        }
    }

    fn active_card(&self, ui: &mut Ui, session: &Session) -> bool {
        let fraction = session.usage_fraction();
        let level = theme::level_color(fraction);
        theme::glass(ui)
            .stroke((1.0, level.unwrap_or(ACCENT).gamma_multiply(0.7)))
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
                        theme::dot(ui, ACTIVE_DOT_RADIUS, LIVE);
                        name_label(
                            ui,
                            session,
                            RichText::new(session.name()).size(NAME_TEXT_SIZE).strong(),
                        );
                        if let Some(color) = level {
                            let hint = if fraction >= FULL_AT {
                                "auto-compact near"
                            } else {
                                "compact soon"
                            };
                            ui.label(
                                RichText::new(format!("{:.0}% · {hint}", fraction * 100.0))
                                    .small()
                                    .color(color),
                            );
                        }
                    },
                );
                let bar =
                    self.context_bar(ui, session, vec2(ui.available_width(), ACTIVE_BAR_HEIGHT));
                let (_, info) = split_row(
                    ui,
                    ROW_HEIGHT,
                    |ui| ui.label(RichText::new(summary(session)).small().weak()),
                    |ui| {
                        let info = self.info_button(ui, session);
                        ui.label(RichText::new(session.latest_label()).small().weak());
                        info
                    },
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
                        let info = self.info_button(ui, session);
                        let age = session
                            .idle_for(now)
                            .map(|idle| format!("{} ago", format_age(idle)))
                            .unwrap_or_default();
                        fixed_width_label(ui, IDLE_VALUE_WIDTH, RichText::new(age).small().weak());
                        fixed_width_label(ui, IDLE_VALUE_WIDTH, idle_new_context(session))
                            .on_hover_text(NEW_CONTEXT_HINT);
                        fixed_width_label(
                            ui,
                            IDLE_CONTEXT_WIDTH,
                            RichText::new(session.latest_short_label()).small(),
                        );
                        fixed_width_label(
                            ui,
                            IDLE_MODEL_WIDTH,
                            RichText::new(&session.model).small().weak(),
                        );
                        info
                    },
                    |ui| name_label(ui, session, RichText::new(session.name())),
                )
                .0
            })
            .inner
    }

    fn context_bar(&self, ui: &mut Ui, session: &Session, size: egui::Vec2) -> bool {
        let response = theme::gradient_bar(ui, session.usage_fraction(), size);
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
        theme::scale_to_window(ui.ctx());
        theme::paint_backdrop(ui);
        self.hide_to_tray_on_close(ui.ctx());
        self.handle_keys(ui.ctx());
        self.apply_updates(ui.ctx());
        self.refresh_overheads();
        self.refresh_alerts();
        ui.ctx().request_repaint_after(ACTIVITY_REFRESH_INTERVAL);
        if self.mini.is_some() {
            let today = self.new_context_today();
            let back = egui::CentralPanel::default()
                .show(ui, |ui| mini::show(ui, &self.sessions, today))
                .inner;
            if back {
                self.toggle_mini(ui.ctx());
            }
            return;
        }
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("totals").show(ui, |ui| self.totals_bar(ui));
        let mut sort = self.idle_sort;
        let opened = egui::CentralPanel::default()
            .show(ui, |ui| self.sessions_list(ui, &mut sort))
            .inner;
        self.idle_sort = sort;
        if opened.is_some() {
            self.details_for = opened;
        }
        self.details_window(ui.ctx());
        self.settings_window(ui.ctx());
    }
}

fn mode_button(ui: &mut Ui) {
    let dark = ui.visuals().dark_mode;
    let (icon, hint) = if dark {
        ("☀", "Switch to light mode")
    } else {
        ("🌙", "Switch to dark mode")
    };
    let clicked = ui
        .add(egui::Button::new(RichText::new(icon).size(ICON_SIZE)).frame(false))
        .on_hover_text(hint)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked();
    if clicked {
        ui.ctx().set_theme(if dark {
            egui::Theme::Light
        } else {
            egui::Theme::Dark
        });
        let _ = config::save_light_mode(dark);
    }
}

fn idle_new_context(session: &Session) -> RichText {
    if session.new_context == 0 {
        RichText::new("—").weak()
    } else {
        RichText::new(session.new_context_label()).color(ACCENT)
    }
}

fn search_box(ui: &mut Ui, filter: &mut String) {
    let search_id = egui::Id::unique(SEARCH_ID);
    let focused = ui.memory(|memory| memory.has_focus(search_id));
    theme::search_field(ui, focused).show(ui, |ui| {
        ui.add(
            egui::TextEdit::singleline(filter)
                .id(search_id)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .align(egui::Align2::LEFT_CENTER)
                .min_size(vec2(0.0, SEARCH_HEIGHT))
                .hint_text("Search  Ctrl+F")
                .desired_width(SEARCH_WIDTH),
        );
        theme::icon(ui, theme::Icon::Search, SEARCH_ICON_SIZE);
    });
}

fn title_area(ui: &mut Ui, machine: &str, live: bool) {
    theme::logo(ui, LOGO_SIZE);
    ui.heading(RichText::new(APP_NAME).strong())
        .on_hover_text(machine);
    let live_width = if live { LIVE_PILL_WIDTH } else { 0.0 };
    let machine_width = ui
        .painter()
        .layout_no_wrap(
            machine.to_owned(),
            egui::TextStyle::Small.resolve(ui.style()),
            ui.visuals().text_color(),
        )
        .size()
        .x
        + PILL_PADDING;
    if ui.available_width() >= machine_width + live_width {
        theme::pill(ui).show(ui, |ui| ui.label(RichText::new(machine).small()));
    }
    if live && ui.available_width() >= LIVE_PILL_WIDTH {
        theme::pill(ui)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = DOT_GAP;
                theme::dot(ui, LIVE_DOT_RADIUS, LIVE);
                ui.label(RichText::new("Live").small().color(LIVE));
            })
            .response
            .on_hover_text("Watching session files; updates within ~2s of each request");
    }
}

fn glyph_button(ui: &mut Ui, glyph: &str, hint: &str) -> bool {
    ui.add(egui::Button::new(RichText::new(glyph).size(ICON_SIZE)).frame(false))
        .on_hover_text(hint)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn icon_button(ui: &mut Ui, icon: theme::Icon, hint: &str) -> bool {
    theme::icon_button(ui, icon, ICON_SIZE)
        .on_hover_text(hint)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn sync_label(ui: &mut Ui, status: &SyncStatus) {
    let (text, hover) = sync_text(status, local_offset());
    let label = theme::pill(ui)
        .show(ui, |ui| ui.label(RichText::new(text).small()))
        .inner;
    if let Some(hover) = hover {
        label.on_hover_text(hover);
    }
}

fn sync_text(status: &SyncStatus, utc_offset: i64) -> (String, Option<String>) {
    match status {
        SyncStatus::Off => ("Sync off".to_owned(), None),
        SyncStatus::Synced { at, rejected } => {
            let text = format!("Synced {}", clock(*at, utc_offset));
            if rejected.is_empty() {
                return (text, None);
            }
            let reasons: Vec<String> = rejected
                .iter()
                .map(|r| format!("{}: {}", r.session_id, r.reason))
                .collect();
            (
                format!("{text} · {} rejected", rejected.len()),
                Some(reasons.join(
                    "
",
                )),
            )
        }
        SyncStatus::Failed(reason) => {
            let short: String = reason.chars().take(SYNC_REASON_CHARS).collect();
            (format!("Sync failed: {short}"), Some(reason.clone()))
        }
    }
}

fn clock(at: SystemTime, utc_offset: i64) -> String {
    let utc = at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let seconds = (i64::try_from(utc).unwrap_or_default() + utc_offset).rem_euclid(86_400);
    format!("{:02}:{:02}", seconds / 3_600, seconds % 3_600 / 60)
}

fn local_offset() -> i64 {
    chrono::Local::now().offset().local_minus_utc().into()
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

fn idle_header(ui: &mut Ui, sort: &mut IdleSort) {
    idle_frame().show(ui, |ui| {
        split_row(
            ui,
            ROW_HEIGHT,
            |ui| {
                ui.selectable_value(sort, IdleSort::Largest, RichText::new("Largest").small());
                ui.selectable_value(sort, IdleSort::Recent, RichText::new("Recent").small());
            },
            |ui| ui.label(RichText::new("IDLE").small().weak()),
        );
    });
    ui.add_space(SECTION_SPACING);
}

fn section_label(ui: &mut Ui, text: &str) {
    idle_frame().show(ui, |ui| ui.label(RichText::new(text).small().weak()));
    ui.add_space(SECTION_SPACING);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::Rejection;

    const HOUR: i64 = 3_600;

    fn at() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_793_324_739)
    }

    fn rejection(session_id: &str, reason: &str) -> Rejection {
        Rejection {
            session_id: session_id.into(),
            reason: reason.into(),
        }
    }

    #[test]
    fn clock_shows_local_time() {
        assert_eq!(clock(at(), 0), "01:45");
        assert_eq!(clock(at(), 7 * HOUR), "08:45");
        assert_eq!(clock(at(), -5 * HOUR), "20:45");
        assert_eq!(clock(at(), 5 * HOUR + 30 * 60), "07:15");
    }

    #[test]
    fn sync_text_counts_rejected_sessions() {
        let synced = |rejected| SyncStatus::Synced { at: at(), rejected };
        assert_eq!(
            sync_text(&synced(Vec::new()), 7 * HOUR),
            ("Synced 08:45".to_owned(), None)
        );
        let rejected = vec![
            rejection("s1", "session belongs to another user"),
            rejection("s2", "numbers must be >= 0"),
        ];
        assert_eq!(
            sync_text(&synced(rejected), 7 * HOUR),
            (
                "Synced 08:45 · 2 rejected".to_owned(),
                Some(
                    "s1: session belongs to another user
s2: numbers must be >= 0"
                        .to_owned()
                )
            )
        );
        assert_eq!(
            sync_text(&SyncStatus::Off, 0),
            ("Sync off".to_owned(), None)
        );
        assert_eq!(
            sync_text(&SyncStatus::Failed("HTTP 500: boom".into()), 0),
            (
                "Sync failed: HTTP 500: boom".to_owned(),
                Some("HTTP 500: boom".to_owned())
            )
        );
    }
}
