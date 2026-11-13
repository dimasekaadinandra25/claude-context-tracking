#[path = "src/icon.rs"]
mod icon;

use std::path::PathBuf;

const ICO_SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];
const BITMAP_HEADER_SIZE: u32 = 40;
const ICO_ENTRY_SIZE: usize = 16;
const ICO_HEADER_SIZE: usize = 6;

fn bitmap(size: u32) -> Vec<u8> {
    let rgba = icon::rgba(size);
    let mut out = Vec::new();
    for value in [BITMAP_HEADER_SIZE, size, size * 2] {
        out.extend(value.to_le_bytes());
    }
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend([0u8; 24]);
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend([rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    out.extend(vec![0u8; (size.div_ceil(32) * 4 * size) as usize]);
    out
}

fn ico() -> Vec<u8> {
    let images: Vec<Vec<u8>> = ICO_SIZES.iter().map(|&size| bitmap(size)).collect();
    let mut out = vec![0, 0, 1, 0];
    out.extend((ICO_SIZES.len() as u16).to_le_bytes());
    let mut offset = ICO_HEADER_SIZE + ICO_ENTRY_SIZE * ICO_SIZES.len();
    for (size, image) in ICO_SIZES.iter().zip(&images) {
        let side = u8::try_from(*size).unwrap_or(0);
        out.extend([side, side, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((image.len() as u32).to_le_bytes());
        out.extend((offset as u32).to_le_bytes());
        offset += image.len();
    }
    for image in images {
        out.extend(image);
    }
    out
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let path = out.join("app.ico");
    let icon = ico();
    std::fs::write(&path, &icon).expect("write app.ico");
    if let Some(profile_dir) = out.ancestors().nth(3) {
        let _ = std::fs::write(profile_dir.join("app.ico"), &icon);
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(&path.to_string_lossy());
    resource.compile().expect("embed the app icon");
}
