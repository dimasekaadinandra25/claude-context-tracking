#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const EXPORT_EVERY: Duration = Duration::from_secs(30 * 60);

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    timestamp: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "aiTitle")]
    ai_title: Option<String>,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: bool,
    message: Option<Msg>,
}

#[derive(Deserialize)]
struct Msg {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Usage {
    input_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
}

#[derive(Default, Clone)]
struct Session {
    id: String,
    project: String,
    title: String,
    /// Model of the latest main-thread request.
    model: String,
    /// Prompt size (input + cache write + cache read) of the latest main-thread request.
    context: u64,
    /// Timestamp of that request; also orders sessions by recency.
    context_ts: String,
    /// Context when tracking started: at app start for existing sessions, else the session's first request.
    start: u64,
    window: u64,
}

impl Session {
    fn name(&self) -> &str {
        if !self.title.is_empty() {
            return &self.title;
        }
        let folder = self.project.rsplit(['\\', '/']).next().unwrap_or_default();
        if folder.is_empty() { &self.id } else { folder }
    }

    fn context_frac(&self) -> f32 {
        self.context as f32 / self.window.max(1) as f32
    }

    fn context_label(&self) -> String {
        format!("{} / {} ({:.0}%)", kfmt(self.context), kfmt(self.window), self.context_frac() * 100.0)
    }

    /// Latest context minus start context (negative after /compact).
    fn diff(&self) -> i64 {
        self.context as i64 - self.start as i64
    }

    fn diff_label(&self) -> String {
        let d = self.diff();
        format!("{}{}", if d < 0 { "-" } else { "+" }, kfmt(d.unsigned_abs()))
    }
}

/// Opus/Sonnet 4.6+, all 5.x and Haiku 5.x have 1M windows; older models have 200K.
fn context_window(model: &str) -> u64 {
    let v: Vec<u32> = model.split('-').filter(|p| p.len() <= 2).filter_map(|p| p.parse().ok()).collect();
    match v[..] {
        [major, ..] if major >= 5 => 1_000_000,
        [4, minor, ..] if minor >= 6 => 1_000_000,
        _ => 200_000,
    }
}

fn kfmt(n: u64) -> String {
    match n {
        1_000_000.. => format!("{:.1}M", n as f64 / 1e6).replace(".0M", "M"),
        1_000.. => format!("{:.1}K", n as f64 / 1e3),
        _ => n.to_string(),
    }
}

/// Folds one JSONL line into the session map. `seen` dedupes by message id:
/// Claude Code writes one line per content block, each repeating the same usage.
fn ingest(sessions: &mut HashMap<String, Session>, seen: &mut HashSet<String>, raw: &str, fallback_id: &str) {
    let Ok(line) = serde_json::from_str::<Line>(raw) else { return };
    let id = line.session_id.clone().unwrap_or_else(|| fallback_id.to_string());
    let s = sessions.entry(id.clone()).or_insert_with(|| Session { id, ..Default::default() });

    if line.kind.as_deref() == Some("ai-title") {
        if let Some(t) = line.ai_title {
            s.title = t;
        }
    }
    if s.project.is_empty() {
        if let Some(cwd) = line.cwd {
            s.project = cwd;
        }
    }
    // Subagents run in their own context window, so only main-thread requests count.
    let Some(msg) = line.message.filter(|_| !line.is_sidechain) else { return };
    let (Some(mid), Some(u), Some(model)) = (msg.id, msg.usage, msg.model.filter(|m| m != "<synthetic>")) else { return };
    let ts = line.timestamp.unwrap_or_default();
    if !seen.insert(mid) || ts < s.context_ts {
        return;
    }
    let prompt = u.input_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens;
    if s.context_ts.is_empty() {
        s.start = prompt;
    }
    s.context = prompt;
    s.context_ts = ts;
    s.window = context_window(&model);
    s.model = model;
}

fn claude_dir() -> PathBuf {
    if let Ok(d) = std::env::var("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    PathBuf::from(home).join(".claude")
}

fn machine_name() -> String {
    // COMPUTERNAME on Windows; `hostname` exists on macOS and Linux (and Windows).
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| {
            let out = std::process::Command::new("hostname").output().ok()?;
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// %LOCALAPPDATA% on Windows, ~/Library/Application Support on macOS, ~/.local/share on Linux.
fn data_file() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    base.join("ClaudeUsageMonitor").join("usage.txt")
}

fn open_in_default_app(path: &Path) {
    let opener = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(path).spawn();
}

/// Tab-separated snapshot, most recently active session first.
fn to_tsv(machine: &str, sessions: &[Session]) -> String {
    let mut list: Vec<&Session> = sessions.iter().collect();
    list.sort_by(|a, b| b.context_ts.cmp(&a.context_ts));
    let mut out = String::from("machine\tsession\tmodel\tstart_context\tlatest_context\tnew_context\n");
    for s in list {
        out += &format!("{machine}\t{}\t{}\t{}\t{}\t{}\n", s.name(), s.model, kfmt(s.start), s.context_label(), s.diff_label());
    }
    out
}

fn write_data_file(text: &str) -> std::io::Result<()> {
    let path = data_file();
    std::fs::create_dir_all(path.parent().unwrap())?;
    // Write then rename, so anything reading the file never sees half a snapshot.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_jsonl(&p, out);
        } else if p.extension().is_some_and(|x| x == "jsonl") {
            out.push(p);
        }
    }
}

enum Progress {
    Step(usize, usize),
    Done(Vec<Session>),
}

/// Session files are append-only JSONL, so each file is read once and then only from its last offset.
// ponytail: in-memory index, full read on every launch; persist offsets + sessions if projects/ grows to GBs.
#[derive(Default)]
struct Index {
    sessions: HashMap<String, Session>,
    seen: HashSet<String>,
    offsets: HashMap<PathBuf, u64>,
}

impl Index {
    /// Returns true if any file had new complete lines.
    fn scan(&mut self, root: &Path, mut on_step: impl FnMut(usize, usize)) -> bool {
        let mut files = Vec::new();
        collect_jsonl(root, &mut files);
        let mut changed = false;
        for (i, f) in files.iter().enumerate() {
            changed |= self.read_new(f);
            on_step(i + 1, files.len());
        }
        changed
    }

