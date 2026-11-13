const BACKGROUND: [f32; 3] = [217.0, 119.0, 87.0];
const RING: [f32; 3] = [255.0, 255.0, 255.0];
const SAMPLES: u32 = 4;
const CORNER: f32 = 0.22;
const RING_OUTER: f32 = 0.33;
const RING_INNER: f32 = 0.2;
const TRACK_ALPHA: f32 = 0.35;
const FILLED_TURNS: f32 = 0.7;

fn in_rounded_square(x: f32, y: f32) -> bool {
    let dx = (x - 0.5).abs() - (0.5 - CORNER);
    let dy = (y - 0.5).abs() - (0.5 - CORNER);
    dx <= 0.0 || dy <= 0.0 || dx * dx + dy * dy <= CORNER * CORNER
}

fn ring_alpha(x: f32, y: f32) -> f32 {
    let (dx, dy) = (x - 0.5, y - 0.5);
    let radius = (dx * dx + dy * dy).sqrt();
    if !(RING_INNER..=RING_OUTER).contains(&radius) {
        return 0.0;
    }
    let turns = dx.atan2(-dy).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
    if turns <= FILLED_TURNS {
        1.0
    } else {
        TRACK_ALPHA
    }
}

pub fn rgba(size: u32) -> Vec<u8> {
    let step = 1.0 / (size * SAMPLES) as f32;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut color = [0.0f32; 3];
            let mut coverage = 0.0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = ((px * SAMPLES + sx) as f32 + 0.5) * step;
                    let y = ((py * SAMPLES + sy) as f32 + 0.5) * step;
                    if !in_rounded_square(x, y) {
                        continue;
                    }
                    let ring = ring_alpha(x, y);
                    for (channel, value) in color.iter_mut().enumerate() {
                        *value +=
                            BACKGROUND[channel] + (RING[channel] - BACKGROUND[channel]) * ring;
                    }
                    coverage += 1.0;
                }
            }
            let samples = (SAMPLES * SAMPLES) as f32;
            for value in color {
                pixels.push(if coverage > 0.0 {
                    (value / coverage).round() as u8
                } else {
                    0
                });
            }
            pixels.push((coverage / samples * 255.0).round() as u8);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_transparent_corners_an_orange_body_and_a_white_ring() {
        let size = 64;
        let pixels = rgba(size);
        let at = |x: u32, y: u32| {
            let i = ((y * size + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
        };
        assert_eq!(pixels.len(), (size * size * 4) as usize);
        assert_eq!(at(0, 0)[3], 0);
        assert_eq!(at(32, 32), [217, 119, 87, 255]);
        assert_eq!(at(32 + 17, 32), [255, 255, 255, 255]);
    }
}
