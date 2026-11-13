use std::fmt::Write as _;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{IllustrateError, Result};
use crate::transform::Matrix4;

/// A single atom selection and rendering rule parsed from a `read` command block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomDescriptor {
    /// 6-character PDB record prefix (`"ATOM  "` or `"HETATM"`), matched against columns 1..=6.
    pub record_prefix: [u8; 6],
    /// 10-character pattern matched against PDB columns 13..=22 (`'-'` acts as wildcard).
    pub pattern: [u8; 10],
    /// Inclusive residue sequence number range `(low, high)`.
    pub res_range: (i32, i32),
    /// RGB base color in `[0.0, 1.0]`.
    pub color: [f32; 3],
    /// Atomic radius in Angstroms (`0.0` omits matching atoms).
    pub radius: f32,
}

impl AtomDescriptor {
    /// Builds an [`AtomDescriptor`] from UTF-8 record prefix (`"ATOM  "`/`"HETATM"`) and
    /// 10-character wildcard pattern strings (padded or truncated to fixed widths).
    pub fn from_strings(
        record: &str,
        pattern: &str,
        res_range: (i32, i32),
        color: [f32; 3],
        radius: f32,
    ) -> Result<Self> {
        let rec_padded = format!("{record:<6}");
        let rec_bytes = rec_padded.as_bytes();
        if rec_bytes.len() < 6 {
            return Err(IllustrateError::InvalidDescriptor {
                line: 0,
                message: format!("invalid record prefix '{record}'"),
            });
        }
        let mut record_prefix = [b' '; 6];
        record_prefix.copy_from_slice(&rec_bytes[0..6]);

        let pat_padded = format!("{pattern:-<10}");
        let pat_bytes = pat_padded.as_bytes();
        let mut pat_arr = [b'-'; 10];
        pat_arr.copy_from_slice(&pat_bytes[0..10]);

        Ok(Self {
            record_prefix,
            pattern: pat_arr,
            res_range,
            color,
            radius,
        })
    }

    /// Returns the 6-character PDB record prefix as a string.
    #[must_use]
    pub fn record_prefix_str(&self) -> String {
        String::from_utf8_lossy(&self.record_prefix).into_owned()
    }

    /// Returns the 10-character wildcard descriptor pattern as a string.
    #[must_use]
    pub fn pattern_str(&self) -> String {
        String::from_utf8_lossy(&self.pattern).into_owned()
    }

    /// Matches a PDB line (padded to at least 80 ASCII bytes) and parsed residue number.
    #[must_use]
    pub fn matches(&self, pdb_line: &[u8], residue_number: i32) -> bool {
        if pdb_line.len() < 26 {
            return false;
        }
        if pdb_line[0..6] != self.record_prefix {
            return false;
        }
        for (idx, &pat_byte) in self.pattern.iter().enumerate() {
            if pat_byte != b'-' && pdb_line[12 + idx] != pat_byte {
                return false;
            }
        }
        residue_number >= self.res_range.0 && residue_number <= self.res_range.1
    }
}

/// Centering strategy for molecular coordinates prior to translation and scaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CenterMode {
    /// No automatic centering (`autocenter = 0`).
    #[default]
    None,
    /// Center X/Y on bounding box midpoint and place frontmost atom just behind `z = 0` (`autocenter = 1`).
    Auto,
    /// Center X/Y/Z on bounding box midpoint (`autocenter = 2`).
    Center,
}

/// World environment, fog, soft conical shadow, and image sizing parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldParams {
    /// Background RGB color (`0.0..=1.0`).
    pub rback: [f32; 3],
    /// Fog RGB color (`0.0..=1.0`).
    pub rfog: [f32; 3],
    /// Fractional transparency of fog at front of molecule (`1.0` = no fog).
    pub pfogh: f32,
    /// Fractional transparency of fog at back of molecule (`1.0` = no fog).
    pub pfogl: f32,
    /// Enable conical soft shadows when non-zero.
    pub icone: i32,
    /// Fractional shadowing contribution per sample (`pcone`).
    pub pcone: f32,
    /// Conical shadow angle factor (`coneangle`).
    pub coneangle: f32,
    /// Minimum z-difference threshold in Angstroms for conical shadows (`rcone`).
    pub rcone: f32,
    /// Minimum brightness multiplier (maximum shadow darkness) (`pshadowmax`).
    pub pshadowmax: f32,
    /// Requested vertical image size (rows, `ixsize`); negative triggers autosizing with padding.
    pub ixsize: i32,
    /// Requested horizontal image size (cols, `iysize`); negative triggers autosizing with padding.
    pub iysize: i32,
}

