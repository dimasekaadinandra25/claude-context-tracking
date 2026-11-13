use crate::alerts::{self, FullWarnings, LimitWarnings};
use crate::config;
use crate::details;
use crate::mini;
use crate::models;
use crate::overhead::{self, Overhead};
use crate::plan_limits::{self, FIVE_HOUR_LABEL, PlanLimits, SEVEN_DAY_LABEL, Window};
use crate::platform::{self, APP_NAME, APP_VERSION};
use crate::session::{Session, format_age, format_tokens, local_offset, total_new_context};
use crate::settings::Settings;
use crate::sync::SyncStatus;
use crate::theme::{self, ACCENT, FULL_AT, LIVE, RED};
use crate::tray::{self, Tray};
use crate::watcher::{self, Command, Update};
use eframe::egui::collapsing_header::{CollapsingState, paint_default_icon};
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
const IDLE_NAME_MIN_WIDTH: f32 = 140.0;
const MAX_CONTENT_WIDTH: f32 = 760.0;
const SUMMARY_SEPARATOR: &str = "  ·  ";
const IDLE_INSET: i8 = 14;
const IDLE_PANEL_MARGIN: i8 = 6;
const SECTION_SPACING: f32 = 4.0;
const CARD_MARGIN: i8 = 11;
const CARD_SPACING: f32 = 8.0;
const SEARCH_WIDTH: f32 = 100.0;
const TOP_ROW_HEIGHT: f32 = 28.0;
const BANNER_ROW_HEIGHT: f32 = 24.0;
const FOLD_ICON_WIDTH: f32 = 10.0;
const LIVE_PILL_WIDTH: f32 = 64.0;
const PILL_PADDING: f32 = 28.0;
const SEARCH_ICON_SIZE: f32 = 13.0;
const SEARCH_HEIGHT: f32 = 20.0;
const SEARCH_ID: &str = "search";
const OLDER_THAN: Duration = Duration::from_secs(7 * 86_400);
const SYNC_REASON_CHARS: usize = 60;
const EMPTY_STATE_SPACE: f32 = 48.0;
const ACTIVITY_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const PLAN_LIMITS_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const PLAN_LIMIT_BAR_HEIGHT: f32 = 6.0;
const PLAN_LIMIT_MIN_BAR: f32 = 24.0;
const PLAN_LIMIT_ROW_HEIGHT: f32 = 18.0;
const OVERHEAD_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const NEW_CONTEXT_HINT: &str = "Context added since start. Keeps counting through /compact: a compaction adds nothing, growth after it adds on top.";
const MODELS_HINT: &str = "Your usage per model over the last 7 days";
const TOTAL_HINT: &str = "New context of the listed sessions since the app started; follows the search filter. The tray shows today's total instead.";
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
    models_open: bool,
    overheads: HashMap<String, Overhead>,
    overheads_refreshed: Option<Instant>,
    plan_limits: Option<PlanLimits>,
    plan_limits_refreshed: Option<Instant>,
    limit_warnings: LimitWarnings,
    tray: Option<Tray>,
    sync: SyncStatus,
    settings: Option<Settings>,
    idle_sort: IdleSort,
    warnings: FullWarnings,
    tray_tooltip: String,
    user_zoom: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum IdleSort {
    Recent,
    Largest,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, hidden: bool) -> Self {
        theme::apply(&cc.egui_ctx);
        let config = config::load();
        if !config.is_enabled() && config::claim_key_reminder() {
            alerts::notify_key_reminder();
        }
        cc.egui_ctx.set_theme(if config.light_mode {
            egui::Theme::Light
        } else {
            egui::Theme::Dark
        });
        let (tx, updates) = channel();
        let commands = watcher::spawn(cc.egui_ctx.clone(), tx.clone());
        let tray = tray::spawn(cc.egui_ctx.clone(), tx, commands.clone());
        if hidden && tray.is_some() {
            tray::set_window_visible(&cc.egui_ctx, false);
        }
        Self {
            updates,
            commands,
            mini: None,
            indexing: Some((0, 0)),
            machine: platform::machine_name(),
            sessions: Vec::new(),
            filter: String::new(),
            details_for: None,
            models_open: false,
            overheads: HashMap::new(),
            overheads_refreshed: None,
            plan_limits: None,
            plan_limits_refreshed: None,
            limit_warnings: LimitWarnings::default(),
            tray,
            sync: if config.is_enabled() {
                SyncStatus::Waiting
            } else {
                SyncStatus::Off
            },
            settings: None,
            idle_sort: IdleSort::Recent,
            warnings: FullWarnings::default(),
            tray_tooltip: String::new(),
            user_zoom: 1.0,
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
                Update::Sync(status) => self.sync = status,
                Update::ToggleMini => toggle_mini = !toggle_mini,
            }
        }
        if toggle_mini {
            self.toggle_mini(ctx);
        }
        if let Some(mut sessions) = latest {
            sessions.sort_by(|a, b| b.last_activity().cmp(a.last_activity()));
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
        let sync = sync_text(&self.sync, local_offset()).0;
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
                let settings = icon_button(ui, theme::Icon::Gear, "Settings");
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
        if let Some(problem) = key_problem(&self.sync)
            && api_key_banner(ui, problem)
        {
            self.settings = Some(Settings::load());
        }
        if let Some(limits) = &self.plan_limits {
            plan_limits_row(ui, limits);
        }
        ui.add_space(6.0);
    }

    fn refresh_plan_limits(&mut self) {
        if self
            .plan_limits_refreshed
            .is_none_or(|at| at.elapsed() >= PLAN_LIMITS_REFRESH_INTERVAL)
        {
            self.plan_limits = plan_limits::load();
            self.plan_limits_refreshed = Some(Instant::now());
            if let Some(limits) = &self.plan_limits {
                let now = plan_limits::unix_now();
                for (label, window) in self.limit_warnings.newly_crossed(&limits.live(now)) {
                    let left =
                        format_age(Duration::from_secs(window.resets_at.saturating_sub(now)));
                    alerts::notify_limit(&limit_name(&label), window, &left);
                }
            }
        }
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

    fn totals_bar(&mut self, ui: &mut Ui) {
        let now = SystemTime::now();
        let (count, active) = self.visible_sessions().fold((0, 0), |(count, active), s| {
            (count + 1, active + usize::from(s.is_active(now)))
        });
        let total = RichText::new(format!(
            "+{}",
            format_tokens(total_new_context(self.visible_sessions()))
        ))
        .strong()
        .color(ACCENT);
        let label = RichText::new("New context since launch").weak();
        let counts = RichText::new(format!("{active} active  ·  {count} sessions")).weak();
        let sync_now = !matches!(self.sync, SyncStatus::Off | SyncStatus::Syncing);
        let gap = ui.spacing().item_spacing.x;
        let base = text_width(ui, total.clone()) + if sync_now { ICON_SIZE + gap } else { 0.0 };
        let sync = RichText::new(sync_text(&self.sync, local_offset()).0).small();
        let shown = fitting(
            &[
                text_width(ui, sync) + PILL_PADDING,
                text_width(ui, label.clone()),
                text_width(ui, counts.clone()),
            ],
            gap,
            ui.available_width() - base,
        );
        ui.add_space(4.0);
        split_row(
            ui,
            ROW_HEIGHT,
            |ui| {
                if ui
                    .link(RichText::new("Models").small())
                    .on_hover_text(MODELS_HINT)
                    .clicked()
                {
                    self.models_open = !self.models_open;
                }
                if shown > 2 {
                    ui.label(counts);
                }
                if shown > 0 && sync_label(ui, &self.sync) {
                    self.settings = Some(Settings::load());
                }
                if sync_now && icon_button(ui, theme::Icon::Refresh, "Sync now") {
                    self.sync = SyncStatus::Syncing;
                    let _ = self.commands.send(Command::SyncNow);
                }
            },
            |ui| {
                if shown > 1 {
                    ui.label(label).on_hover_text(TOTAL_HINT);
                }
                ui.label(total).on_hover_text(TOTAL_HINT);
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
        theme::dialog(ctx, "Context usage", &mut open, |ui| {
            details::show(ui, session, self.overhead(session))
        });
        if !open {
            self.details_for = None;
        }
    }

    fn models_window(&mut self, ctx: &egui::Context) {
        if !self.models_open {
            return;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let rows = models::totals(&self.sessions, models::since(now));
        let mut open = true;
        theme::dialog(ctx, models::TITLE, &mut open, |ui| models::show(ui, &rows));
        self.models_open = open;
    }

    fn sessions_list(&self, ui: &mut Ui, sort: &mut IdleSort) -> Option<String> {
        if self.sessions.is_empty() {
            if self.indexing.is_none() {
                let projects = platform::projects_dir();
                empty_state(
                    ui,
                    "No Claude Code sessions found",
                    &projects.display().to_string(),
                );
            }
            return None;
        }
        let now = SystemTime::now();
        let (active, mut idle): (Vec<_>, Vec<_>) =
            self.visible_sessions().partition(|s| s.is_active(now));
        if active.is_empty() && idle.is_empty() {
            empty_state(ui, "No sessions match your search", "Esc clears the search");
            return None;
        }
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
                    let mut fold = CollapsingState::load_with_default_open(
                        ui.ctx(),
                        ui.make_persistent_id("older"),
                        false,
                    );
                    idle_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().indent = FOLD_ICON_WIDTH;
                            fold.show_toggle_button(ui, paint_default_icon);
                            let label = ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "Older than 7 days ({})",
                                            older.len()
                                        ))
                                        .small()
                                        .weak(),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if label.clicked() {
                                fold.toggle(ui);
                            }
                        });
                    });
                    fold.show_body_unindented(ui, |ui| {
                        ui.add(egui::Separator::default().spacing(0.0));
                        self.idle_rows(ui, &older, now, &mut opened);
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
                    |ui| {
                        theme::dot(ui, ACTIVE_DOT_RADIUS, LIVE);
                        name_label(
                            ui,
                            session,
                            RichText::new(session.name()).size(NAME_TEXT_SIZE).strong(),
                        );
                    },
                );
                let bar =
                    self.context_bar(ui, session, vec2(ui.available_width(), ACTIVE_BAR_HEIGHT));
                let latest = RichText::new(session.latest_label()).small().weak();
                let reserve =
                    ROW_HEIGHT + text_width(ui, latest.clone()) + 2.0 * ui.spacing().item_spacing.x;
                let (_, info) = split_row(
                    ui,
                    ROW_HEIGHT,
                    |ui| summary_label(ui, session, reserve),
                    |ui| {
                        let info = self.info_button(ui, session);
                        ui.label(latest);
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
                let columns = idle_columns(ui.available_width(), ui.spacing().item_spacing.x);
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
                        if columns > 0 {
                            fixed_width_label(
                                ui,
                                IDLE_CONTEXT_WIDTH,
                                RichText::new(session.latest_short_label()).small(),
                            );
                        }
                        if columns > 1 {
                            fixed_width_label(
                                ui,
                                IDLE_MODEL_WIDTH,
                                RichText::new(&session.model).small().weak(),
                            );
                        }
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
        let (rect, response) = ui.allocate_exact_size(
            vec2(INFO_RADIUS * 2.0 + 2.0, ROW_HEIGHT),
            egui::Sense::click(),
        );
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
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.hide_to_tray_on_close(ctx);
        self.apply_updates(ctx);
        self.refresh_alerts();
        self.refresh_plan_limits();
        ctx.request_repaint_after(ACTIVITY_REFRESH_INTERVAL);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        theme::scale_to_window(ui.ctx(), &mut self.user_zoom);
        theme::paint_backdrop(ui);
        self.refresh_overheads();
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
        self.handle_keys(ui.ctx());
        egui::Panel::top("top").show(ui, |ui| centered(ui, |ui| self.top_bar(ui)));
        egui::Panel::bottom("totals").show(ui, |ui| centered(ui, |ui| self.totals_bar(ui)));
        let mut sort = self.idle_sort;
        let opened = egui::CentralPanel::default()
            .show(ui, |ui| {
                centered(ui, |ui| self.sessions_list(ui, &mut sort))
            })
            .inner;
        self.idle_sort = sort;
        if opened.is_some() {
            self.details_for = opened;
        }
        self.details_window(ui.ctx());
        self.models_window(ui.ctx());
        if let Some(settings) = &mut self.settings
            && settings.show(ui.ctx())
        {
            self.settings = None;
        }
    }
}

fn limit_name(label: &str) -> String {
    match label {
        FIVE_HOUR_LABEL => "Your 5-hour Claude limit".to_owned(),
        SEVEN_DAY_LABEL => "Your weekly Claude limit".to_owned(),
        model => format!("Your weekly {model} limit"),
    }
}

fn plan_limits_row(ui: &mut Ui, limits: &PlanLimits) {
    let now = plan_limits::unix_now();
    let windows = limits.live(now);
    if windows.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.columns(windows.len(), |columns| {
        for (column, (label, window)) in columns.iter_mut().zip(windows) {
            plan_limit_meter(column, &label, window, now);
        }
    });
}

fn plan_limit_meter(ui: &mut Ui, label: &str, window: Window, now: u64) {
    let fraction = (window.used_percentage / 100.0) as f32;
    let left = format_age(Duration::from_secs(window.resets_at.saturating_sub(now)));
    let percent = format!("{:.0}%", window.used_percentage);
    let name = RichText::new(label).small().weak();
    let mut value = RichText::new(&percent).small().strong();
    if let Some(color) = theme::level_color(fraction) {
        value = value.color(color);
    }
    let remaining = RichText::new(&left).small().weak();
    let gap = ui.spacing().item_spacing.x;
    let fixed = text_width(ui, name.clone()) + text_width(ui, value.clone()) + gap * 2.0;
    let extra = text_width(ui, remaining.clone()) + gap;
    let available = ui.available_width();
    let show_remaining = available - fixed - extra >= PLAN_LIMIT_MIN_BAR;
    let reserved = if show_remaining { fixed + extra } else { fixed };
    let bar = (available - reserved).max(PLAN_LIMIT_MIN_BAR);
    ui.allocate_ui_with_layout(
        vec2(available, PLAN_LIMIT_ROW_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(name);
            let mut hover = format!("{percent} used, resets in {left}");
            if label != FIVE_HOUR_LABEL
                && let Some(budget) = window.weekly_budget(now)
            {
                hover = format!(
                    "{hover}
{budget}"
                );
            }
            theme::gradient_bar(ui, fraction, vec2(bar, PLAN_LIMIT_BAR_HEIGHT))
                .on_hover_text(hover);
            ui.label(value);
            if show_remaining {
                ui.label(remaining);
            }
        },
    );
}

fn empty_state(ui: &mut Ui, title: &str, detail: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(EMPTY_STATE_SPACE);
        ui.label(RichText::new(title).strong());
        ui.label(RichText::new(detail).small().weak());
    });
}