    fn read_new(&mut self, path: &Path) -> bool {
        let off = self.offsets.get(path).copied().unwrap_or(0);
        if std::fs::metadata(path).map_or(true, |m| m.len() <= off) {
            return false;
        }
        let mut buf = Vec::new();
        let read = File::open(path).and_then(|mut f| {
            f.seek(SeekFrom::Start(off))?;
            f.read_to_end(&mut buf)
        });
        // A line still being written has no trailing newline yet; leave it for the next scan.
        let Some(end) = read.ok().and_then(|_| buf.iter().rposition(|&b| b == b'\n')).map(|p| p + 1) else {
            return false;
        };
        let fallback = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        for raw in String::from_utf8_lossy(&buf[..end]).lines() {
            ingest(&mut self.sessions, &mut self.seen, raw, &fallback);
        }
        self.offsets.insert(path.to_path_buf(), off + end as u64);
        true
    }

    /// Diff counts growth while the app is open, so existing sessions start from their current context.
    fn mark_start(&mut self) {
        for s in self.sessions.values_mut() {
            s.start = s.context;
        }
    }

    fn snapshot(&self) -> Vec<Session> {
        self.sessions.values().filter(|s| s.window > 0).cloned().collect()
    }
}

// ponytail: polls every 2s; switch to the `notify` crate if lower latency or idle CPU ever matters.
fn start_watching(ctx: egui::Context) -> Receiver<Progress> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let root = claude_dir().join("projects");
        let mut index = Index::default();
        index.scan(&root, |d, t| {
            let _ = tx.send(Progress::Step(d, t));
            ctx.request_repaint();
        });
        index.mark_start();
        let machine = machine_name();
        let (mut changed, mut last_export, mut last_written) = (true, None::<Instant>, String::new());
        loop {
            if changed {
                if tx.send(Progress::Done(index.snapshot())).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            if last_export.is_none_or(|t| t.elapsed() >= EXPORT_EVERY) {
                // Skip the write when nothing changed since the last one; a failed write is retried next interval.
                let tsv = to_tsv(&machine, &index.snapshot());
                if tsv != last_written && write_data_file(&tsv).is_ok() {
                    last_written = tsv;
                }
                last_export = Some(Instant::now());
            }
            std::thread::sleep(Duration::from_secs(2));
            changed = index.scan(&root, |_, _| {});
        }
    });
    rx
}

