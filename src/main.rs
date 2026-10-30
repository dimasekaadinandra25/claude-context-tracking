#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod details;
mod export;
mod index;
mod overhead;
mod platform;
mod session;
mod sync;
mod theme;
mod tray;
mod watcher;

use eframe::egui;
use std::io::Write;

const WINDOW_SIZE: [f32; 2] = [1100.0, 650.0];

fn main() -> eframe::Result {
    install_crash_log();

    if std::env::args().any(|arg| arg == "--dump") {
        print_snapshot();
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(WINDOW_SIZE),
        ..Default::default()
    };
    eframe::run_native(
        platform::APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}

fn print_snapshot() {
    let mut index = index::Index::default();
    index.scan(&platform::projects_dir(), |_, _| {});
    print!(
        "{}",
        export::to_tsv(&platform::machine_name(), &index.snapshot())
    );
}

fn install_crash_log() {
    std::panic::set_hook(Box::new(|info| {
        let path = platform::crash_log();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let report = format!("{info}\n{}\n", std::backtrace::Backtrace::force_capture());
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| file.write_all(report.as_bytes()));
    }));
}
