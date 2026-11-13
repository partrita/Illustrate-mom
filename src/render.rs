use crate::command::{CenterMode, CommandDeck};
use crate::error::{IllustrateError, Result};
use crate::framebuffer::FrameBuffer;
use crate::pdb::PdbStructure;
use crate::transform::BioMatrix;

/// Diagnostics and pixel buffers produced by [`render`].
#[derive(Debug, Clone)]
pub struct RenderResult {
    /// Horizontal width in pixels (`iysize`).
    pub width: usize,
    /// Vertical height in pixels (`ixsize`).
    pub height: usize,
    /// Row-major RGB pixel values (`width * height * 3`), quantized `0..=255`.
    pub rgb: Vec<u8>,
    /// Row-major grayscale-as-RGB opacity values (`width * height * 3`), quantized `0..=255`.
    pub opacity: Vec<u8>,
    /// Number of spheres rasterized per atom descriptor.
    pub spheres_per_type: Vec<usize>,
    /// Computed depth buffer minimum and maximum (`zpix_min`, `zpix_max`).
    pub z_range: (f32, f32),
}

/// Legacy-compatible renderer matching `illustrate.f` pixel-for-pixel.
pub fn render(deck: &CommandDeck, pdb: &PdbStructure) -> Result<RenderResult> {
    render_impl(deck, pdb, false)
}

/// Interactive wobble-free renderer using a rotation-invariant centroid & bounding sphere
/// plus sub-pixel continuous sphere rasterization.
pub fn render_smooth(deck: &CommandDeck, pdb: &PdbStructure) -> Result<RenderResult> {
    render_impl(deck, pdb, true)
}

/// Computes the exact pixel `(width, height)` produced by [`render_smooth`] for `deck` and `pdb`.
pub fn smooth_canvas_dimensions(deck: &CommandDeck, pdb: &PdbStructure) -> Result<(usize, usize)> {
    let default_biomat = [BioMatrix::identity()];
    let biomats: &[BioMatrix] = if pdb.biomats.is_empty() {
        &default_biomat
    } else {
        &pdb.biomats
    };
    let radius_max = deck
        .descriptors
        .iter()
        .map(|d| d.radius * deck.scale)
        .fold(0.0_f32, f32::max);
    let mut ixsize = deck.world.ixsize;
    let mut iysize = deck.world.iysize;

    if deck.center_mode != CenterMode::None && (ixsize <= 0 || iysize <= 0) {
        let (_centroid, r_bound) = assembly_centroid_and_radius(pdb, biomats);
        let span = 2.0 * r_bound * deck.scale;
        ixsize = (-2.0_f32 * (ixsize as f32) + 2.0_f32 * radius_max + span) as i32;
        iysize = (-2.0_f32 * (iysize as f32) + 2.0_f32 * radius_max + span) as i32;
    }

    ixsize = (ixsize.min(3000) / 2) * 2;
    iysize = (iysize.min(3000) / 2) * 2;
    if ixsize <= 0 || iysize <= 0 {
        return Err(IllustrateError::InvalidDimensions {
            width: iysize.max(0) as usize,
            height: ixsize.max(0) as usize,
        });
    }
    Ok((iysize as usize, ixsize as usize))
}

fn assembly_centroid_and_radius(pdb: &PdbStructure, biomats: &[BioMatrix]) -> ([f32; 3], f32) {
    let mut sum = [0.0_f64; 3];
    let mut count = 0_usize;
    for atom in pdb.atoms.iter().skip(1) {
        for biomat in biomats {
            let p = biomat.apply(atom.coord);
            sum[0] += p[0] as f64;
            sum[1] += p[1] as f64;
            sum[2] += p[2] as f64;
            count += 1;
        }
    }
    if count == 0 {
        return ([0.0; 3], 0.0);
    }
    let c = [
        (sum[0] / count as f64) as f32,
        (sum[1] / count as f64) as f32,
        (sum[2] / count as f64) as f32,
    ];
    let mut max_d2 = 0.0_f32;
    for atom in pdb.atoms.iter().skip(1) {
        for biomat in biomats {
            let p = biomat.apply(atom.coord);
            let dx = p[0] - c[0];
            let dy = p[1] - c[1];
            let dz = p[2] - c[2];
            let d2 = dx * dx + dy * dy + dz * dz;
            if d2 > max_d2 {
                max_d2 = d2;
            }
        }
    }
    (c, max_d2.sqrt())
}

