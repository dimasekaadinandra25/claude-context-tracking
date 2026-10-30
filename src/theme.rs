use eframe::egui::{Color32, Context, Stroke, Theme, Visuals};

pub const ACCENT: Color32 = Color32::from_rgb(217, 119, 87);

struct Palette {
    background: Color32,
    surface: Color32,
    sunken: Color32,
    border: Color32,
    text: Color32,
    strong_text: Color32,
}

const DARK: Palette = Palette {
    background: Color32::from_rgb(38, 38, 36),
    surface: Color32::from_rgb(48, 48, 46),
    sunken: Color32::from_rgb(31, 30, 29),
    border: Color32::from_rgb(66, 65, 60),
    text: Color32::from_rgb(194, 192, 182),
    strong_text: Color32::from_rgb(250, 249, 245),
};

const LIGHT: Palette = Palette {
    background: Color32::from_rgb(250, 249, 245),
    surface: Color32::from_rgb(240, 238, 230),
    sunken: Color32::from_rgb(255, 255, 255),
    border: Color32::from_rgb(222, 220, 209),
    text: Color32::from_rgb(61, 61, 58),
    strong_text: Color32::from_rgb(20, 20, 19),
};

pub fn apply(ctx: &Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(Visuals::dark(), &DARK));
    ctx.set_visuals_of(Theme::Light, visuals(Visuals::light(), &LIGHT));
}

fn visuals(mut visuals: Visuals, palette: &Palette) -> Visuals {
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.background;
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.faint_bg_color = palette.surface;
    visuals.extreme_bg_color = palette.sunken;
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.6);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = palette.background;
    widgets.noninteractive.weak_bg_fill = palette.background;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    for state in [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
    ] {
        state.bg_fill = palette.surface;
        state.weak_bg_fill = palette.surface;
        state.fg_stroke = Stroke::new(1.0, palette.strong_text);
    }
    widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals
}