impl Default for WorldParams {
    fn default() -> Self {
        Self {
            rback: [0.5, 0.5, 0.5],
            rfog: [1.0, 1.0, 1.0],
            pfogh: 1.0,
            pfogl: 1.0,
            icone: 0,
            pcone: 0.0,
            coneangle: 0.0,
            rcone: 0.0,
            pshadowmax: 1.0,
            ixsize: 0,
            iysize: 0,
        }
    }
}

/// Outline rendering parameters configured by the `illustrate` command card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlineParams {
    /// Whether outline rendering is enabled (`illustrationflag != 0`).
    pub enabled: bool,
    /// Contour outline lower and upper thresholds.
    pub l_low: f32,
    pub l_high: f32,
    /// Derivative kernel selector (`1..=4`).
    pub ikernel: i32,
    /// Range of z-differences (Angstroms before scaling) considered by kernels 3 and 4.
    pub l_diff_min: f32,
    pub l_diff_max: f32,
    /// Subunit outline lower and upper thresholds.
    pub r_low: f32,
    pub r_high: f32,
    /// Residue outline lower and upper thresholds.
    pub g_low: f32,
    pub g_high: f32,
    /// Minimum residue number difference required to draw residue outlines.
    pub resdiff: f32,
}

impl Default for OutlineParams {
    fn default() -> Self {
        Self {
            enabled: false,
            l_low: 1.0,
            l_high: 10.0,
            ikernel: 0,
            l_diff_min: 1.0,
            l_diff_max: 50.0,
            r_low: 0.0,
            r_high: 0.0,
            g_low: 350_000.0,
            g_high: 21_000.0,
            resdiff: 0.0,
        }
    }
}

/// Complete parsed command deck ready for PDB loading and rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandDeck {
    /// Path to the PDB input file specified by `read`.
    pub pdb_path: PathBuf,
    /// Ordered atom selection/rendering descriptors.
    pub descriptors: Vec<AtomDescriptor>,
    /// Cumulative translation vector `(x, y, z)` in Angstroms.
    pub translation: [f32; 3],
    /// Cumulative 4x4 rotation matrix.
    pub rotation: Matrix4,
    /// Cumulative scale factor in pixels per Angstrom.
    pub scale: f32,
    /// Coordinate centering mode.
    pub center_mode: CenterMode,
    /// World, fog, shadow, and image size settings.
    pub world: WorldParams,
    /// Outline kernel and threshold settings.
    pub outline: OutlineParams,
    /// Output PPM filename specified by `calculate` (if any).
    pub output_path: Option<PathBuf>,
}

/// Strips inline `#` comments and trims surrounding whitespace.
fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(idx) => line[..idx].trim(),
        None => line.trim(),
    }
}

/// Parses comma- and/or whitespace-separated tokens (Fortran list-directed `*` style).
fn parse_tokens(line: &str) -> Vec<&str> {
    strip_comment(line)
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|tok| !tok.is_empty())
        .collect()
}

fn parse_f32(tok: &str, line_no: usize) -> Result<f32> {
    tok.parse::<f32>()
        .map_err(|e| IllustrateError::InvalidCommand {
            line: line_no,
            message: format!("expected float, got '{tok}': {e}"),
        })
}

fn parse_i32(tok: &str, line_no: usize) -> Result<i32> {
    tok.parse::<i32>()
        .map_err(|e| IllustrateError::InvalidCommand {
            line: line_no,
            message: format!("expected integer, got '{tok}': {e}"),
        })
}

