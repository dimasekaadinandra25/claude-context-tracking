use eframe::egui::{
    Color32, Context, CornerRadius, Frame, LayerId, Margin, Mesh, Painter, Rect, Response, Sense,
    Shadow, Stroke, TextStyle, Theme, Ui, Vec2, Visuals, pos2, vec2,
};
use std::f32::consts::TAU;

pub const ACCENT: Color32 = Color32::from_rgb(217, 119, 87);
pub const VIOLET: Color32 = Color32::from_rgb(155, 95, 208);
pub const WINDOW_SIZE: [f32; 2] = [560.0, 640.0];
pub const LIVE: Color32 = Color32::from_rgb(79, 195, 161);
pub const AMBER: Color32 = Color32::from_rgb(227, 182, 92);
pub const RED: Color32 = Color32::from_rgb(229, 83, 75);
pub const WARN_AT: f32 = 0.8;
pub const FULL_AT: f32 = 0.9;

const GLASS_RADIUS: u8 = 14;
const WINDOW_RADIUS: u8 = 16;
const WIDGET_RADIUS: u8 = 10;
const PILL_RADIUS: u8 = 20;
const LOGO_RADIUS: f32 = 5.0;
const BODY_SIZE: f32 = 12.0;
const SMALL_SIZE: f32 = 10.0;
const HEADING_SIZE: f32 = 14.0;
const MAX_ZOOM: f32 = 1.5;
const ZOOM_STEP: f32 = 0.05;
const GLOW_SEGMENTS: usize = 64;
const GLOW_MID: f32 = 0.55;
const GLOW_MID_STRENGTH: f32 = 0.45;

struct Glow {
    color: Color32,
    anchor: [f32; 2],
    radius: [f32; 2],
}

struct Palette {
    background: Color32,
    glass: Color32,
    glass_stroke: Color32,
    window: Color32,
    field: Color32,
    track: Color32,
    border: Color32,
    text: Color32,
    strong_text: Color32,
    shadow: Color32,
    glows: [Glow; 2],
}

const DARK: Palette = Palette {
    background: Color32::from_rgb(15, 12, 18),
    glass: Color32::from_rgba_premultiplied(13, 13, 13, 13),
    glass_stroke: Color32::from_rgba_premultiplied(23, 23, 23, 23),
    window: Color32::from_rgb(28, 23, 32),
    field: Color32::from_rgba_premultiplied(18, 18, 18, 18),
    track: Color32::from_rgba_premultiplied(26, 26, 26, 26),
    border: Color32::from_rgb(52, 44, 58),
    text: Color32::from_rgb(169, 161, 184),
    strong_text: Color32::from_rgb(245, 241, 250),
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 90),
    glows: [
        Glow {
            color: Color32::from_rgba_premultiplied(56, 31, 23, 66),
            anchor: [0.0, 0.0],
            radius: [1.1, 0.95],
        },
        Glow {
            color: Color32::from_rgba_premultiplied(28, 19, 42, 51),
            anchor: [1.0, 1.0],
            radius: [1.0, 0.9],
        },
    ],
};

const LIGHT: Palette = Palette {
    background: Color32::from_rgb(238, 233, 236),
    glass: Color32::from_rgba_premultiplied(128, 128, 128, 128),
    glass_stroke: Color32::from_rgba_premultiplied(191, 191, 191, 191),
    window: Color32::from_rgb(250, 247, 249),
    field: Color32::from_rgba_premultiplied(153, 153, 153, 153),
    track: Color32::from_rgba_premultiplied(3, 2, 4, 30),
    border: Color32::from_rgb(220, 210, 220),
    text: Color32::from_rgb(85, 76, 98),
    strong_text: Color32::from_rgb(30, 23, 38),
    shadow: Color32::from_rgba_premultiplied(10, 6, 12, 28),
    glows: [
        Glow {
            color: Color32::from_rgba_premultiplied(84, 65, 55, 84),
            anchor: [0.0, 0.0],
            radius: [1.1, 0.95],
        },
        Glow {
            color: Color32::from_rgba_premultiplied(55, 50, 69, 70),
            anchor: [1.0, 1.0],
            radius: [1.0, 0.9],
        },
    ],
};

