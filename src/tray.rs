use crate::platform::{self, APP_NAME};
use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const ICON_SIZE: u32 = 32;
const ICON_RGB: [u8; 3] = [217, 119, 87];

#[cfg(not(target_os = "linux"))]
pub type Tray = TrayIcon;

#[cfg(target_os = "linux")]
pub type Tray = ();

#[cfg(not(target_os = "linux"))]
pub fn spawn(ctx: egui::Context) -> Option<Tray> {
    build(ctx)
}

#[cfg(target_os = "linux")]
pub fn spawn(ctx: egui::Context) -> Option<Tray> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if gtk::init().is_err() {
            let _ = tx.send(false);
            return;
        }
        let tray = build(ctx);
        let _ = tx.send(tray.is_some());
        gtk::main();
        drop(tray);
    });
    rx.recv().unwrap_or(false).then_some(())
}

pub fn set_window_visible(ctx: &egui::Context, visible: bool) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(visible));
    if visible {
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
    ctx.request_repaint();
}

fn build(ctx: egui::Context) -> Option<TrayIcon> {
    let show = MenuItem::new("Show", true, None);
    let open = MenuItem::new("Open data file", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    menu.append_items(&[&show, &open, &quit]).ok()?;

    let (show_id, open_id, quit_id) = (show.id().clone(), open.id().clone(), quit.id().clone());
    let menu_ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == show_id {
            set_window_visible(&menu_ctx, true);
        } else if event.id == open_id {
            platform::open_in_default_app(&platform::data_file());
        } else if event.id == quit_id {
            std::process::exit(0);
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
    let center = (ICON_SIZE as f32 - 1.0) / 2.0;
    let radius_sq = (ICON_SIZE as f32 / 2.0 - 1.0).powi(2);
    let mut rgba = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let (dx, dy) = (x as f32 - center, y as f32 - center);
            let alpha = if dx * dx + dy * dy <= radius_sq {
                u8::MAX
            } else {
                0
            };
            rgba.extend_from_slice(&ICON_RGB);
            rgba.push(alpha);
        }
    }
    Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE).expect("icon buffer matches its dimensions")
}
