use crate::platform::{self, APP_NAME};
use crate::watcher::{self, Command, Update};
use eframe::egui;
use std::sync::mpsc::Sender;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const ICON_SIZE: u32 = 32;

#[cfg(not(target_os = "linux"))]
pub type Tray = TrayIcon;

#[cfg(target_os = "linux")]
pub type Tray = ();

#[cfg(not(target_os = "linux"))]
pub fn spawn(
    ctx: egui::Context,
    updates: Sender<Update>,
    commands: Sender<Command>,
) -> Option<Tray> {
    build(ctx, updates, commands)
}

#[cfg(target_os = "linux")]
pub fn spawn(
    ctx: egui::Context,
    updates: Sender<Update>,
    commands: Sender<Command>,
) -> Option<Tray> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if gtk::init().is_err() {
            let _ = tx.send(false);
            return;
        }
        let tray = build(ctx, updates, commands);
        let _ = tx.send(tray.is_some());
        gtk::main();
        drop(tray);
    });
    rx.recv().unwrap_or(false).then_some(())
}

#[cfg(not(target_os = "linux"))]
pub fn set_tooltip(tray: &Tray, text: &str) {
    let _ = tray.set_tooltip(Some(text));
}

#[cfg(target_os = "linux")]
pub fn set_tooltip(_tray: &Tray, _text: &str) {}

pub fn set_window_visible(ctx: &egui::Context, visible: bool) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(visible));
    if visible {
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
    ctx.request_repaint();
}

fn build(
    ctx: egui::Context,
    updates: Sender<Update>,
    commands: Sender<Command>,
) -> Option<TrayIcon> {
    let show = MenuItem::new("Show", true, None);
    let mini = MenuItem::new("Mini mode", true, None);
    let sync = MenuItem::new("Sync now", true, None);
    let open = MenuItem::new("Open data file", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    menu.append_items(&[&show, &mini, &sync, &open, &quit])
        .ok()?;

    let (show_id, mini_id, sync_id) = (show.id().clone(), mini.id().clone(), sync.id().clone());
    let (open_id, quit_id) = (open.id().clone(), quit.id().clone());
    let menu_ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == show_id {
            set_window_visible(&menu_ctx, true);
        } else if event.id == mini_id {
            let _ = updates.send(Update::ToggleMini);
            set_window_visible(&menu_ctx, true);
        } else if event.id == sync_id {
            let _ = commands.send(Command::SyncNow);
        } else if event.id == open_id {
            platform::open_in_default_app(&platform::data_file());
        } else if event.id == quit_id {
            set_window_visible(&menu_ctx, false);
            watcher::quit(&commands);
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            set_window_visible(&ctx, true);
        }
    }));

    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(APP_NAME)
        .with_icon(icon())
        .build()
        .ok()
}

fn icon() -> Icon {
    Icon::from_rgba(crate::icon::rgba(ICON_SIZE), ICON_SIZE, ICON_SIZE)
        .expect("icon buffer matches its dimensions")
}