fn next_non_empty_line<'a>(lines: &'a [String], idx: &mut usize) -> Option<(usize, &'a str)> {
    while *idx < lines.len() {
        let current = *idx;
        *idx += 1;
        let cleaned = strip_comment(&lines[current]);
        if !cleaned.is_empty() {
            return Some((current + 1, lines[current].as_str()));
        }
    }
    None
}

fn is_known_command_prefix(prefix: &str) -> bool {
    matches!(
        prefix,
        "rea" | "tra" | "xro" | "yro" | "zro" | "sca" | "cen" | "wor" | "cal" | "ill"
    )
}

impl CommandDeck {
    /// Parses a Fortran-compatible `illustrate` command stream.
    pub fn parse(input: &str) -> Result<Self> {
        let lines: Vec<String> = input
            .lines()
            .map(|l| l.trim_end_matches('\r').to_string())
            .collect();

        let mut idx = 0usize;
        let mut pdb_path: Option<PathBuf> = None;
        let mut descriptors = Vec::new();
        let mut translation = [0.0_f32; 3];
        let mut rotation = Matrix4::identity();
        let mut scale = 1.0_f32;
        let mut center_mode = CenterMode::None;
        let mut world = WorldParams::default();
        let mut outline = OutlineParams::default();
        let mut output_path: Option<PathBuf> = None;

        while idx < lines.len() {
            let line_no = idx + 1;
            let raw = &lines[idx];
            idx += 1;

            let stripped = strip_comment(raw);
            if stripped.is_empty() {
                continue;
            }

            let cmd: String = stripped.chars().take(3).collect();
            if cmd.len() < 3 {
                return Err(IllustrateError::InvalidCommand {
                    line: line_no,
                    message: format!("unknown command card '{stripped}'"),
                });
            }

            match cmd.as_str() {
                "rea" => {
                    let (file_line_no, file_line) = next_non_empty_line(&lines, &mut idx)
                        .ok_or_else(|| IllustrateError::InvalidCommand {
                            line: line_no,
                            message: "expected PDB filename after 'read'".into(),
                        })?;
                    let pdb_str = strip_comment(file_line);
                    if pdb_str.is_empty() {
                        return Err(IllustrateError::InvalidCommand {
                            line: file_line_no,
                            message: "empty PDB filename".into(),
                        });
                    }
                    pdb_path = Some(PathBuf::from(pdb_str));
                    descriptors.clear();

                    while idx < lines.len() {
                        let d_line_no = idx + 1;
                        let d_raw = &lines[idx];
                        let d_stripped = strip_comment(d_raw);
                        if d_stripped.is_empty() {
                            idx += 1;
                            continue;
                        }
                        if d_raw.starts_with("END") || d_stripped.starts_with("END") {
                            idx += 1;
                            break;
                        }
                        if !d_raw.starts_with("ATOM  ") && !d_raw.starts_with("HETATM") {
                            let prefix: String = d_stripped.chars().take(3).collect();
                            if !is_known_command_prefix(&prefix) {
                                idx += 1;
                            }
                            break;
                        }
                        idx += 1;
                        descriptors.push(parse_descriptor_line(d_raw, d_line_no)?);
                    }
                }
                "tra" => {
                    let (p_no, p_line) =
                        next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                            IllustrateError::InvalidCommand {
                                line: line_no,
                                message: "expected x,y,z translation vector".into(),
                            }
                        })?;
                    let toks = parse_tokens(p_line);
                    if toks.len() < 3 {
                        return Err(IllustrateError::InvalidCommand {
                            line: p_no,
                            message: "translation requires 3 values".into(),
                        });
                    }
                    translation[0] += parse_f32(toks[0], p_no)?;
                    translation[1] += parse_f32(toks[1], p_no)?;
                    translation[2] += parse_f32(toks[2], p_no)?;
                }
                "xro" => {
                    let (p_no, p_line) =
                        next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                            IllustrateError::InvalidCommand {
                                line: line_no,
                                message: "expected rotation angle after 'xrot'".into(),
                            }
                        })?;
                    let toks = parse_tokens(p_line);
                    let angle = parse_f32(
                        toks.first()
                            .ok_or_else(|| IllustrateError::InvalidCommand {
                                line: p_no,
                                message: "missing angle for 'xrot'".into(),
                            })?,
                        p_no,
                    )?;
                    rotation.catenate(&Matrix4::rotation_x(angle));
                }
                "yro" => {
                    let (p_no, p_line) =
                        next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                            IllustrateError::InvalidCommand {
                                line: line_no,
                                message: "expected rotation angle after 'yrot'".into(),
                            }
                        })?;
                    let toks = parse_tokens(p_line);
                    let angle = parse_f32(
                        toks.first()
                            .ok_or_else(|| IllustrateError::InvalidCommand {
                                line: p_no,
                                message: "missing angle for 'yrot'".into(),
                            })?,
                        p_no,
                    )?;
                    rotation.catenate(&Matrix4::rotation_y(angle));
                }
                "zro" => {
                    let (p_no, p_line) =
                        next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                            IllustrateError::InvalidCommand {
                                line: line_no,
                                message: "expected rotation angle after 'zrot'".into(),
                            }
                        })?;
                    let toks = parse_tokens(p_line);
                    let angle = parse_f32(
                        toks.first()
                            .ok_or_else(|| IllustrateError::InvalidCommand {
                                line: p_no,
                                message: "missing angle for 'zrot'".into(),
                            })?,
                        p_no,
                    )?;
                    rotation.catenate(&Matrix4::rotation_z(angle));
                }
                "sca" => {
                    let (p_no, p_line) =
                        next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                            IllustrateError::InvalidCommand {
                                line: line_no,
                                message: "expected scale factor after 'scale'".into(),
                            }
                        })?;
                    let toks = parse_tokens(p_line);
                    let factor = parse_f32(
                        toks.first()
                            .ok_or_else(|| IllustrateError::InvalidCommand {
                                line: p_no,
                                message: "missing scale value".into(),
                            })?,
                        p_no,
                    )?;
                    scale *= factor;
                }
                "cen" => {
                    let (_p_no, p_line) =
                        next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                            IllustrateError::InvalidCommand {
                                line: line_no,
                                message: "expected centering mode ('auto' or 'center')".into(),
                            }
                        })?;
                    let mode: String = strip_comment(p_line).chars().take(3).collect();
                    center_mode = match mode.as_str() {
                        "aut" => CenterMode::Auto,
                        "cen" => CenterMode::Center,
                        _ => CenterMode::None,
                    };
                }
                "wor" => {
                    let (p1_no, p1) = next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                        IllustrateError::InvalidCommand {
                            line: line_no,
                            message: "expected background/fog parameters for 'world'".into(),
                        }
                    })?;
                    let t1 = parse_tokens(p1);
                    if t1.len() < 8 {
                        return Err(IllustrateError::InvalidCommand {
                            line: p1_no,
                            message:
                                "world line 1 requires 8 values (rback[3], rfog[3], pfogh, pfogl)"
                                    .into(),
                        });
                    }
                    world.rback = [
                        parse_f32(t1[0], p1_no)?.min(1.0),
                        parse_f32(t1[1], p1_no)?.min(1.0),
                        parse_f32(t1[2], p1_no)?.min(1.0),
                    ];
                    world.rfog = [
                        parse_f32(t1[3], p1_no)?.min(1.0),
                        parse_f32(t1[4], p1_no)?.min(1.0),
                        parse_f32(t1[5], p1_no)?.min(1.0),
                    ];
                    world.pfogh = parse_f32(t1[6], p1_no)?.min(1.0);
                    let mut pfogl = parse_f32(t1[7], p1_no)?;
                    if pfogl < 0.0 {
                        pfogl = -pfogl;
                    }
                    world.pfogl = pfogl.min(1.0);

                    let (p2_no, p2) = next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                        IllustrateError::InvalidCommand {
                            line: line_no,
                            message: "expected shadow parameters for 'world'".into(),
                        }
                    })?;
                    let t2 = parse_tokens(p2);
                    if t2.len() < 5 {
                        return Err(IllustrateError::InvalidCommand {
                            line: p2_no,
                            message: "world line 2 requires 5 values (icone, pcone, coneangle, rcone, pshadowmax)"
                                .into(),
                        });
                    }
                    world.icone = parse_i32(t2[0], p2_no)?;
                    world.pcone = parse_f32(t2[1], p2_no)?;
                    world.coneangle = parse_f32(t2[2], p2_no)?;
                    world.rcone = parse_f32(t2[3], p2_no)?;
                    world.pshadowmax = parse_f32(t2[4], p2_no)?;

                    let (p3_no, p3) = next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                        IllustrateError::InvalidCommand {
                            line: line_no,
                            message: "expected image dimensions for 'world'".into(),
                        }
                    })?;
                    let t3 = parse_tokens(p3);
                    if t3.len() < 2 {
                        return Err(IllustrateError::InvalidCommand {
                            line: p3_no,
                            message: "world line 3 requires 2 integers (ixsize, iysize)".into(),
                        });
                    }
                    world.ixsize = parse_i32(t3[0], p3_no)?.min(3000);
                    world.iysize = parse_i32(t3[1], p3_no)?.min(3000);
                }
                "ill" => {
                    outline.enabled = true;
                    let (p1_no, p1) = next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                        IllustrateError::InvalidCommand {
                            line: line_no,
                            message: "expected contour outline parameters for 'illustrate'".into(),
                        }
                    })?;
                    let t1 = parse_tokens(p1);
                    if t1.len() < 5 {
                        return Err(IllustrateError::InvalidCommand {
                            line: p1_no,
                            message: "illustrate line 1 requires 5 values".into(),
                        });
                    }
                    outline.l_low = parse_f32(t1[0], p1_no)?;
                    outline.l_high = parse_f32(t1[1], p1_no)?;
                    outline.ikernel = parse_i32(t1[2], p1_no)?;
                    outline.l_diff_min = parse_f32(t1[3], p1_no)?;
                    outline.l_diff_max = parse_f32(t1[4], p1_no)?;

                    let (p2_no, p2) = next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                        IllustrateError::InvalidCommand {
                            line: line_no,
                            message: "expected subunit outline parameters for 'illustrate'".into(),
                        }
                    })?;
                    let t2 = parse_tokens(p2);
                    if t2.len() < 2 {
                        return Err(IllustrateError::InvalidCommand {
                            line: p2_no,
                            message: "illustrate line 2 requires 2 values".into(),
                        });
                    }
                    outline.r_low = parse_f32(t2[0], p2_no)?;
                    outline.r_high = parse_f32(t2[1], p2_no)?;

                    let (p3_no, p3) = next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                        IllustrateError::InvalidCommand {
                            line: line_no,
                            message: "expected residue outline parameters for 'illustrate'".into(),
                        }
                    })?;
                    let t3 = parse_tokens(p3);
                    if t3.len() < 3 {
                        return Err(IllustrateError::InvalidCommand {
                            line: p3_no,
                            message: "illustrate line 3 requires 3 values".into(),
                        });
                    }
                    outline.g_low = parse_f32(t3[0], p3_no)?;
                    outline.g_high = parse_f32(t3[1], p3_no)?;
                    outline.resdiff = parse_f32(t3[2], p3_no)?;
                }
                "cal" => {
                    let (p_no, p_line) =
                        next_non_empty_line(&lines, &mut idx).ok_or_else(|| {
                            IllustrateError::InvalidCommand {
                                line: line_no,
                                message: "expected output filename after 'calculate'".into(),
                            }
                        })?;
                    let out_str = strip_comment(p_line);
                    if out_str.is_empty() {
                        return Err(IllustrateError::InvalidCommand {
                            line: p_no,
                            message: "empty output filename after 'calculate'".into(),
                        });
                    }
                    output_path = Some(PathBuf::from(out_str));
                }
                other => {
                    return Err(IllustrateError::InvalidCommand {
                        line: line_no,
                        message: format!("unknown command card '{other}'"),
                    });
                }
            }
        }

        Ok(Self {
            pdb_path: pdb_path.ok_or_else(|| IllustrateError::InvalidCommand {
                line: 1,
                message: "command deck missing 'read' command".into(),
            })?,
            descriptors,
            translation,
            rotation,
            scale,
            center_mode,
            world,
            outline,
            output_path,
        })
    }

    /// Serializes this [`CommandDeck`] into a legacy-compatible `.inp` command deck string.
    ///
    /// If `euler_deg` is `None`, the Euler angles `[xrot, yrot, zrot]` are decomposed from
    /// [`Self::rotation`].
    #[must_use]
    pub fn to_script(&self, euler_deg: Option<[f32; 3]>) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "read");
        let _ = writeln!(out, "{}", self.pdb_path.display());
        for desc in &self.descriptors {
            let _ = writeln!(
                out,
                "{}{} {},{}, {:.3},{:.3},{:.3}, {:.2}",
                desc.record_prefix_str(),
                desc.pattern_str(),
                desc.res_range.0,
                desc.res_range.1,
                desc.color[0],
                desc.color[1],
                desc.color[2],
                desc.radius
            );
        }
        let _ = writeln!(out, "END");

        match self.center_mode {
            CenterMode::Auto => {
                let _ = writeln!(out, "center\nauto");
            }
            CenterMode::Center => {
                let _ = writeln!(out, "center\ncen");
            }
            CenterMode::None => {}
        }

        let _ = writeln!(
            out,
            "trans\n{:.3},{:.3},{:.3}",
            self.translation[0], self.translation[1], self.translation[2]
        );
        let _ = writeln!(out, "scale\n{:.3}", self.scale);

        let [xrot, yrot, zrot] = euler_deg.unwrap_or_else(|| self.rotation.to_euler_angles_deg());
        if zrot.abs() > 1e-4 {
            let _ = writeln!(out, "zrot\n{zrot:.3}");
        }
        if yrot.abs() > 1e-4 {
            let _ = writeln!(out, "yrot\n{yrot:.3}");
        }
        if xrot.abs() > 1e-4 {
            let _ = writeln!(out, "xrot\n{xrot:.3}");
        }

        let _ = writeln!(
            out,
            "wor\n{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
            self.world.rback[0],
            self.world.rback[1],
            self.world.rback[2],
            self.world.rfog[0],
            self.world.rfog[1],
            self.world.rfog[2],
            self.world.pfogh,
            self.world.pfogl
        );
        let _ = writeln!(
            out,
            "{},{:.4},{:.3},{:.3},{:.3}",
            self.world.icone,
            self.world.pcone,
            self.world.coneangle,
            self.world.rcone,
            self.world.pshadowmax
        );
        let _ = writeln!(out, "{},{}", self.world.ixsize, self.world.iysize);

        if self.outline.enabled {
            let _ = writeln!(
                out,
                "illustrate\n{:.3},{:.3},{},{:.3},{:.3}",
                self.outline.l_low,
                self.outline.l_high,
                self.outline.ikernel,
                self.outline.l_diff_min,
                self.outline.l_diff_max
            );
            let _ = writeln!(out, "{:.3},{:.3}", self.outline.r_low, self.outline.r_high);
            let _ = writeln!(
                out,
                "{:.3},{:.3},{:.1}",
                self.outline.g_low, self.outline.g_high, self.outline.resdiff
            );
        }

        let out_name = self
            .output_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "output.pnm".to_string());
        let _ = writeln!(out, "calculate\n{out_name}");

        out
    }
}

