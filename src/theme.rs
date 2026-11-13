use eframe::egui;
use eframe::egui::epaint::{RectShape, TessellationOptions, Tessellator};
use eframe::egui::gui_zoom::kb_shortcuts::{ZOOM_IN, ZOOM_IN_SECONDARY, ZOOM_OUT, ZOOM_RESET};
use eframe::egui::{
    Color32, Context, CornerRadius, Frame, LayerId, Margin, Mesh, Painter, Rect, Response,
    ScrollArea, Sense, Shadow, Shape, Stroke, TextStyle, Theme, Ui, UiBuilder, Vec2, Visuals,
    Window, pos2, vec2,
};
use std::f32::consts::TAU;

pub const ACCENT: Color32 = Color32::from_rgb(217, 119, 87);
pub const VIOLET: Color32 = Color32::from_rgb(155, 95, 208);
pub const WINDOW_SIZE: [f32; 2] = [560.0, 640.0];
pub const MIN_WINDOW_SIZE: [f32; 2] = [480.0, 360.0];
pub const LIVE: Color32 = Color32::from_rgb(79, 195, 161);
pub const AMBER: Color32 = Color32::from_rgb(227, 182, 92);
pub const RED: Color32 = Color32::from_rgb(229, 83, 75);
pub const WARN_AT: f32 = 0.8;
pub const FULL_AT: f32 = 0.9;

const GLASS_RADIUS: u8 = 14;
const WINDOW_RADIUS: u8 = 16;
const WIDGET_RADIUS: u8 = 10;
const PILL_RADIUS: u8 = 20;
const LOGO_PIXELS: u32 = 64;
const LOGO_TEXTURE: &str = "app_logo";
const BODY_SIZE: f32 = 12.0;
const SMALL_SIZE: f32 = 10.0;
const HEADING_SIZE: f32 = 14.0;
const MAX_ZOOM: f32 = 1.5;
const ZOOM_STEP: f32 = 0.05;
const USER_ZOOM_STEP: f32 = 0.1;
const MIN_USER_ZOOM: f32 = 0.7;
const MAX_USER_ZOOM: f32 = 2.0;
const DIALOG_CHROME: f32 = 48.0;
const GLOW_SEGMENTS: usize = 64;
const ICON_STROKE: f32 = 1.4;
const REFRESH_GAP: f32 = 0.9;
const REFRESH_STEPS: usize = 24;
const SUN_RAYS: usize = 8;
const GEAR_TEETH: usize = 8;
const MOON_STEPS: usize = 20;
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
    weak_text: Color32,
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
    weak_text: Color32::from_rgb(150, 142, 166),
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
    weak_text: Color32::from_rgb(104, 95, 117),
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
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
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
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.weak_text_color = Some(palette.weak_text);

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

pub fn scale_to_window(ctx: &Context, user_zoom: &mut f32) {
    *user_zoom = ctx.input_mut(|input| {
        if input.consume_shortcut(&ZOOM_RESET) {
            1.0
        } else if input.consume_shortcut(&ZOOM_IN) || input.consume_shortcut(&ZOOM_IN_SECONDARY) {
            (*user_zoom + USER_ZOOM_STEP).min(MAX_USER_ZOOM)
        } else if input.consume_shortcut(&ZOOM_OUT) {
            (*user_zoom - USER_ZOOM_STEP).max(MIN_USER_ZOOM)
        } else {
            *user_zoom
        }
    });
    let zoom = ctx.zoom_factor();
    let target = zoom_target(ctx.content_rect().size() * zoom, *user_zoom);
    if (target - zoom).abs() >= ZOOM_STEP / 2.0 {
        ctx.set_zoom_factor(target);
    }
}

fn zoom_target(size: Vec2, user_zoom: f32) -> f32 {
    let fit = (size.x / MIN_WINDOW_SIZE[0]).max(1.0);
    (zoom_for(size) * user_zoom).min(fit)
}

pub fn dialog(ctx: &Context, title: &str, open: &mut bool, add: impl FnOnce(&mut Ui)) {
    let height = ctx.content_rect().height() - DIALOG_CHROME;
    Window::new(title)
        .open(open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), height));
            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                ScrollArea::vertical().show(ui, add)
            });
        });
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

#[derive(Clone, Copy)]
pub enum Icon {
    Mini,
    Refresh,
    Search,
    Sun,
    Moon,
    Gear,
}

pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let color = if response.hovered() {
        ACCENT
    } else {
        ui.visuals().text_color()
    };
    paint_icon(ui, rect, icon, color);
    response
}

pub fn icon(ui: &mut Ui, icon: Icon, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_icon(ui, rect, icon, ui.visuals().weak_text_color());
}

fn paint_icon(ui: &Ui, rect: Rect, icon: Icon, color: Color32) {
    let stroke = Stroke::new(ICON_STROKE, color);
    let painter = ui.painter();
    let unit = rect.width() / 2.0;
    match icon {
        Icon::Mini => {
            let box_size = Vec2::splat(unit * 0.95);
            let shift = vec2(unit * 0.2, -unit * 0.2);
            let back = Rect::from_center_size(rect.center() + shift, box_size);
            let front = Rect::from_center_size(rect.center() - shift, box_size);
            painter.rect_stroke(back, 2.0, stroke, eframe::egui::StrokeKind::Middle);
            painter.rect_filled(front, 2.0, palette(ui).window);
            painter.rect_stroke(front, 2.0, stroke, eframe::egui::StrokeKind::Middle);
        }
        Icon::Refresh => {
            let (points, tip, base) = refresh_arc(rect.center(), unit * 0.62);
            painter.add(Shape::line(points, stroke));
            painter.add(Shape::convex_polygon(
                base.into_iter().chain([tip]).collect(),
                color,
                Stroke::NONE,
            ));
        }
        Icon::Search => {
            let lens = rect.center() - Vec2::splat(unit * 0.15);
            let radius = unit * 0.45;
            painter.circle_stroke(lens, radius, stroke);
            let handle = Vec2::splat(std::f32::consts::FRAC_1_SQRT_2);
            painter.line_segment(
                [lens + handle * radius, lens + handle * unit * 0.95],
                stroke,
            );
        }
        Icon::Sun => {
            painter.circle_filled(rect.center(), unit * 0.34, color);
            for (from, to) in rays(rect.center(), unit * 0.56, unit * 0.82, SUN_RAYS) {
                painter.line_segment([from, to], stroke);
            }
        }
        Icon::Moon => {
            painter.add(crescent(rect.center(), unit * 0.62, color));
        }
        Icon::Gear => {
            let teeth = Stroke::new(ICON_STROKE * 2.2, color);
            for (from, to) in rays(rect.center(), unit * 0.5, unit * 0.8, GEAR_TEETH) {
                painter.line_segment([from, to], teeth);
            }
            painter.circle_stroke(rect.center(), unit * 0.52, stroke);
            painter.circle_stroke(rect.center(), unit * 0.2, stroke);
        }
    }
}

pub fn search_field(ui: &Ui, focused: bool) -> Frame {
    let palette = palette(ui);
    let border = if focused {
        ACCENT
    } else {
        palette.glass_stroke
    };
    Frame::new()
        .fill(palette.field)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(PILL_RADIUS)
        .inner_margin(Margin::symmetric(10, 2))
}

fn rays(
    center: eframe::egui::Pos2,
    inner: f32,
    outer: f32,
    count: usize,
) -> Vec<(eframe::egui::Pos2, eframe::egui::Pos2)> {
    (0..count)
        .map(|i| {
            let angle = TAU * i as f32 / count as f32;
            let direction = vec2(angle.cos(), angle.sin());
            (center + direction * inner, center + direction * outer)
        })
        .collect()
}

