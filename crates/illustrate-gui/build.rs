use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

fn render_emblem_rgba(size: u32) -> Vec<u8> {
    let mut data = vec![0_u8; (size * size * 4) as usize];
    let scale = size as f32 / 128.0;

    // Draw a stylized Goodsell-inspired hemoglobin tetramer emblem with black ink outlines
    let spheres: [(f32, f32, f32, [u8; 3]); 4] = [
        (46.0 * scale, 46.0 * scale, 28.0 * scale, [255, 153, 153]),
        (82.0 * scale, 48.0 * scale, 28.0 * scale, [255, 204, 153]),
        (48.0 * scale, 82.0 * scale, 28.0 * scale, [255, 178, 128]),
        (82.0 * scale, 82.0 * scale, 28.0 * scale, [255, 138, 138]),
    ];
    let border = 3.2 * scale;

    for y in 0..size {
        for x in 0..size {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let mut best_z = -1.0_f32;
            let mut best_color = [0_u8; 4];

            for &(cx, cy, r, rgb) in &spheres {
                let dx = fx - cx;
                let dy = fy - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= r {
                    let z = (r * r - dist * dist).sqrt();
                    if z > best_z {
                        best_z = z;
                        if dist > r - border {
                            best_color = [26, 28, 36, 255];
                        } else {
                            best_color = [rgb[0], rgb[1], rgb[2], 255];
                        }
                    }
                }
            }

            let idx = ((y * size + x) * 4) as usize;
            data[idx..idx + 4].copy_from_slice(&best_color);
        }
    }

    data
}

fn encode_png(size: u32) -> Vec<u8> {
    let data = render_emblem_rgba(size);
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, size, size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut png_writer = encoder.write_header().expect("write png header");
        png_writer.write_image_data(&data).expect("write png data");
    }
    out
}

fn write_png_if_missing(path: &Path, size: u32) {
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let bytes = encode_png(size);
    fs::write(path, bytes).expect("write png icon");
}

fn write_ico_if_missing(path: &Path, size: u32) {
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let png_bytes = encode_png(size);
    let byte_dim = if size >= 256 { 0_u8 } else { size as u8 };
    let image_size = png_bytes.len() as u32;
    let offset = 6_u32 + 16_u32;

    let file = File::create(path).expect("create icon.ico");
    let mut writer = BufWriter::new(file);
    // ICONDIR header: reserved (0), type (1 = ICO), count (1)
    writer
        .write_all(&[0, 0, 1, 0, 1, 0])
        .expect("write ICO header");
    // ICONDIRENTRY (16 bytes) with embedded PNG image
    writer
        .write_all(&[
            byte_dim, // width
            byte_dim, // height
            0,        // color count
            0,        // reserved
        ])
        .expect("write ICO entry dimensions");
    writer
        .write_all(&1_u16.to_le_bytes())
        .expect("write ICO planes");
    writer
        .write_all(&32_u16.to_le_bytes())
        .expect("write ICO bit count");
    writer
        .write_all(&image_size.to_le_bytes())
        .expect("write ICO image size");
    writer
        .write_all(&offset.to_le_bytes())
        .expect("write ICO data offset");
    writer.write_all(&png_bytes).expect("write ICO png payload");
}

fn main() {
    println!("cargo:rerun-if-changed=icons");
    write_png_if_missing(Path::new("icons/32x32.png"), 32);
    write_png_if_missing(Path::new("icons/128x128.png"), 128);
    write_png_if_missing(Path::new("icons/128x128@2x.png"), 256);
    write_png_if_missing(Path::new("icons/icon.png"), 512);
    write_ico_if_missing(Path::new("icons/icon.ico"), 256);
    tauri_build::build();
}