fn set_visible(ctx: &egui::Context, visible: bool) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(visible));
    if visible {
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
    ctx.request_repaint();
}

fn tray_image() -> Icon {
    let mut px = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32 {
        for x in 0..32 {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let alpha = if dx * dx + dy * dy <= 225.0 { 255 } else { 0 };
            px.extend_from_slice(&[217, 119, 87, alpha]);
        }
    }
    Icon::from_rgba(px, 32, 32).expect("valid 32x32 icon")
}

fn setup_tray(ctx: egui::Context) -> Option<TrayIcon> {
    let (show, open, quit) = (
        MenuItem::new("Show", true, None),
        MenuItem::new("Open data file", true, None),
        MenuItem::new("Quit", true, None),
    );
    let menu = Menu::new();
    menu.append_items(&[&show, &open, &quit]).ok()?;
    let (show_id, open_id, quit_id) = (show.id().clone(), open.id().clone(), quit.id().clone());
    let show_ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == show_id {
            set_visible(&show_ctx, true);
        } else if e.id == open_id {
            open_in_default_app(&data_file());
        } else if e.id == quit_id {
            std::process::exit(0);
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            set_visible(&ctx, true);
        }
    }));
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("Claude Usage Monitor")
        .with_icon(tray_image())
        .build()
        .ok()
}

#[cfg(not(target_os = "linux"))]
type Tray = TrayIcon;
/// On Linux the icon lives on its GTK thread; the app only needs to know it exists.
#[cfg(target_os = "linux")]
type Tray = ();

#[cfg(not(target_os = "linux"))]
fn start_tray(ctx: egui::Context) -> Option<Tray> {
    setup_tray(ctx)
}

/// tray-icon on Linux needs GTK initialised and its main loop running on the thread that owns the icon.
// ponytail: libappindicator has no click events, so on Linux the window is shown via the menu's "Show";
// Wayland also ignores hide requests, so closing there leaves the window up until Quit.
#[cfg(target_os = "linux")]
fn start_tray(ctx: egui::Context) -> Option<Tray> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        if gtk::init().is_err() {
            let _ = tx.send(false);
            return;
        }
        let tray = setup_tray(ctx);
        let _ = tx.send(tray.is_some());
        gtk::main();
        drop(tray);
    });
    rx.recv().unwrap_or(false).then_some(())
}

#[derive(PartialEq, Clone, Copy)]
enum Col { Session, Model, Start, Context, Diff }

struct App {
    rx: Receiver<Progress>,
    indexed: bool,
    progress: (usize, usize),
    machine: String,
    sessions: Vec<Session>,
    filter: String,
    /// None = most recently active first.
    sort: Option<Col>,
    desc: bool,
    /// Kept alive for the icon to stay in the tray; None if the tray couldn't be created.
    tray: Option<Tray>,
}

