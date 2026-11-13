use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::error::Result;
use crate::render::RenderResult;

/// Writes a buffer of row-major RGB `u8` values (`width * height * 3`) in Netpbm P3 ASCII PPM
/// format matching Fortran `illustrate.f` (`1003`/`1004`/`1002` `20i4` formatting).
pub fn write_ppm_p3<W: Write>(writer: W, width: usize, height: usize, rgb: &[u8]) -> Result<()> {
    let mut out = BufWriter::new(writer);
    writeln!(out, "P3")?;
    writeln!(out, "{width:5}{height:5}")?;
    writeln!(out, "{:5}", 255)?;

    let row_len = width * 3;
    for row in rgb.chunks_exact(row_len) {
        for chunk in row.chunks(20) {
            for &val in chunk {
                write!(out, "{val:4}")?;
            }
            writeln!(out)?;
        }
    }

    out.flush()?;
    Ok(())
}

/// Writes an RGB `u8` buffer to a `.pnm` / `.ppm` file on disk.
pub fn save_ppm_file(path: &Path, width: usize, height: usize, rgb: &[u8]) -> Result<()> {
    let file = File::create(path)?;
    write_ppm_p3(file, width, height, rgb)
}

/// Encodes an RGBA PNG image in memory combining [`RenderResult::rgb`] with alpha from
/// [`RenderResult::opacity`].
pub fn encode_png_rgba(result: &RenderResult) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, result.width as u32, result.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut png_writer = encoder.write_header()?;

        let pixel_count = result.width * result.height;
        let mut rgba = Vec::with_capacity(pixel_count * 4);
        for i in 0..pixel_count {
            let base = i * 3;
            rgba.push(result.rgb[base]);
            rgba.push(result.rgb[base + 1]);
            rgba.push(result.rgb[base + 2]);
            rgba.push(result.opacity[base]);
        }
        png_writer.write_image_data(&rgba)?;
    }
    Ok(out)
}

/// Encodes an RGB PNG image in memory from row-major `rgb` bytes (`width * height * 3`).
pub fn encode_png_rgb(width: usize, height: usize, rgb: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut png_writer = encoder.write_header()?;
        png_writer.write_image_data(rgb)?;
    }
    Ok(out)
}

/// Writes an RGBA PNG image combining [`RenderResult::rgb`] with the alpha channel from
/// [`RenderResult::opacity`].
pub fn save_png_file(path: &Path, result: &RenderResult) -> Result<()> {
    let bytes = encode_png_rgba(result)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Parses a Netpbm P3 ASCII PPM file into `(width, height, rgb_values)` for verification.
pub fn parse_ppm_p3(text: &str) -> Result<(usize, usize, Vec<u8>)> {
    let mut tokens = text.lines().flat_map(|line| {
        let clean = match line.find('#') {
            Some(idx) => &line[..idx],
            None => line,
        };
        clean.split_whitespace()
    });

    let magic = tokens.next().unwrap_or_default();
    if magic != "P3" {
        return Err(crate::error::IllustrateError::InvalidCommand {
            line: 1,
            message: format!("expected P3 magic header, got '{magic}'"),
        });
    }

    let width: usize = tokens.next().and_then(|t| t.parse().ok()).ok_or_else(|| {
        crate::error::IllustrateError::InvalidCommand {
            line: 2,
            message: "invalid PPM width".into(),
        }
    })?;
    let height: usize = tokens.next().and_then(|t| t.parse().ok()).ok_or_else(|| {
        crate::error::IllustrateError::InvalidCommand {
            line: 2,
            message: "invalid PPM height".into(),
        }
    })?;
    let _maxval: usize = tokens.next().and_then(|t| t.parse().ok()).ok_or_else(|| {
        crate::error::IllustrateError::InvalidCommand {
            line: 3,
            message: "invalid PPM maxval".into(),
        }
    })?;

    let mut data = Vec::with_capacity(width * height * 3);
    for tok in tokens {
        let val: u8 = tok
            .parse()
            .map_err(|_| crate::error::IllustrateError::InvalidCommand {
                line: 4,
                message: format!("invalid PPM pixel value '{tok}'"),
            })?;
        data.push(val);
    }

    Ok((width, height, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_decodes_valid_rgba_png() {
        let res = RenderResult {
            width: 2,
            height: 2,
            rgb: vec![
                255, 0, 0, 0, 255, 0, // row 0
                0, 0, 255, 255, 255, 255, // row 1
            ],
            opacity: vec![
                255, 255, 255, 128, 128, 128, // row 0
                64, 64, 64, 0, 0, 0, // row 1
            ],
            spheres_per_type: vec![1],
            z_range: (-10.0, -1.0),
        };

        let tmp =
            std::env::temp_dir().join(format!("illustrate_png_test_{}.png", std::process::id()));
        save_png_file(&tmp, &res).expect("save_png_file");

        let file = File::open(&tmp).unwrap();
        let decoder = png::Decoder::new(file);
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();

        assert_eq!(info.width, 2);
        assert_eq!(info.height, 2);
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(&buf[0..8], &[255, 0, 0, 255, 0, 255, 0, 128]);

        let _ = std::fs::remove_file(&tmp);
    }
}