fn parse_descriptor_line(raw: &str, line_no: usize) -> Result<AtomDescriptor> {
    let padded = format!("{raw:<80}");
    let bytes = padded.as_bytes();
    if bytes.len() < 18 {
        return Err(IllustrateError::InvalidDescriptor {
            line: line_no,
            message: "descriptor line too short".into(),
        });
    }

    let mut record_prefix = [0_u8; 6];
    record_prefix.copy_from_slice(&bytes[0..6]);

    let mut pattern = [0_u8; 10];
    pattern.copy_from_slice(&bytes[6..16]);

    let params_slice = if bytes[16] == b' ' {
        &padded[17..]
    } else {
        &padded[16..]
    };
    let toks = parse_tokens(params_slice);
    if toks.len() < 6 {
        return Err(IllustrateError::InvalidDescriptor {
            line: line_no,
            message: format!(
                "expected 6 numeric fields after pattern, found {}",
                toks.len()
            ),
        });
    }

    let res_low = parse_i32(toks[0], line_no)?;
    let res_high = parse_i32(toks[1], line_no)?;
    let r = parse_f32(toks[2], line_no)?;
    let g = parse_f32(toks[3], line_no)?;
    let b = parse_f32(toks[4], line_no)?;
    let radius = parse_f32(toks[5], line_no)?;

    Ok(AtomDescriptor {
        record_prefix,
        pattern,
        res_range: (res_low, res_high),
        color: [r, g, b],
        radius,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sample_2hhb_style_deck_with_comments() {
        let deck_src = "\
read # READ command
2hhb.pdb # PDB file
HETATM-----HOH-- 0,9999, 0.5,0.5,0.5, 0.0 # omit water
ATOM  -C-------A 0,9999, 1.0,0.6,0.6, 1.6
END
center
auto
trans
0.,0.,0.
scale
12.0
zrot
90.
wor
1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0
1,0.0023,2.0,1.0,0.2
-30,-30
illustrate
3.0,10.0,4,0.0,5.0
3.0,10.0
3.0,8.0,6000.
calculate
2hhb.pnm
";
        let deck = CommandDeck::parse(deck_src).expect("should parse sample deck");
        assert_eq!(deck.pdb_path, PathBuf::from("2hhb.pdb"));
        assert_eq!(deck.descriptors.len(), 2);
        assert_eq!(deck.center_mode, CenterMode::Auto);
        assert_eq!(deck.scale, 12.0);
        assert_eq!(deck.world.ixsize, -30);
        assert_eq!(deck.outline.ikernel, 4);
        assert_eq!(deck.output_path, Some(PathBuf::from("2hhb.pnm")));
    }

    #[test]
    fn wildcard_matching_checks_columns_13_to_22() {
        let desc = parse_descriptor_line("ATOM  -C-------A 0,9999, 1.0,0.6,0.6, 1.6", 1).unwrap();
        let line_match = format!(
            "{:<80}",
            "ATOM      2  CA  VAL A   1       6.953  16.758   4.864  1.00 26.86           C"
        );
        let line_wrong_chain = format!(
            "{:<80}",
            "ATOM      2  CA  VAL B   1       6.953  16.758   4.864  1.00 26.86           C"
        );
        assert!(desc.matches(line_match.as_bytes(), 1));
        assert!(!desc.matches(line_wrong_chain.as_bytes(), 1));
    }

    #[test]
    fn to_script_round_trips_command_deck() {
        let deck_src = "\
read
2hhb.pdb
HETATM-----HOH-- 0,9999, 0.5,0.5,0.5, 0.0
ATOM  -C-------A 0,9999, 1.0,0.6,0.6, 1.6
END
center
auto
trans
1.0,2.0,-3.0
scale
12.0
zrot
90.0
xrot
15.0
wor
1.0,1.0,1.0,0.9,0.9,0.9,1.0,0.5
1,0.0023,2.0,1.0,0.2
-30,-30
illustrate
3.0,10.0,4,0.0,5.0
3.0,10.0
3.0,8.0,6000.0
calculate
2hhb.pnm
";
        let deck = CommandDeck::parse(deck_src).unwrap();
        let script = deck.to_script(None);
        let reparsed = CommandDeck::parse(&script).unwrap();

        assert_eq!(reparsed.pdb_path, deck.pdb_path);
        assert_eq!(reparsed.descriptors, deck.descriptors);
        assert_eq!(reparsed.center_mode, deck.center_mode);
        assert_eq!(reparsed.translation, deck.translation);
        assert_eq!(reparsed.scale, deck.scale);
        assert_eq!(reparsed.world, deck.world);
        assert_eq!(reparsed.outline, deck.outline);
        assert_eq!(reparsed.output_path, deck.output_path);

        let v = [1.0, -2.0, 3.0];
        let a = deck.rotation.transform_vector(v);
        let b = reparsed.rotation.transform_vector(v);
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-4);
        }
    }
}