impl App {
    fn sort(&mut self) {
        let (c, desc) = (self.sort, self.desc);
        self.sessions.sort_by(|a, b| {
            let o = match c {
                None => return b.context_ts.cmp(&a.context_ts),
                Some(Col::Session) => a.name().cmp(b.name()),
                Some(Col::Model) => a.model.cmp(&b.model),
                Some(Col::Start) => a.start.cmp(&b.start),
                Some(Col::Context) => a.context.cmp(&b.context),
                Some(Col::Diff) => a.diff().cmp(&b.diff()),
            };
            if desc { o.reverse() } else { o }
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Close button hides to tray; Quit lives in the tray menu.
        if self.tray.is_some() && ui.ctx().input(|i| i.viewport().close_requested()) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            set_visible(ui.ctx(), false);
        }

        let mut fresh = None;
        for p in self.rx.try_iter() {
            match p {
                Progress::Step(d, t) => self.progress = (d, t),
                Progress::Done(list) => fresh = Some(list),
            }
        }
        if let Some(list) = fresh {
            self.sessions = list;
            self.indexed = true;
            self.sort();
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Claude Usage Monitor");
                ui.separator();
                ui.label("Search:");
                ui.text_edit_singleline(&mut self.filter);
                if self.indexed {
                    ui.colored_label(egui::Color32::from_rgb(60, 180, 90), "● Live")
                        .on_hover_text("Watching session files; updates within ~2s of each request");
                }
            });
            if !self.indexed {
                let (d, t) = self.progress;
                let frac = if t == 0 { 0.0 } else { d as f32 / t as f32 };
                ui.add(egui::ProgressBar::new(frac).text(format!("Indexing sessions… {d}/{t} files")));
            }
        });

        let q = self.filter.to_lowercase();
        let rows: Vec<&Session> = self
            .sessions
            .iter()
            .filter(|s| q.is_empty() || format!("{} {} {}", s.name(), s.model, s.project).to_lowercase().contains(&q))
            .collect();

        let mut clicked = None;
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                egui::Grid::new("sessions").striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                    ui.strong("Machine");
                    for (c, name) in [
                        (Col::Session, "Session"), (Col::Model, "Model"), (Col::Start, "Start context"),
                        (Col::Context, "Latest context"), (Col::Diff, "New context"),
                    ] {
                        let arrow = if self.sort == Some(c) { if self.desc { " ⬇" } else { " ⬆" } } else { "" };
                        if ui.button(egui::RichText::new(format!("{name}{arrow}")).strong()).clicked() {
                            clicked = Some(c);
                        }
                    }
                    ui.end_row();
                    for s in &rows {
                        ui.label(&self.machine);
                        ui.label(s.name()).on_hover_text(format!("{}\n{}", s.project, s.id));
                        ui.label(&s.model);
                        ui.label(kfmt(s.start))
                            .on_hover_text("Context when tracking started: at app start, or the first request of a session started later");
                        ui.add(egui::ProgressBar::new(s.context_frac()).desired_width(240.0).text(s.context_label()))
                            .on_hover_text("Prompt size of the latest request (input + cache write + cache read)");
                        ui.label(s.diff_label()).on_hover_text("New context added since start: latest context minus start context");
                        ui.end_row();
                    }
                });
            });
        });

        if let Some(c) = clicked {
            self.desc = if self.sort == Some(c) { !self.desc } else { true };
            self.sort = Some(c);
            self.sort();
        }
    }
}