fn mode_button(ui: &mut Ui) {
    let dark = ui.visuals().dark_mode;
    let (icon, hint) = if dark {
        (theme::Icon::Sun, "Switch to light mode")
    } else {
        (theme::Icon::Moon, "Switch to dark mode")
    };
    if icon_button(ui, icon, hint) {
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

type KeyProblem = (egui::Color32, &'static str, &'static str);

fn key_problem(status: &SyncStatus) -> Option<KeyProblem> {
    match status {
        SyncStatus::Off => Some((
            theme::AMBER,
            "Your usage isn't sent to the team dashboard yet",
            "Add API key",
        )),
        SyncStatus::KeyRejected => Some((
            RED,
            "API key rejected. Check it in Settings.",
            "Open Settings",
        )),
        _ => None,
    }
}

fn api_key_banner(ui: &mut Ui, (color, message, button): KeyProblem) -> bool {
    ui.add_space(4.0);
    theme::glass(ui)
        .stroke((1.0, color.gamma_multiply(0.8)))
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            split_row(
                ui,
                BANNER_ROW_HEIGHT,
                |ui| {
                    ui.add(
                        egui::Button::new(RichText::new(button).color(egui::Color32::WHITE))
                            .fill(ACCENT),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                },
                |ui| {
                    theme::dot(ui, LIVE_DOT_RADIUS, color);
                    ui.add(
                        egui::Label::new(
                            RichText::new(message).color(ui.visuals().strong_text_color()),
                        )
                        .truncate(),
                    );
                },
            )
            .0
        })
        .inner
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
    let title = RichText::new(APP_NAME).heading().strong();
    let gap = ui.spacing().item_spacing.x;
    let machine_width = text_width(ui, RichText::new(machine).small()) + PILL_PADDING;
    let (show_title, show_machine) = title_parts(
        ui.available_width(),
        gap,
        text_width(ui, title.clone()),
        machine_width,
        live,
    );
    if show_title {
        ui.label(title).on_hover_text(machine);
    }
    if show_machine {
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

fn icon_button(ui: &mut Ui, icon: theme::Icon, hint: &str) -> bool {
    theme::icon_button(ui, icon, ICON_SIZE)
        .on_hover_text(hint)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn sync_label(ui: &mut Ui, status: &SyncStatus) -> bool {
    let (text, hover) = sync_text(status, local_offset());
    let mut text = RichText::new(text).small();
    if *status == SyncStatus::KeyRejected {
        text = text.color(RED);
    }
    let update = matches!(
        status,
        SyncStatus::Synced {
            update: Some(_),
            ..
        }
    );
    if update {
        text = text.color(theme::AMBER);
    }
    let label = theme::pill(ui).show(ui, |ui| ui.label(text)).inner;
    if update {
        let label = label
            .interact(egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let label = match hover {
            Some(hover) => label.on_hover_text(hover),
            None => label,
        };
        if label.clicked() {
            crate::update::start(ui.ctx().clone());
        }
        return false;
    }
    if key_problem(status).is_some() {
        return label
            .interact(egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Open Settings")
            .clicked();
    }
    if let Some(hover) = hover {
        label.on_hover_text(hover);
    }
    false
}

fn sync_text(status: &SyncStatus, utc_offset: i64) -> (String, Option<String>) {
    match status {
        SyncStatus::Off => ("Sync off".to_owned(), None),
        SyncStatus::Waiting => ("Waiting to sync".to_owned(), None),
        SyncStatus::Syncing => ("Syncing…".to_owned(), None),
        SyncStatus::KeyRejected => ("Key rejected".to_owned(), None),
        SyncStatus::Synced {
            at,
            rejected,
            update,
        } => {
            let mut text = format!("Synced {}", clock(*at, utc_offset));
            let mut hover: Vec<String> = rejected
                .iter()
                .map(|r| format!("{}: {}", r.session_id, r.reason))
                .collect();
            if !rejected.is_empty() {
                text = format!("{text} · {} rejected", rejected.len());
            }
            if let Some(version) = update {
                text = format!("{text} · v{version} available");
                hover.push(format!(
                    "Your team already runs v{version}, this app is v{APP_VERSION}. Click to install the update."
                ));
            }
            (text, (!hover.is_empty()).then(|| hover.join("\n")))
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

fn summary_label(ui: &mut Ui, session: &Session, reserve: f32) {
    let parts = summary_parts(session);
    let widths: Vec<f32> = parts
        .iter()
        .map(|part| text_width(ui, RichText::new(part).small()))
        .collect();
    let width = (ui.available_width() - reserve).max(0.0);
    let separator = text_width(ui, RichText::new(SUMMARY_SEPARATOR).small());
    let shown = 1 + fitting(&widths[1..], separator, width - widths[0]);
    let text = RichText::new(parts[..shown].join(SUMMARY_SEPARATOR))
        .small()
        .weak();
    ui.allocate_ui(vec2(width, ROW_HEIGHT), |ui| {
        ui.add(egui::Label::new(text).truncate())
    });
}

fn summary_parts(session: &Session) -> Vec<String> {
    let mut parts = vec![
        session.model.clone(),
        format!("started at {}", format_tokens(session.start_context)),
    ];
    if session.compactions > 0 {
        parts.push(format!("compacted {}×", session.compactions));
    }
    parts
}

fn title_parts(
    available: f32,
    gap: f32,
    title_width: f32,
    machine_width: f32,
    live: bool,
) -> (bool, bool) {
    let room = available - if live { LIVE_PILL_WIDTH + gap } else { 0.0 };
    let shown = fitting(&[title_width, machine_width], gap, room);
    (shown >= 1, shown >= 2)
}

fn fitting(widths: &[f32], gap: f32, available: f32) -> usize {
    let mut used = 0.0;
    widths
        .iter()
        .take_while(|&&width| {
            used += gap + width;
            used <= available
        })
        .count()
}

fn idle_columns(width: f32, gap: f32) -> usize {
    let base = ROW_HEIGHT + 2.0 * IDLE_VALUE_WIDTH + 3.0 * gap + IDLE_NAME_MIN_WIDTH;
    fitting(&[IDLE_CONTEXT_WIDTH, IDLE_MODEL_WIDTH], gap, width - base)
}

fn text_width(ui: &Ui, text: impl Into<egui::WidgetText>) -> f32 {
    text.into()
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        )
        .size()
        .x
}

fn centered<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let available = ui.available_rect_before_wrap();
    let half = available.width().min(MAX_CONTENT_WIDTH) / 2.0;
    let x = available.center().x;
    let rect = egui::Rect::from_x_y_ranges(x - half..=x + half, available.y_range());
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), add)
        .inner
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
    fn key_banner_amber_when_off_and_red_when_rejected() {
        assert_eq!(key_problem(&SyncStatus::Off).unwrap().0, theme::AMBER);
        let (color, message, button) = key_problem(&SyncStatus::KeyRejected).unwrap();
        assert_eq!(color, RED);
        assert_eq!(message, "API key rejected. Check it in Settings.");
        assert_eq!(button, "Open Settings");
        assert!(key_problem(&SyncStatus::Waiting).is_none());
        assert!(key_problem(&SyncStatus::Syncing).is_none());
        assert!(key_problem(&SyncStatus::Failed("HTTP 500".into())).is_none());
    }

    #[test]
    fn live_pill_outranks_title_and_machine() {
        assert_eq!(title_parts(400.0, 8.0, 150.0, 100.0, true), (true, true));
        assert_eq!(title_parts(250.0, 8.0, 150.0, 100.0, true), (true, false));
        assert_eq!(title_parts(150.0, 8.0, 150.0, 100.0, true), (false, false));
        assert_eq!(title_parts(170.0, 8.0, 150.0, 100.0, false), (true, false));
    }

    #[test]
    fn fitting_counts_leading_items_that_fit() {
        assert_eq!(fitting(&[10.0, 20.0, 30.0], 5.0, 100.0), 3);
        assert_eq!(fitting(&[10.0, 20.0, 30.0], 5.0, 70.0), 2);
        assert_eq!(fitting(&[10.0, 20.0, 30.0], 5.0, 14.0), 0);
        assert_eq!(fitting(&[10.0, 1.0], 5.0, -1.0), 0);
        assert_eq!(fitting(&[], 5.0, 100.0), 0);
    }

    #[test]
    fn idle_columns_hide_model_then_context_as_width_shrinks() {
        assert_eq!(idle_columns(514.0, 8.0), 2);
        assert_eq!(idle_columns(472.0, 8.0), 2);
        assert_eq!(idle_columns(434.0, 8.0), 1);
        assert_eq!(idle_columns(376.0, 8.0), 1);
        assert_eq!(idle_columns(375.0, 8.0), 0);
        assert_eq!(idle_columns(100.0, 8.0), 0);
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
        let synced = |rejected| SyncStatus::Synced {
            at: at(),
            rejected,
            update: None,
        };
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
            sync_text(
                &SyncStatus::Synced {
                    at: at(),
                    rejected: Vec::new(),
                    update: Some("9.0.0".into()),
                },
                7 * HOUR
            ),
            (
                "Synced 08:45 · v9.0.0 available".to_owned(),
                Some(format!(
                    "Your team already runs v9.0.0, this app is v{APP_VERSION}. Click to install the update."
                ))
            )
        );
        assert_eq!(
            sync_text(&SyncStatus::Off, 0),
            ("Sync off".to_owned(), None)
        );
        assert_eq!(sync_text(&SyncStatus::Waiting, 0).0, "Waiting to sync");
        assert_eq!(sync_text(&SyncStatus::Syncing, 0).0, "Syncing…");
        assert_eq!(sync_text(&SyncStatus::KeyRejected, 0).0, "Key rejected");
        assert_eq!(
            sync_text(&SyncStatus::Failed("HTTP 500: boom".into()), 0),
            (
                "Sync failed: HTTP 500: boom".to_owned(),
                Some("HTTP 500: boom".to_owned())
            )
        );
    }
}
