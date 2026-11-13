#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod activity;
mod alerts;
mod app;
mod autostart;
mod cache;
mod config;
mod details;
mod export;
mod icon;
mod index;
mod mini;
mod models;
mod overhead;
mod plan_limits;
mod platform;
mod session;
mod settings;
mod sync;
mod theme;
mod tray;
mod watcher;

use eframe::egui;
use std::fs::TryLockError;
use std::io::Write;

const WINDOW_ICON_SIZE: u32 = 64;

fn main() -> eframe::Result {
    install_crash_log();

    if std::env::args().any(|arg| arg == plan_limits::STATUS_LINE_FLAG) {
        std::process::exit(plan_limits::run_status_line());
    }

    if std::env::args().any(|arg| arg == "--dump") {
        print_snapshot();
        return Ok(());
    }

    let instance = platform::lock_instance(&platform::instance_lock_file());
    if matches!(instance, Err(TryLockError::WouldBlock)) {
        return Ok(());
    }

    let _ = autostart::upgrade();
    let _ = plan_limits::keep_path_current();
    let hidden = autostart::launched_hidden(std::env::args());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(theme::WINDOW_SIZE)
            .with_min_inner_size(theme::MIN_WINDOW_SIZE)
            .with_icon(egui::IconData {
                rgba: icon::rgba(WINDOW_ICON_SIZE),
                width: WINDOW_ICON_SIZE,
                height: WINDOW_ICON_SIZE,
            }),
        ..Default::default()
    };
    eframe::run_native(
        platform::APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, hidden)))),
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