fn main() -> eframe::Result {
    // Release builds have no console, so record panics where the data file lives.
    std::panic::set_hook(Box::new(|info| {
        let log = data_file().with_file_name("crash.log");
        let _ = std::fs::create_dir_all(log.parent().unwrap());
        let msg = format!("{info}\n{}\n", std::backtrace::Backtrace::force_capture());
        let _ = std::fs::OpenOptions::new().create(true).append(true).open(log).and_then(|mut f| {
            use std::io::Write;
            f.write_all(msg.as_bytes())
        });
    }));
    // `--dump`: print the index as TSV instead of opening the window (console only in debug builds).
    if std::env::args().any(|a| a == "--dump") {
        let mut index = Index::default();
        index.scan(&claude_dir().join("projects"), |_, _| {});
        print!("{}", to_tsv(&machine_name(), &index.snapshot()));
        return Ok(());
    }
    eframe::run_native(
        "Claude Usage Monitor",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1000.0, 650.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            Ok(Box::new(App {
                tray: start_tray(cc.egui_ctx.clone()),
                rx: start_watching(cc.egui_ctx.clone()),
                indexed: false,
                progress: (0, 0),
                machine: machine_name(),
                sessions: Vec::new(),
                filter: String::new(),
                sort: None,
                desc: true,
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: &str, ts: &str, cache_read: u64) -> String {
        req_in("s1", id, ts, cache_read)
    }

    fn req_in(session: &str, id: &str, ts: &str, cache_read: u64) -> String {
        format!(r#"{{"type":"assistant","sessionId":"{session}","cwd":"C:\\Code\\RAG","timestamp":"{ts}","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":0,"cache_read_input_tokens":{cache_read}}}}}}}"#)
    }

    #[test]
    fn names_and_skips_junk() {
        let (mut m, mut seen) = (HashMap::new(), HashSet::new());
        ingest(&mut m, &mut seen, &req("m1", "2026-01-01T00:00:00Z", 1102), "file");
        ingest(&mut m, &mut seen, "not json", "file");
        assert_eq!(m["s1"].name(), "RAG");
        ingest(&mut m, &mut seen, r#"{"type":"ai-title","aiTitle":"Hello","sessionId":"s1"}"#, "file");
        let s = &m["s1"];
        assert_eq!((s.name(), s.model.as_str(), s.context, s.window), ("Hello", "claude-opus-5-5", 1102, 1_000_000));
    }

    #[test]
    fn diff_is_latest_minus_start() {
        let root = std::env::temp_dir().join(format!("cum-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let (f1, f2) = (root.join("s1.jsonl"), root.join("s2.jsonl"));
        let old = req("m0", "2026-01-01T00:00:00Z", 4_000);
        let at_start = req("m1", "2026-01-01T00:01:00Z", 10_000);
        let next = req("m2", "2026-01-01T00:02:00Z", 100_000);
        // Last line half-written: must be ignored until its newline lands.
        std::fs::write(&f1, format!("{old}\n{at_start}\n{}", &next[..20])).unwrap();

        let mut index = Index::default();
        assert!(index.scan(&root, |_, _| {}));
        index.mark_start();
        let s = &index.sessions["s1"];
        assert_eq!((s.start, s.context, s.diff()), (10_000, 10_000, 0));

        std::fs::write(&f1, format!("{old}\n{at_start}\n{next}\n")).unwrap();
        assert!(index.scan(&root, |_, _| {}));
        assert_eq!(index.sessions["s1"].diff_label(), "+90.0K");

        let later = req("m3", "2026-01-01T00:07:00Z", 150_000);
        std::fs::write(&f1, format!("{old}\n{at_start}\n{next}\n{later}\n")).unwrap();
        // A session first seen after app start starts from its own first request.
        let s2 = format!("{}\n{}\n", req_in("s2", "n1", "2026-01-01T00:03:00Z", 10_000), req_in("s2", "n2", "2026-01-01T00:04:00Z", 100_000));
        std::fs::write(&f2, s2).unwrap();
        assert!(index.scan(&root, |_, _| {}));
        assert_eq!(index.sessions["s1"].diff_label(), "+140.0K");
        assert_eq!(index.sessions["s2"].diff_label(), "+90.0K");
        assert!(!index.scan(&root, |_, _| {}));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn context_windows() {
        for (m, w) in [
            ("claude-opus-5-5", 1_000_000), ("claude-opus-5", 1_000_000), ("claude-sonnet-4-6", 1_000_000),
            ("claude-haiku-4-5", 200_000), ("claude-opus-4-20250514", 200_000), ("claude-3-5-sonnet-20241022", 200_000),
        ] {
            assert_eq!(context_window(m), w, "{m}");
        }
    }
}
