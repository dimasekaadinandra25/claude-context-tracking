use crate::cache;
use crate::export::{to_tsv, write_atomic};
use crate::index::Index;
use crate::platform;
use crate::session::Session;
use crate::sync::{SyncStatus, Syncer};
use eframe::egui;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const EXPORT_INTERVAL: Duration = Duration::from_secs(30 * 60);
const QUIT_TIMEOUT: Duration = Duration::from_secs(5);

pub enum Update {
    Indexing { done: usize, total: usize },
    Sessions(Vec<Session>),
    Sync(SyncStatus),
    ToggleMini,
}

pub enum Command {
    SyncNow,
    Quit,
}

pub fn spawn(ctx: egui::Context, tx: Sender<Update>) -> Sender<Command> {
    let (commands, rx) = channel();
    std::thread::spawn(move || run(&ctx, &tx, &rx));
    commands
}

pub fn quit(commands: &Sender<Command>) {
    std::thread::spawn(|| {
        std::thread::sleep(QUIT_TIMEOUT);
        std::process::exit(0);
    });
    if commands.send(Command::Quit).is_err() {
        std::process::exit(0);
    }
}

fn run(ctx: &egui::Context, tx: &Sender<Update>, commands: &Receiver<Command>) {
    let root = platform::projects_dir();
    let cache_file = cache::path();
    let mut index = cache::load(&cache_file).unwrap_or_default();
    let scanned = index.scan(&root, |done, total| {
        let _ = tx.send(Update::Indexing { done, total });
        ctx.request_repaint();
    });
    index.reset_baselines();
    let mut cache = cache::Saver::new(cache_file, scanned);

    let machine = platform::machine_name();
    let mut exporter = Exporter::new(machine.clone());
    let mut syncer = Syncer::new();
    let mut changed = true;
    loop {
        if changed {
            if tx.send(Update::Sessions(index.snapshot())).is_err() {
                return;
            }
            ctx.request_repaint();
        }
        exporter.tick(&index);
        cache.tick(&index);
        if let Some(status) = syncer.tick(&index, &machine) {
            let _ = tx.send(Update::Sync(status));
            ctx.request_repaint();
        }
        match commands.recv_timeout(POLL_INTERVAL) {
            Ok(Command::SyncNow) => syncer.request(),
            Ok(Command::Quit) => {
                cache.mark(index.scan(&root, |_, _| {}));
                syncer.run(&index, &machine);
                cache.flush(&index);
                std::process::exit(0);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        changed = index.scan(&root, |_, _| {});
        cache.mark(changed);
    }
}

struct Exporter {
    machine: String,
    last_run: Option<Instant>,
    last_written: String,
}

impl Exporter {
    fn new(machine: String) -> Self {
        Self {
            machine,
            last_run: None,
            last_written: String::new(),
        }
    }

    fn tick(&mut self, index: &Index) {
        if self
            .last_run
            .is_some_and(|at| at.elapsed() < EXPORT_INTERVAL)
        {
            return;
        }
        self.last_run = Some(Instant::now());
        let tsv = to_tsv(&self.machine, &index.snapshot());
        if tsv != self.last_written && write_atomic(&platform::data_file(), &tsv).is_ok() {
            self.last_written = tsv;
        }
    }
}