fn crescent(center: eframe::egui::Pos2, radius: f32, color: Color32) -> Mesh {
    let offset = vec2(radius * 0.55, -radius * 0.45);
    let inner_center = center + offset;
    let inner_radius = radius * 0.85;
    let distance = offset.length();
    let along =
        (radius * radius - inner_radius * inner_radius + distance * distance) / (2.0 * distance);
    let across = (radius * radius - along * along).max(0.0).sqrt();
    let axis = offset / distance;
    let normal = vec2(-axis.y, axis.x);
    let base = center + axis * along;
    let tips = [base + normal * across, base - normal * across];
    let away = (-offset).angle();
    let outer = arc_through(center, radius, tips, away);
    let inner = arc_through(inner_center, inner_radius, tips, away);
    let mut mesh = Mesh::default();
    for (o, i) in outer.iter().zip(&inner) {
        mesh.colored_vertex(*o, color);
        mesh.colored_vertex(*i, color);
    }
    for k in 0..MOON_STEPS as u32 {
        let a = 2 * k;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    mesh
}

fn arc_through(
    center: eframe::egui::Pos2,
    radius: f32,
    tips: [eframe::egui::Pos2; 2],
    via: f32,
) -> Vec<eframe::egui::Pos2> {
    let start = (tips[0] - center).angle();
    let mut end = (tips[1] - center).angle();
    let contains = |from: f32, to: f32| {
        let span = (to - from).rem_euclid(TAU);
        (via - from).rem_euclid(TAU) <= span
    };
    if contains(start, end) {
        end = start + (end - start).rem_euclid(TAU);
    } else {
        end = start - (start - end).rem_euclid(TAU);
    }
    (0..=MOON_STEPS)
        .map(|k| {
            let angle = start + (end - start) * k as f32 / MOON_STEPS as f32;
            center + vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

fn refresh_arc(
    center: eframe::egui::Pos2,
    radius: f32,
) -> (
    Vec<eframe::egui::Pos2>,
    eframe::egui::Pos2,
    [eframe::egui::Pos2; 2],
) {
    let start = -TAU / 4.0 + REFRESH_GAP;
    let sweep = TAU - REFRESH_GAP;
    let at = |angle: f32| center + vec2(angle.cos(), angle.sin()) * radius;
    let points: Vec<_> = (0..=REFRESH_STEPS)
        .map(|i| at(start + sweep * i as f32 / REFRESH_STEPS as f32))
        .collect();
    let end_angle = start + sweep;
    let end = at(end_angle);
    let tangent = vec2(-end_angle.sin(), end_angle.cos());
    let normal = vec2(end_angle.cos(), end_angle.sin());
    let head = radius * 0.55;
    let tip = end + tangent * head;
    (
        points,
        tip,
        [end + normal * head * 0.6, end - normal * head * 0.6],
    )
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
    let id = egui::Id::unique(LOGO_TEXTURE);
    let texture = ui
        .ctx()
        .data(|data| data.get_temp::<egui::TextureHandle>(id))
        .unwrap_or_else(|| {
            let side = LOGO_PIXELS as usize;
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [side, side],
                &crate::icon::rgba(LOGO_PIXELS),
            );
            let texture = ui
                .ctx()
                .load_texture(LOGO_TEXTURE, image, egui::TextureOptions::LINEAR);
            ui.ctx()
                .data_mut(|data| data.insert_temp(id, texture.clone()));
            texture
        });
    ui.add(egui::Image::new((texture.id(), Vec2::splat(size))));
}

pub fn gradient_bar(ui: &mut Ui, fraction: f32, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let radius = rect.height() / 2.0;
    painter.rect_filled(rect, radius, palette(ui).track);
    let fraction = fraction.clamp(0.0, 1.0);
    let filled = Rect::from_min_size(
        rect.min,
        vec2(
            fill_width(rect.width(), rect.height(), fraction),
            rect.height(),
        ),
    );
    let end = level_color(fraction).unwrap_or_else(|| ACCENT.lerp_to_gamma(VIOLET, fraction));
    gradient_fill(painter, filled, radius, ACCENT, end);
    response
}

fn gradient_fill(painter: &Painter, rect: Rect, radius: f32, from: Color32, to: Color32) {
    if rect.width() <= 0.0 {
        return;
    }
    let options = painter.ctx().tessellation_options(|options| *options);
    painter.add(gradient_mesh(
        rect,
        radius,
        (from, to),
        painter.pixels_per_point(),
        options,
    ));
}

fn gradient_mesh(
    rect: Rect,
    radius: f32,
    (from, to): (Color32, Color32),
    pixels_per_point: f32,
    options: TessellationOptions,
) -> Mesh {
    let mut tessellator = Tessellator::new(pixels_per_point, options, [1, 1], Vec::new());
    let mut mesh = Mesh::default();
    tessellator.tessellate_rect(&RectShape::filled(rect, radius, Color32::WHITE), &mut mesh);
    for vertex in &mut mesh.vertices {
        let position = (vertex.pos.x - rect.left()) / rect.width();
        vertex.color = from
            .lerp_to_gamma(to, position.clamp(0.0, 1.0))
            .gamma_multiply(f32::from(vertex.color.a()) / 255.0);
    }
    mesh
}

fn fill_width(track: f32, height: f32, fraction: f32) -> f32 {
    if fraction <= 0.0 {
        return 0.0;
    }
    (track * fraction).max(height).min(track)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_fills_stay_round_and_never_leave_the_track() {
        assert_eq!(fill_width(100.0, 7.0, 0.0), 0.0);
        assert_eq!(fill_width(100.0, 7.0, 0.02), 7.0);
        assert_eq!(fill_width(100.0, 7.0, 0.5), 50.0);
        assert_eq!(fill_width(100.0, 7.0, 1.0), 100.0);
        assert_eq!(fill_width(4.0, 7.0, 0.5), 4.0);
    }

    #[test]
    fn gradient_fill_is_one_shape_with_the_tracks_outline() {
        let rect = Rect::from_min_size(pos2(10.0, 20.3), vec2(50.0, 7.0));
        let options = TessellationOptions::default();
        let mesh = gradient_mesh(rect, 3.5, (ACCENT, VIOLET), 1.0, options);
        let mut track = Mesh::default();
        Tessellator::new(1.0, options, [1, 1], Vec::new())
            .tessellate_rect(&RectShape::filled(rect, 3.5, Color32::WHITE), &mut track);
        assert_eq!(mesh.vertices.len(), track.vertices.len());
        assert!(
            mesh.vertices
                .iter()
                .zip(&track.vertices)
                .all(|(a, b)| a.pos == b.pos && a.color.a() == b.color.a())
        );
        let solid = |x: f32| {
            mesh.vertices
                .iter()
                .filter(|v| v.color.a() == 255)
                .min_by(|a, b| (a.pos.x - x).abs().total_cmp(&(b.pos.x - x).abs()))
                .unwrap()
                .color
        };
        assert!(solid(rect.left()).r() > solid(rect.right()).r());
    }

    #[test]
    fn zoom_grows_with_window_up_to_limit() {
        assert_eq!(zoom_for(vec2(WINDOW_SIZE[0], WINDOW_SIZE[1])), 1.0);
        assert_eq!(zoom_for(vec2(400.0, 300.0)), 1.0);
        assert!((zoom_for(vec2(840.0, 1000.0)) - 1.5).abs() < 1e-4);
        assert!((zoom_for(vec2(1920.0, 768.0)) - 1.2).abs() < 1e-4);
        assert_eq!(zoom_for(vec2(3840.0, 2160.0)), MAX_ZOOM);
    }

    #[test]
    fn zoom_never_squeezes_content_below_minimum_width() {
        assert_eq!(zoom_target(vec2(480.0, 360.0), 2.0), 1.0);
        assert!((zoom_target(vec2(560.0, 640.0), 2.0) - 560.0 / 480.0).abs() < 1e-4);
        assert!((zoom_target(vec2(1920.0, 1080.0), 2.0) - 3.0).abs() < 1e-4);
        assert!((zoom_target(vec2(1920.0, 1080.0), 1.0) - 1.5).abs() < 1e-4);
        assert!((zoom_target(vec2(480.0, 360.0), 0.7) - 0.7).abs() < 1e-4);
        assert_eq!(zoom_target(vec2(340.0, 110.0), 1.5), 1.0);
    }

    #[test]
    fn refresh_arc_is_centered_with_head_at_the_end() {
        let center = pos2(10.0, 10.0);
        let (points, tip, base) = refresh_arc(center, 5.0);
        assert_eq!(points.len(), REFRESH_STEPS + 1);
        assert!(
            points
                .iter()
                .all(|p| (p.distance(center) - 5.0).abs() < 1e-3)
        );
        let end = *points.last().unwrap();
        assert!((base[0].distance(end) - base[1].distance(end)).abs() < 1e-3);
        assert!(tip.distance(end) > 0.0);
    }

    #[test]
    fn crescent_is_a_closed_filled_strip_inside_the_icon() {
        let center = pos2(10.0, 10.0);
        let mesh = crescent(center, 6.0, ACCENT);
        assert_eq!(mesh.vertices.len(), 2 * (MOON_STEPS + 1));
        assert_eq!(mesh.indices.len(), 6 * MOON_STEPS);
        assert!(
            mesh.vertices
                .iter()
                .all(|v| v.pos.distance(center) <= 6.0 + 1e-3)
        );
        let first = &mesh.vertices[..2];
        assert!(first[0].pos.distance(first[1].pos) < 1e-3);
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