pub fn apply(ctx: &Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(Visuals::dark(), &DARK));
    ctx.set_visuals_of(Theme::Light, visuals(Visuals::light(), &LIGHT));
    ctx.all_styles_mut(|style| {
        for (text_style, size) in [
            (TextStyle::Body, BODY_SIZE),
            (TextStyle::Button, BODY_SIZE),
            (TextStyle::Monospace, BODY_SIZE),
            (TextStyle::Small, SMALL_SIZE),
            (TextStyle::Heading, HEADING_SIZE),
        ] {
            if let Some(font) = style.text_styles.get_mut(&text_style) {
                font.size = size;
            }
        }
    });
}

fn palette(ui: &Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

fn visuals(mut visuals: Visuals, palette: &Palette) -> Visuals {
    visuals.panel_fill = Color32::TRANSPARENT;
    visuals.window_fill = palette.window;
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    visuals.window_shadow = Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: palette.shadow,
    };
    visuals.menu_corner_radius = CornerRadius::same(WIDGET_RADIUS);
    visuals.popup_shadow = visuals.window_shadow;
    visuals.faint_bg_color = palette.glass;
    visuals.extreme_bg_color = palette.field;
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.6);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = palette.window;
    widgets.noninteractive.weak_bg_fill = palette.window;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    for state in [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.bg_fill = palette.field;
        state.weak_bg_fill = palette.field;
        state.bg_stroke = Stroke::new(1.0, palette.glass_stroke);
        state.fg_stroke = Stroke::new(1.0, palette.strong_text);
        state.corner_radius = CornerRadius::same(WIDGET_RADIUS);
    }
    widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals
}

pub fn scale_to_window(ctx: &Context) {
    let zoom = ctx.zoom_factor();
    let target = zoom_for(ctx.content_rect().size() * zoom);
    if (target - zoom).abs() >= ZOOM_STEP / 2.0 {
        ctx.set_zoom_factor(target);
    }
}

fn zoom_for(size: Vec2) -> f32 {
    let ratio = (size.x / WINDOW_SIZE[0]).min(size.y / WINDOW_SIZE[1]);
    let zoom = ratio.clamp(1.0, MAX_ZOOM);
    (zoom / ZOOM_STEP).floor() * ZOOM_STEP
}

pub fn paint_backdrop(ui: &Ui) {
    let palette = palette(ui);
    let rect = ui.ctx().content_rect();
    let painter = ui.ctx().layer_painter(LayerId::background());
    painter.rect_filled(rect, 0.0, palette.background);
    for glow in &palette.glows {
        let center = pos2(
            rect.left() + rect.width() * glow.anchor[0],
            rect.top() + rect.height() * glow.anchor[1],
        );
        let radius = vec2(
            rect.width() * glow.radius[0],
            rect.height() * glow.radius[1],
        );
        painter.add(radial_glow(center, radius, glow.color));
    }
}

fn radial_glow(center: eframe::egui::Pos2, radius: Vec2, color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    let rings = [
        (GLOW_MID, color.gamma_multiply(GLOW_MID_STRENGTH)),
        (1.0, Color32::TRANSPARENT),
    ];
    for (scale, ring_color) in rings {
        for i in 0..GLOW_SEGMENTS {
            let angle = TAU * i as f32 / GLOW_SEGMENTS as f32;
            let offset = vec2(angle.cos() * radius.x, angle.sin() * radius.y) * scale;
            mesh.colored_vertex(center + offset, ring_color);
        }
    }
    let segments = GLOW_SEGMENTS as u32;
    for i in 0..segments {
        let next = (i + 1) % segments;
        let (inner, inner_next) = (1 + i, 1 + next);
        let (outer, outer_next) = (1 + segments + i, 1 + segments + next);
        mesh.add_triangle(0, inner, inner_next);
        mesh.add_triangle(inner, outer, outer_next);
        mesh.add_triangle(inner, outer_next, inner_next);
    }
    mesh
}

