use crate::platform::{self, APP_NAME};
use crate::session::{Session, format_tokens};
use crate::tray::{self, Tray};
use crate::watcher::{self, Update};
use eframe::egui;
use std::cmp::Ordering;
use std::sync::mpsc::Receiver;

const LIVE_COLOR: egui::Color32 = egui::Color32::from_rgb(60, 180, 90);
const CONTEXT_BAR_WIDTH: f32 = 240.0;

#[derive(PartialEq, Eq, Clone, Copy)]
enum Column {
    Session,
    Model,
    StartContext,
    LatestContext,
    NewContext,
    Compactions,
}

const SORTABLE_COLUMNS: [(Column, &str); 6] = [
    (Column::Session, "Session"),
    (Column::Model, "Model"),
    (Column::StartContext, "Start context"),
    (Column::LatestContext, "Latest context"),
    (Column::NewContext, "New context"),
    (Column::Compactions, "Compacted"),
];

impl Column {
    fn compare(self, a: &Session, b: &Session) -> Ordering {
        match self {
            Self::Session => a.name().cmp(b.name()),
            Self::Model => a.model.cmp(&b.model),
            Self::StartContext => a.start_context.cmp(&b.start_context),
            Self::LatestContext => a.latest_context.cmp(&b.latest_context),
            Self::NewContext => a.new_context.cmp(&b.new_context),
            Self::Compactions => a.compactions.cmp(&b.compactions),
        }
    }
}

pub struct App {
    updates: Receiver<Update>,
    indexing: Option<(usize, usize)>,
    machine: String,
    sessions: Vec<Session>,
    filter: String,
    sort: Option<(Column, bool)>,
    tray: Option<Tray>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            updates: watcher::spawn(cc.egui_ctx.clone()),
            indexing: Some((0, 0)),
            machine: platform::machine_name(),
            sessions: Vec::new(),
            filter: String::new(),
            sort: None,
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
        if let Some(sessions) = latest {
            self.sessions = sessions;
            self.indexing = None;
            self.sort_sessions();
        }
    }

    fn sort_sessions(&mut self) {
        match self.sort {
            None => self.sessions.sort_by(|a, b| b.latest_at.cmp(&a.latest_at)),
            Some((column, descending)) => self.sessions.sort_by(|a, b| {
                let order = column.compare(a, b);
                if descending { order.reverse() } else { order }
            }),
        }
    }

    fn toggle_sort(&mut self, column: Column) {
        let descending = match self.sort {
            Some((current, descending)) if current == column => !descending,
            _ => true,
        };
        self.sort = Some((column, descending));
        self.sort_sessions();
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(APP_NAME);
            ui.separator();
            ui.label("Search:");
            ui.text_edit_singleline(&mut self.filter);
            if self.indexing.is_none() {
                ui.colored_label(LIVE_COLOR, "● Live")
                    .on_hover_text("Watching session files; updates within ~2s of each request");
            }
        });
        if let Some((done, total)) = self.indexing {
            let fraction = if total == 0 {
                0.0
            } else {
                done as f32 / total as f32
            };
            ui.add(
                egui::ProgressBar::new(fraction)
                    .text(format!("Indexing sessions… {done}/{total} files")),
            );
        }
    }

    fn sessions_table(&self, ui: &mut egui::Ui) -> Option<Column> {
        let query = self.filter.to_lowercase();
        let mut clicked = None;
        egui::ScrollArea::both().show(ui, |ui| {
            egui::Grid::new("sessions")
                .striped(true)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.strong("Machine");
                    for (column, title) in SORTABLE_COLUMNS {
                        if ui
                            .button(egui::RichText::new(self.header_text(column, title)).strong())
                            .clicked()
                        {
                            clicked = Some(column);
                        }
                    }
                    ui.end_row();
                    for session in self.sessions.iter().filter(|s| matches_query(s, &query)) {
                        self.session_row(ui, session);
                        ui.end_row();
                    }
                });
        });
        clicked
    }

    fn header_text(&self, column: Column, title: &str) -> String {
        match self.sort {
            Some((current, true)) if current == column => format!("{title} ⬇"),
            Some((current, false)) if current == column => format!("{title} ⬆"),
            _ => title.to_owned(),
        }
    }

    fn session_row(&self, ui: &mut egui::Ui, session: &Session) {
        ui.label(&self.machine);
        ui.label(session.name())
            .on_hover_text(format!("{}\n{}", session.project, session.id));
        ui.label(&session.model);
        ui.label(format_tokens(session.start_context)).on_hover_text(
            "Context when tracking started: at app start, or the first request of a session started later",
        );
        ui.add(
            egui::ProgressBar::new(session.usage_fraction())
                .desired_width(CONTEXT_BAR_WIDTH)
                .text(session.latest_label()),
        )
        .on_hover_text("Prompt size of the latest request (input + cache write + cache read)");
        ui.label(session.new_context_label()).on_hover_text(
            "Context added since start. Keeps counting through /compact: a compaction adds nothing, growth after it adds on top.",
        );
        let compactions = match session.compactions {
            0 => "—".to_owned(),
            n => format!("{n}×"),
        };
        ui.label(compactions).on_hover_text(
            "Times this session was compacted (manual /compact or auto-compact) since start",
        );
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.hide_to_tray_on_close(ui.ctx());
        self.apply_updates();
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        let clicked = egui::CentralPanel::default()
            .show(ui, |ui| self.sessions_table(ui))
            .inner;
        if let Some(column) = clicked {
            self.toggle_sort(column);
        }
    }
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