fn render_impl(deck: &CommandDeck, pdb: &PdbStructure, smooth: bool) -> Result<RenderResult> {
    let default_biomat = [BioMatrix::identity()];
    let biomats: &[BioMatrix] = if pdb.biomats.is_empty() {
        &default_biomat
    } else {
        &pdb.biomats
    };

    // Populate conical shadow distance lookup table (-51..=51)
    let conemax = 50.0_f32;
    let mut rtable = [[0.0_f32; 103]; 103];
    for i in -51_i32..=51 {
        for j in -51_i32..=51 {
            let fi = i as f32;
            let fj = j as f32;
            let dist = (fi * fi + fj * fj).sqrt();
            rtable[(i + 51) as usize][(j + 51) as usize] =
                if dist > conemax { 10_000.0 } else { dist };
        }
    }
    rtable[51][51] = 10_000.0;

    // Color lookup table: index 0 is background color, 1..=ndes are descriptor colors
    let mut colortype = Vec::with_capacity(deck.descriptors.len() + 1);
    colortype.push(deck.world.rback);
    for desc in &deck.descriptors {
        colortype.push(desc.color);
    }

    // Scale atomic radii
    let mut radtype = Vec::with_capacity(deck.descriptors.len());
    let mut radius_max = 0.0_f32;
    for desc in &deck.descriptors {
        let scaled = desc.radius * deck.scale;
        if scaled > radius_max {
            radius_max = scaled;
        }
        radtype.push(scaled);
    }

    let mut xtranc = 0.0_f32;
    let mut ytranc = 0.0_f32;
    let mut ztranc = 0.0_f32;
    let mut ixsize = deck.world.ixsize;
    let mut iysize = deck.world.iysize;

    if deck.center_mode != CenterMode::None {
        if smooth {
            let (centroid, r_bound) = assembly_centroid_and_radius(pdb, biomats);
            let rc = deck.rotation.transform_vector(centroid);
            xtranc = -rc[0];
            ytranc = -rc[1];
            match deck.center_mode {
                CenterMode::Auto => ztranc = -rc[2] - r_bound - radius_max - 1.0,
                CenterMode::Center => ztranc = -rc[2],
                CenterMode::None => {}
            }
            if ixsize <= 0 || iysize <= 0 {
                let span = 2.0 * r_bound * deck.scale;
                ixsize = (-2.0_f32 * (ixsize as f32) + 2.0_f32 * radius_max + span) as i32;
                iysize = (-2.0_f32 * (iysize as f32) + 2.0_f32 * radius_max + span) as i32;
            }
        } else {
            let mut xmin = 10_000.0_f32;
            let mut xmax = -10_000.0_f32;
            let mut ymin = 10_000.0_f32;
            let mut ymax = -10_000.0_f32;
            let mut zmin = 10_000.0_f32;
            let mut zmax = -10_000.0_f32;

            for atom in pdb.atoms.iter().skip(1) {
                for biomat in biomats {
                    let r1 = biomat.apply(atom.coord);
                    let r2 = deck.rotation.transform_vector(r1);
                    xmin = xmin.min(r2[0]);
                    xmax = xmax.max(r2[0]);
                    ymin = ymin.min(r2[1]);
                    ymax = ymax.max(r2[1]);
                    zmin = zmin.min(r2[2]);
                    zmax = zmax.max(r2[2]);
                }
            }

            xtranc = -xmin - (xmax - xmin) / 2.0;
            ytranc = -ymin - (ymax - ymin) / 2.0;
            match deck.center_mode {
                CenterMode::Auto => ztranc = -zmax - radius_max - 1.0,
                CenterMode::Center => ztranc = -zmin - (zmax - zmin) / 2.0,
                CenterMode::None => {}
            }

            if ixsize <= 0 || iysize <= 0 {
                ixsize = (-2.0_f32 * (ixsize as f32)
                    + 2.0_f32 * radius_max
                    + (xmax - xmin) * deck.scale) as i32;
                iysize = (-2.0_f32 * (iysize as f32)
                    + 2.0_f32 * radius_max
                    + (ymax - ymin) * deck.scale) as i32;
            }
        }
    }

    ixsize = (ixsize.min(3000) / 2) * 2;
    iysize = (iysize.min(3000) / 2) * 2;
    if ixsize <= 0 || iysize <= 0 {
        return Err(IllustrateError::InvalidDimensions {
            width: iysize.max(0) as usize,
            height: ixsize.max(0) as usize,
        });
    }

    let mut fb = FrameBuffer::new(ixsize, iysize);
    let mut spheres_per_type = Vec::with_capacity(radtype.len());

    if pdb.atom_count() > 0 {
        for (irad_idx, &rad) in radtype.iter().enumerate() {
            let type_id = irad_idx + 1;
            let irlim = rad as i32;
            if irlim > 100 {
                return Err(IllustrateError::RadiusTooLarge { scaled_radius: rad });
            }

            let sphdat: Vec<[f32; 3]> = if smooth {
                Vec::new()
            } else {
                let mut table = Vec::new();
                for ix in (-irlim - 1)..=(irlim + 1) {
                    for iy in (-irlim - 1)..=(irlim + 1) {
                        let x = ix as f32;
                        let y = iy as f32;
                        let d = (x * x + y * y).sqrt();
                        if d > rad {
                            continue;
                        }
                        let z = (rad * rad - d * d).sqrt();
                        table.push([x, y, z]);
                    }
                }
                table
            };

            let r2_rad = rad * rad;
            let mut icount = 0usize;
            for (ia, atom) in pdb.atoms.iter().enumerate().skip(1) {
                if atom.type_id != type_id {
                    continue;
                }
                icount += 1;

                for (ibio_idx, biomat) in biomats.iter().enumerate() {
                    let ibio = (ibio_idx + 1) as i32;
                    let r1 = biomat.apply(atom.coord);
                    let r2 = deck.rotation.transform_vector(r1);
                    let rx2 = (r2[0] + xtranc + deck.translation[0]) * deck.scale;
                    let ry2 = (r2[1] + ytranc + deck.translation[1]) * deck.scale;
                    let rz2 = (r2[2] + ztranc + deck.translation[2]) * deck.scale;

                    if rz2 < 0.0 {
                        if smooth {
                            let cx = rx2 + (ixsize as f32) / 2.0;
                            let cy = ry2 + (iysize as f32) / 2.0;
                            let ix_min = ((cx - rad).floor() as i32).max(1);
                            let ix_max = ((cx + rad).ceil() as i32).min(ixsize);
                            let iy_min = ((cy - rad).floor() as i32).max(1);
                            let iy_max = ((cy + rad).ceil() as i32).min(iysize);
                            for ix in ix_min..=ix_max {
                                let dx = (ix as f32) - cx;
                                let dx2 = dx * dx;
                                for iy in iy_min..=iy_max {
                                    let dy = (iy as f32) - cy;
                                    let d2 = dx2 + dy * dy;
                                    if d2 <= r2_rad {
                                        let z = (r2_rad - d2).sqrt() + rz2;
                                        if z > fb.z(ix, iy) {
                                            fb.set_z(ix, iy, z);
                                            fb.set_atom(ix, iy, ia);
                                            fb.set_bio(ix, iy, ibio);
                                        }
                                    }
                                }
                            }
                        } else {
                            for &[sx, sy, sz] in &sphdat {
                                let x = sx + rx2 + (ixsize as f32) / 2.0;
                                let y = sy + ry2 + (iysize as f32) / 2.0;
                                let ix = x as i32;
                                let iy = y as i32;
                                if x > (ixsize as f32) || x < 1.0 || y > (iysize as f32) || y < 1.0
                                {
                                    continue;
                                }
                                let z = sz + rz2;
                                if z > fb.z(ix, iy) {
                                    fb.set_z(ix, iy, z);
                                    fb.set_atom(ix, iy, ia);
                                    fb.set_bio(ix, iy, ibio);
                                }
                            }
                        }
                    }
                }
            }

            spheres_per_type.push(icount);
        }
    }

    // Find z-buffer range and clamp zpix <= 0.0
    let mut zpix_max = -100_000.0_f32;
    let mut zpix_min = 100_000.0_f32;
    for ix in 1..=ixsize {
        for iy in 1..=iysize {
            let zv = fb.z(ix, iy);
            if zv > zpix_max {
                zpix_max = zv;
            }
            if zv != -10_000.0 && zv < zpix_min {
                zpix_min = zv;
            }
        }
    }
    zpix_max = zpix_max.min(0.0);
    let zpix_spread = zpix_max - zpix_min;

    for ix in 1..=ixsize {
        for iy in 1..=iysize {
            fb.set_z(ix, iy, fb.z(ix, iy).min(0.0));
        }
    }

    let l_diff_min = deck.outline.l_diff_min * deck.scale;
    let l_diff_max = deck.outline.l_diff_max * deck.scale;
    let pfogdiff = deck.world.pfogh - deck.world.pfogl;

    // Preserved as outer state matching Fortran `real*4 l(-1:1,-1:1)`
    let mut l_buf = [[0.0_f32; 3]; 3];

    let width = iysize as usize;
    let height = ixsize as usize;
    let mut rgb = Vec::with_capacity(width * height * 3);
    let mut opacity = Vec::with_capacity(width * height * 3);

    for ix in 1..=ixsize {
        for iy in 1..=iysize {
            let atom_idx = fb.atom(ix, iy);
            let z_here = fb.z(ix, iy);

            // Conical shadows
            let mut pconetot = 1.0_f32;
            if deck.world.icone != 0 && atom_idx != 0 {
                for i in (-50_i32..=50).step_by(5) {
                    for j in (-50_i32..=50).step_by(5) {
                        if ix + i > 0 && ix + i < ixsize && iy + j > 0 && iy + j < iysize {
                            let rzdiff = fb.z(ix + i, iy + j) - z_here;
                            if rzdiff > deck.world.rcone
                                && rtable[(i + 51) as usize][(j + 51) as usize]
                                    * deck.world.coneangle
                                    < rzdiff + deck.world.rcone
                            {
                                pconetot -= deck.world.pcone;
                            }
                        }
                    }
                }
                pconetot = pconetot.max(deck.world.pshadowmax);
            }

            // Depth cueing (fog)
            let mut pfh = deck.world.pfogh - (zpix_max - z_here) / zpix_spread * pfogdiff;
            if z_here < zpix_min {
                pfh = 1.0;
            }

            // Outlines (subunit, residue, contour)
            let mut l_opacity = 0.0_f32;
            if deck.outline.enabled {
                let mut g_opacity = 0.0_f32;
                if ix > 1 && ix < ixsize && iy > 1 && iy < iysize {
                    let su_here = pdb.atoms[atom_idx].su;
                    let bio_here = fb.bio(ix, iy);
                    let res_here = pdb.atoms[atom_idx].res;
                    let mut g = 0.0_f32;
                    let mut r = 0.0_f32;

                    for i in -2_i32..=2 {
                        for j in -2_i32..=2 {
                            if (i * j).abs() != 4 {
                                let neighbor_atom = fb.atom(ix + i, iy + j);
                                if su_here != pdb.atoms[neighbor_atom].su
                                    || bio_here != fb.bio(ix + i, iy + j)
                                {
                                    r += 1.0;
                                }
                                if ((res_here - pdb.atoms[neighbor_atom].res).abs() as f32)
                                    > deck.outline.resdiff
                                {
                                    g += 1.0;
                                }
                            }
                        }
                    }

                    g_opacity = ((g - deck.outline.g_low)
                        / (deck.outline.g_high - deck.outline.g_low))
                        .min(1.0);
                    let r_opacity = ((r - deck.outline.r_low)
                        / (deck.outline.r_high - deck.outline.r_low))
                        .min(1.0);
                    g_opacity = g_opacity.max(r_opacity).max(0.0);
                }

                if ix > 2 && ix < ixsize - 1 && iy > 2 && iy < iysize - 1 {
                    let mut rl = 0.0_f32;
                    let mut l_opacity_ave = 0.0_f32;

                    for ixl in -1_i32..=1 {
                        for iyl in -1_i32..=1 {
                            let ixc = ix + ixl;
                            let iyc = iy + iyl;
                            let lx = (ixl + 1) as usize;
                            let ly = (iyl + 1) as usize;

                            match deck.outline.ikernel {
                                1 => {
                                    l_buf[lx][ly] = (1.0_f32 / 3.0_f32
                                        * (-0.8 * fb.z(ixc - 1, iyc - 1)
                                            - 1.0 * fb.z(ixc - 1, iyc)
                                            - 0.8 * fb.z(ixc - 1, iyc + 1)
                                            - 1.0 * fb.z(ixc, iyc - 1)
                                            + 7.2 * fb.z(ixc, iyc)
                                            - 1.0 * fb.z(ixc, iyc + 1)
                                            - 0.8 * fb.z(ixc + 1, iyc - 1)
                                            - 1.0 * fb.z(ixc + 1, iyc)
                                            - 0.8 * fb.z(ixc + 1, iyc + 1)))
                                    .abs();
                                }
                                2 => {
                                    l_buf[lx][ly] = (1.0_f32 / 3.0_f32
                                        * (-0.8 * fb.z(ixc - 1, iyc - 1)
                                            - 1.0 * fb.z(ixc - 1, iyc)
                                            - 0.8 * fb.z(ixc - 1, iyc + 1)
                                            - 1.0 * fb.z(ixc, iyc - 1)
                                            + 8.8 * fb.z(ixc, iyc)
                                            - 1.0 * fb.z(ixc, iyc + 1)
                                            - 0.8 * fb.z(ixc + 1, iyc - 1)
                                            - 1.0 * fb.z(ixc + 1, iyc)
                                            - 0.8 * fb.z(ixc + 1, iyc + 1)
                                            - 0.1 * fb.z(ixc + 2, iyc - 1)
                                            - 0.2 * fb.z(ixc + 2, iyc)
                                            - 0.1 * fb.z(ixc + 2, iyc + 1)
                                            - 0.1 * fb.z(ixc - 2, iyc - 1)
                                            - 0.2 * fb.z(ixc - 2, iyc)
                                            - 0.1 * fb.z(ixc - 2, iyc + 1)
                                            - 0.1 * fb.z(ixc - 1, iyc + 2)
                                            - 0.2 * fb.z(ixc, iyc + 2)
                                            - 0.1 * fb.z(ixc + 1, iyc + 2)
                                            - 0.1 * fb.z(ixc - 1, iyc - 2)
                                            - 0.2 * fb.z(ixc, iyc - 2)
                                            - 0.1 * fb.z(ixc + 1, iyc - 2)))
                                    .abs();
                                }
                                3 => {
                                    for i in -1_i32..=1 {
                                        for j in -1_i32..=1 {
                                            let mut rd = (z_here - fb.z(ix + i, iy + j)).abs();
                                            if rd > l_diff_min {
                                                rd = (rd - l_diff_min) / (l_diff_max - l_diff_min);
                                                l_buf[lx][ly] += rd.min(1.0);
                                            }
                                        }
                                    }
                                }
                                4 => {
                                    for i in -2_i32..=2 {
                                        for j in -2_i32..=2 {
                                            if (i * j).abs() != 4 {
                                                let mut rd = (z_here - fb.z(ix + i, iy + j)).abs();
                                                if rd > l_diff_min {
                                                    rd = (rd - l_diff_min)
                                                        / (l_diff_max - l_diff_min);
                                                    l_buf[lx][ly] += rd.min(1.0);
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }

                            #[allow(clippy::manual_clamp)]
                            {
                                l_buf[lx][ly] = ((l_buf[lx][ly] - deck.outline.l_low)
                                    / (deck.outline.l_high - deck.outline.l_low))
                                    .min(1.0)
                                    .max(0.0);
                            }
                            if l_buf[lx][ly] > 0.0 {
                                rl += 1.0;
                            }
                            l_opacity_ave += l_buf[lx][ly];
                        }
                    }

                    l_opacity = if rl >= 6.0 {
                        l_opacity_ave / 6.0
                    } else {
                        l_buf[1][1]
                    };
                    #[allow(clippy::manual_clamp)]
                    {
                        l_opacity = l_opacity.min(1.0).max(0.0);
                    }
                }

                l_opacity = l_opacity.max(g_opacity);
            }

            // Final pixel intensity and opacity
            let type_id = pdb.atoms[atom_idx].type_id;
            let base_color = colortype[type_id];
            let ropacity = if type_id != 0 { 1.0_f32 } else { 0.0_f32 };
            let mut rgba = [0.0_f32; 4];

            for ic in 0..3 {
                let rcolor = pfh * (pconetot * base_color[ic]) + (1.0 - pfh) * deck.world.rfog[ic];
                rgba[ic] = (1.0 - l_opacity) * rcolor;
            }
            rgba[3] = ropacity.max(l_opacity);
            fb.set_pixel(ix, iy, rgba);

            for channel in &rgba[0..3] {
                rgb.push(((*channel * 255.0_f32) as i32).clamp(0, 255) as u8);
            }
            let op_q = ((rgba[3] * 255.0_f32) as i32).clamp(0, 255) as u8;
            opacity.push(op_q);
            opacity.push(op_q);
            opacity.push(op_q);
        }
    }

    Ok(RenderResult {
        width,
        height,
        rgb,
        opacity,
        spheres_per_type,
        z_range: (zpix_min, zpix_max),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdb::parse_pdb_reader;

    #[test]
    fn renders_single_atom_with_kernels_1_to_4() {
        for kernel in 1..=4 {
            let cmd = format!(
                "\
read
single.pdb
ATOM  -C-------- 0,9999, 1.0,0.5,0.2, 2.0
END
center
auto
scale
10.0
wor
1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0
1,0.0023,2.0,1.0,0.2
-10,-10
illustrate
3.0,10.0,{kernel},0.0,5.0
3.0,10.0
3.0,8.0,6000.
calculate
out.pnm
"
            );
            let deck = CommandDeck::parse(&cmd).unwrap();
            let pdb_src =
                "ATOM      1  CA  ALA A   1       0.000   0.000   0.000  1.00 10.00           C\n";
            let pdb = parse_pdb_reader(pdb_src.as_bytes(), &deck.descriptors).unwrap();
            let result = render(&deck, &pdb).unwrap();

            assert!(result.width > 20 && result.height > 20);
            assert_eq!(result.rgb.len(), result.width * result.height * 3);
            assert_eq!(result.opacity.len(), result.width * result.height * 3);
            assert_eq!(result.spheres_per_type, vec![1]);
            // Center pixel should have atom opacity 255
            let cy = result.height / 2;
            let cx = result.width / 2;
            let op_idx = (cy * result.width + cx) * 3;
            assert_eq!(result.opacity[op_idx], 255);
        }
    }

    #[test]
    fn renders_biomt_copies_fog_and_conical_shadows() {
        let cmd = "\
read
dimer.pdb
ATOM  -C-------A 0,9999, 0.9,0.2,0.2, 2.0
END
center
cen
trans
0.0,0.0,0.0
scale
10.0
wor
1.0,1.0,1.0, 0.0,0.0,1.0, 0.9,0.3
1,0.05,1.0,0.2,0.4
-12,-12
illustrate
3.0,10.0,1,0.0,5.0
1.0,4.0
1.0,4.0,1.0
calculate
dimer.pnm
";
        let deck = CommandDeck::parse(cmd).unwrap();
        let pdb_src = "\
REMARK 350 BIOMOLECULE: 1
REMARK 350 APPLY THE FOLLOWING TO CHAINS: A
REMARK 350   BIOMT1   1  1.000000  0.000000  0.000000        0.00000
REMARK 350   BIOMT2   1  0.000000  1.000000  0.000000        0.00000
REMARK 350   BIOMT3   1  0.000000  0.000000  1.000000        0.00000
REMARK 350   BIOMT1   2  1.000000  0.000000  0.000000        0.00000
REMARK 350   BIOMT2   2  0.000000  1.000000  0.000000        4.00000
REMARK 350   BIOMT3   2  0.000000  0.000000  1.000000       -3.00000
REMARK 400
ATOM      1  CA  ALA A   1       0.000   0.000   0.000  1.00 10.00           C
";
        let pdb = parse_pdb_reader(pdb_src.as_bytes(), &deck.descriptors).unwrap();
        assert_eq!(pdb.biomats.len(), 2);

        let with_shadow = render(&deck, &pdb).unwrap();

        let mut no_shadow_deck = deck.clone();
        no_shadow_deck.world.icone = 0;
        let without_shadow = render(&no_shadow_deck, &pdb).unwrap();

        assert_ne!(
            with_shadow.rgb, without_shadow.rgb,
            "conical shadow should darken occluded regions"
        );
        // Fog should introduce non-zero blue channel tint on deep spheres (rfog = [0,0,1])
        let has_blue_fog_tint = with_shadow
            .rgb
            .as_chunks::<3>()
            .0
            .iter()
            .any(|px| px[2] > 0 && px[0] < 255);
        assert!(has_blue_fog_tint);
    }

    #[test]
    fn explicit_dimensions_work_without_autocenter() {
        let cmd = "\
read
manual.pdb
ATOM  -C-------- 0,9999, 0.8,0.4,0.2, 1.5
END
trans
0.0,0.0,-5.0
scale
8.0
wor
1.0,1.0,1.0, 1.0,1.0,1.0, 1.0,1.0
0,0.0,1.0,1.0,1.0
64,48
calculate
manual.pnm
";
        let deck = CommandDeck::parse(cmd).unwrap();
        let pdb_src =
            "ATOM      1  CA  ALA A   1       0.000   0.000   0.000  1.00 10.00           C\n";
        let pdb = parse_pdb_reader(pdb_src.as_bytes(), &deck.descriptors).unwrap();
        let res = render(&deck, &pdb).unwrap();
        assert_eq!((res.width, res.height), (48, 64));
    }

    #[test]
    fn render_smooth_keeps_constant_dimensions_across_rotations() {
        let cmd = "\
read
asym.pdb
ATOM  -C-------- 0,9999, 0.9,0.5,0.4, 1.5
END
center
auto
scale
10.0
wor
1.0,1.0,1.0, 1.0,1.0,1.0, 1.0,1.0
0,0.0,1.0,1.0,1.0
-20,-20
calculate
asym.pnm
";
        let mut deck = CommandDeck::parse(cmd).unwrap();
        let pdb_src = "\
ATOM      1  CA  ALA A   1      -6.000   0.000   0.000  1.00 10.00           C
ATOM      2  CA  ALA A   2       6.000   1.000   0.000  1.00 10.00           C
ATOM      3  CA  ALA A   3       0.000   0.500   2.000  1.00 10.00           C
";
        let pdb = parse_pdb_reader(pdb_src.as_bytes(), &deck.descriptors).unwrap();

        let base = render_smooth(&deck, &pdb).unwrap();
        assert_eq!(
            (base.width, base.height),
            smooth_canvas_dimensions(&deck, &pdb).unwrap()
        );

        for angles in [
            [0.0_f32, 0.0, 45.0],
            [35.0, -50.0, 90.0],
            [90.0, 25.0, -120.0],
        ] {
            deck.rotation =
                crate::transform::Matrix4::from_euler_angles_deg(angles[0], angles[1], angles[2]);
            let rotated = render_smooth(&deck, &pdb).unwrap();
            assert_eq!((rotated.width, rotated.height), (base.width, base.height));
        }
    }
}