pub fn glass(ui: &Ui) -> Frame {
    let palette = palette(ui);
    Frame::new()
        .fill(palette.glass)
        .stroke(Stroke::new(1.0, palette.glass_stroke))
        .corner_radius(GLASS_RADIUS)
        .shadow(Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: palette.shadow.gamma_multiply(0.5),
        })
}

pub fn dot(ui: &mut Ui, radius: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(radius * 2.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), radius, color);
}

pub fn pill(ui: &Ui) -> Frame {
    let palette = palette(ui);
    Frame::new()
        .fill(palette.glass)
        .stroke(Stroke::new(1.0, palette.glass_stroke))
        .corner_radius(PILL_RADIUS)
        .inner_margin(Margin::symmetric(10, 3))
}

pub fn level_color(fraction: f32) -> Option<Color32> {
    if fraction >= FULL_AT {
        Some(RED)
    } else if fraction >= WARN_AT {
        Some(AMBER)
    } else {
        None
    }
}

pub fn logo(ui: &mut Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    gradient_fill(ui.painter(), rect, LOGO_RADIUS, ACCENT, VIOLET);
}

pub fn gradient_bar(ui: &mut Ui, fraction: f32, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let radius = rect.height() / 2.0;
    painter.rect_filled(rect, radius, palette(ui).track);
    let fraction = fraction.clamp(0.0, 1.0);
    let filled = Rect::from_min_size(rect.min, vec2(rect.width() * fraction, rect.height()));
    let end = level_color(fraction).unwrap_or_else(|| ACCENT.lerp_to_gamma(VIOLET, fraction));
    gradient_fill(painter, filled, radius, ACCENT, end);
    response
}

fn gradient_fill(painter: &Painter, rect: Rect, radius: f32, from: Color32, to: Color32) {
    if rect.width() <= 0.0 {
        return;
    }
    if rect.width() <= radius * 2.0 {
        painter.rect_filled(rect, radius, from);
        return;
    }
    let cap = vec2(radius * 2.0, rect.height());
    painter.rect_filled(Rect::from_min_size(rect.min, cap), radius, from);
    painter.rect_filled(
        Rect::from_min_size(pos2(rect.right() - cap.x, rect.top()), cap),
        radius,
        to,
    );
    let middle =
        Rect::from_x_y_ranges(rect.left() + radius..=rect.right() - radius, rect.y_range());
    let mut mesh = Mesh::default();
    mesh.colored_vertex(middle.left_top(), from);
    mesh.colored_vertex(middle.right_top(), to);
    mesh.colored_vertex(middle.right_bottom(), to);
    mesh.colored_vertex(middle.left_bottom(), from);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_grows_with_window_up_to_limit() {
        assert_eq!(zoom_for(vec2(WINDOW_SIZE[0], WINDOW_SIZE[1])), 1.0);
        assert_eq!(zoom_for(vec2(400.0, 300.0)), 1.0);
        assert!((zoom_for(vec2(840.0, 1000.0)) - 1.5).abs() < 1e-4);
        assert!((zoom_for(vec2(1920.0, 768.0)) - 1.2).abs() < 1e-4);
        assert_eq!(zoom_for(vec2(3840.0, 2160.0)), MAX_ZOOM);
    }

    #[test]
    fn radial_glow_fades_to_transparent_edge() {
        let mesh = radial_glow(pos2(0.0, 0.0), vec2(100.0, 50.0), ACCENT);
        assert_eq!(mesh.vertices.len(), 1 + 2 * GLOW_SEGMENTS);
        assert_eq!(mesh.indices.len(), 3 * 3 * GLOW_SEGMENTS);
        assert_eq!(mesh.vertices[0].color, ACCENT);
        assert!(
            mesh.vertices[1 + GLOW_SEGMENTS..]
                .iter()
                .all(|v| v.color == Color32::TRANSPARENT)
        );
        assert_eq!(mesh.vertices[1 + GLOW_SEGMENTS].pos, pos2(100.0, 0.0));
    }
}
