use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::command::AtomDescriptor;
use crate::error::{IllustrateError, Result};
use crate::transform::BioMatrix;

/// Sentinel / real atom entry stored in [`PdbStructure::atoms`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Atom {
    /// Cartesian coordinates `(x, y, z)` in Angstroms.
    pub coord: [f32; 3],
    /// 1-based descriptor index (`0` for background sentinel).
    pub type_id: usize,
    /// Subunit identifier (`9999` for background sentinel).
    pub su: i32,
    /// Residue sequence number (`9999` for background sentinel).
    pub res: i32,
}

/// Loaded PDB structure together with biological assembly metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct PdbStructure {
    /// Atoms stored with sentinel at index `0` (`atoms[1..]` are real atoms).
    pub atoms: Vec<Atom>,
    /// Number of automatically assigned subunits (`MODEL` + chain transitions).
    pub num_subunits: i32,
    /// Chain IDs listed under `REMARK 350 ... CHAINS:` for `BIOMOLECULE: 1`.
    pub bio_chains: Vec<u8>,
    /// Biological assembly transformation matrices (`BIOMT1..3`).
    pub biomats: Vec<BioMatrix>,
}

impl PdbStructure {
    /// Number of real atoms (`atoms.len() - 1`).
    #[must_use]
    pub fn atom_count(&self) -> usize {
        self.atoms.len().saturating_sub(1)
    }
}

/// Scans raw PDB or mmCIF text and returns sorted unique non-blank chain IDs present on
/// `ATOM` / `HETATM` records (excluding `HOH` waters).
#[must_use]
pub fn inspect_pdb_chains(pdb_text: &str) -> Vec<char> {
    if is_mmcif_text(pdb_text) {
        return inspect_mmcif_chains(pdb_text);
    }

    let mut chains = Vec::new();
    for raw in pdb_text.lines() {
        let padded = pad_80(raw);
        if !padded.starts_with("ATOM") && !padded.starts_with("HETATM") {
            continue;
        }
        if &padded[17..20] == "HOH" {
            continue;
        }
        let ch = padded.as_bytes()[21] as char;
        if ch != ' ' && !chains.contains(&ch) {
            chains.push(ch);
        }
    }
    chains
}

/// Resolves a PDB path using CWD -> script directory -> `examples/` fallback order.
pub fn resolve_pdb_path(pdb_path: &Path, script_dir: Option<&Path>) -> Result<PathBuf> {
    if pdb_path.exists() {
        return Ok(pdb_path.to_path_buf());
    }
    if let Some(dir) = script_dir {
        let candidate = dir.join(pdb_path);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    let examples_candidate = Path::new("examples").join(pdb_path);
    if examples_candidate.exists() {
        return Ok(examples_candidate);
    }
    Err(IllustrateError::PdbNotFound {
        path: pdb_path.to_path_buf(),
    })
}

/// Loads and classifies atoms from a PDB file on disk.
pub fn load_pdb_file(path: &Path, descriptors: &[AtomDescriptor]) -> Result<PdbStructure> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    parse_pdb_reader(reader, descriptors)
}

/// Loads and classifies atoms from any buffered reader containing PDB or mmCIF records.
pub fn parse_pdb_reader<R: BufRead>(
    mut reader: R,
    descriptors: &[AtomDescriptor],
) -> Result<PdbStructure> {
    let mut text = String::new();
    reader.read_to_string(&mut text)?;
    if is_mmcif_text(&text) {
        return parse_mmcif_str(&text, descriptors);
    }
    parse_pdb_str(&text, descriptors)
}

fn parse_pdb_str(text: &str, descriptors: &[AtomDescriptor]) -> Result<PdbStructure> {
    let mut lines = text.lines();

    let mut atoms = Vec::with_capacity(4096);
    // Index 0 is the background sentinel matching Fortran `su(0)=9999, res(0)=9999, type=0`.
    atoms.push(Atom {
        coord: [0.0; 3],
        type_id: 0,
        su: 9999,
        res: 9999,
    });

    let mut nsu: i32 = 0;
    let mut chainlast: Option<u8> = None;
    let mut bio_chains: Vec<u8> = Vec::new();
    let mut biomats: Vec<BioMatrix> = Vec::new();
    let mut line_no: usize = 0;

    while let Some(raw) = lines.next() {
        line_no += 1;
        let mut padded = pad_80(raw);

        if padded.starts_with("MODEL") {
            nsu += 1;
        }

        if &padded[11..25] == "BIOMOLECULE: 1" {
            // Mirrors Fortran `8010` loop inside `BIOMOLECULE: 1` block.
            for sub_raw in lines.by_ref() {
                line_no += 1;
                padded = pad_80(sub_raw);

                if &padded[0..10] == "REMARK 350" && &padded[34..40] == "CHAINS" {
                    let bytes = padded.as_bytes();
                    let mut ich = 42usize; // 0-based for Fortran col 43
                    while ich < 80 && bytes[ich] != b' ' {
                        bio_chains.push(bytes[ich]);
                        ich += 3;
                    }
                    continue;
                }

                if &padded[13..19] == "BIOMT1" {
                    biomats.push(BioMatrix::identity());
                    let cur = biomats.last_mut().expect("just pushed");
                    parse_biomt_row(&padded[19..], &mut cur.rows[0], line_no)?;
                } else if &padded[13..19] == "BIOMT2"
                    && let Some(cur) = biomats.last_mut()
                {
                    parse_biomt_row(&padded[19..], &mut cur.rows[1], line_no)?;
                } else if &padded[13..19] == "BIOMT3"
                    && let Some(cur) = biomats.last_mut()
                {
                    parse_biomt_row(&padded[19..], &mut cur.rows[2], line_no)?;
                }

                if &padded[13..19] == "      " {
                    break;
                }
            }
        }

        if !padded.starts_with("ATOM") && !padded.starts_with("HETATM") {
            continue;
        }

        let ires: i32 = padded[22..26]
            .trim()
            .parse()
            .map_err(|e| IllustrateError::InvalidPdb {
                line: line_no,
                message: format!("invalid residue number '{}': {e}", &padded[22..26]),
            })?;

        let bytes = padded.as_bytes();
        for (idx, desc) in descriptors.iter().enumerate() {
            if !desc.matches(bytes, ires) {
                continue;
            }
            if desc.radius == 0.0 {
                break;
            }
            let chain = bytes[21];
            if !bio_chains.is_empty() && !bio_chains.contains(&chain) {
                break;
            }

            let x = parse_coord(&padded[30..38], line_no)?;
            let y = parse_coord(&padded[38..46], line_no)?;
            let z = parse_coord(&padded[46..54], line_no)?;

            if chainlast != Some(chain) {
                nsu += 1;
                chainlast = Some(chain);
            }

            atoms.push(Atom {
                coord: [x, y, z],
                type_id: idx + 1,
                su: nsu,
                res: ires,
            });
            break;
        }
    }

    Ok(PdbStructure {
        atoms,
        num_subunits: nsu,
        bio_chains,
        biomats,
    })
}

fn is_mmcif_text(text: &str) -> bool {
    for raw in text.lines() {
        let trimmed = raw.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("data_")
            || trimmed.starts_with("loop_")
            || trimmed.starts_with("_atom_site.")
        {
            return true;
        }
        if trimmed.starts_with("HEADER")
            || trimmed.starts_with("TITLE")
            || trimmed.starts_with("REMARK")
            || trimmed.starts_with("MODEL")
            || trimmed.starts_with("ATOM  ")
            || trimmed.starts_with("HETATM")
        {
            return false;
        }
    }
    false
}

#[derive(Debug, Clone)]
struct CifToken {
    text: String,
    quoted: bool,
    line: usize,
}

fn tokenize_mmcif(text: &str) -> Vec<CifToken> {
    let mut tokens = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0usize;

    while i < lines.len() {
        let raw_line = lines[i].trim_end_matches('\r');
        let line_no = i + 1;

        if let Some(rest) = raw_line.strip_prefix(';') {
            let mut body = String::from(rest);
            i += 1;
            while i < lines.len() {
                let next = lines[i].trim_end_matches('\r');
                if next.starts_with(';') {
                    i += 1;
                    break;
                }
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(next);
                i += 1;
            }
            tokens.push(CifToken {
                text: body,
                quoted: true,
                line: line_no,
            });
            continue;
        }

        let bytes = raw_line.as_bytes();
        let mut col = 0usize;
        while col < bytes.len() {
            while col < bytes.len() && bytes[col].is_ascii_whitespace() {
                col += 1;
            }
            if col >= bytes.len() || bytes[col] == b'#' {
                break;
            }
            if bytes[col] == b'\'' || bytes[col] == b'"' {
                let quote = bytes[col];
                col += 1;
                let start = col;
                while col < bytes.len() {
                    if bytes[col] == quote
                        && (col + 1 == bytes.len() || bytes[col + 1].is_ascii_whitespace())
                    {
                        break;
                    }
                    col += 1;
                }
                let slice = std::str::from_utf8(&bytes[start..col]).unwrap_or("");
                tokens.push(CifToken {
                    text: slice.to_string(),
                    quoted: true,
                    line: line_no,
                });
                if col < bytes.len() && bytes[col] == quote {
                    col += 1;
                }
            } else {
                let start = col;
                while col < bytes.len() && !bytes[col].is_ascii_whitespace() {
                    col += 1;
                }
                let slice = std::str::from_utf8(&bytes[start..col]).unwrap_or("");
                tokens.push(CifToken {
                    text: slice.to_string(),
                    quoted: false,
                    line: line_no,
                });
            }
        }
        i += 1;
    }

    tokens
}

fn clean_cif_opt(val: Option<&str>) -> Option<&str> {
    val.map(str::trim)
        .filter(|s| !s.is_empty() && *s != "." && *s != "?")
}

fn format_pdb_atom_name(atom_name: &str, type_symbol: Option<&str>) -> [u8; 4] {
    let name = atom_name.trim();
    let elem = clean_cif_opt(type_symbol).unwrap_or("");
    let formatted = if name.len() >= 4 {
        name[..4].to_string()
    } else if elem.len() >= 2
        && name
            .to_ascii_uppercase()
            .starts_with(&elem.to_ascii_uppercase())
    {
        format!("{name:<4}")
    } else {
        format!(" {name:<3}")
    };
    let mut out = [b' '; 4];
    for (i, b) in formatted.as_bytes().iter().take(4).enumerate() {
        out[i] = *b;
    }
    out
}

fn synthesize_pdb_header(
    group_pdb: &str,
    atom_name: &str,
    type_symbol: Option<&str>,
    alt_id: Option<&str>,
    comp_id: &str,
    chain_char: u8,
) -> [u8; 26] {
    let mut buf = [b' '; 26];
    let rec = if group_pdb.eq_ignore_ascii_case("HETATM") {
        b"HETATM"
    } else {
        b"ATOM  "
    };
    buf[0..6].copy_from_slice(rec);

    let name_bytes = format_pdb_atom_name(atom_name, type_symbol);
    buf[12..16].copy_from_slice(&name_bytes);

    if let Some(alt) = clean_cif_opt(alt_id)
        && let Some(&b) = alt.as_bytes().first()
    {
        buf[16] = b;
    }

    let comp = comp_id.trim();
    let comp_fmt = format!("{comp:>3}");
    for (i, b) in comp_fmt.as_bytes().iter().take(3).enumerate() {
        buf[17 + i] = *b;
    }

    buf[21] = chain_char;
    buf
}

fn expand_oper_group(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in expr.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((a, b)) = trimmed.split_once('-')
            && let (Ok(start), Ok(end)) = (a.trim().parse::<i32>(), b.trim().parse::<i32>())
        {
            let step = if start <= end { 1 } else { -1 };
            let mut cur = start;
            loop {
                out.push(cur.to_string());
                if cur == end {
                    break;
                }
                cur += step;
            }
        } else {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn expand_oper_expression(expr: &str) -> Vec<Vec<String>> {
    let trimmed = expr.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == "?" {
        return Vec::new();
    }
    if !trimmed.contains('(') {
        return expand_oper_group(trimmed)
            .into_iter()
            .map(|id| vec![id])
            .collect();
    }

    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut rest = trimmed;
    while let Some(open) = rest.find('(') {
        let after_open = &rest[open + 1..];
        if let Some(close) = after_open.find(')') {
            let group_items = expand_oper_group(&after_open[..close]);
            if !group_items.is_empty() {
                groups.push(group_items);
            }
            rest = &after_open[close + 1..];
        } else {
            break;
        }
    }

    let mut combos: Vec<Vec<String>> = vec![Vec::new()];
    for group in groups {
        let mut next = Vec::with_capacity(combos.len() * group.len());
        for prefix in &combos {
            for item in &group {
                let mut combined = prefix.clone();
                combined.push(item.clone());
                next.push(combined);
            }
        }
        combos = next;
    }
    combos
}

fn compose_biomatrix(a: &BioMatrix, b: &BioMatrix) -> BioMatrix {
    let mut out = BioMatrix::identity();
    for r in 0..3 {
        for c in 0..3 {
            out.rows[r][c] = a.rows[r][0] * b.rows[0][c]
                + a.rows[r][1] * b.rows[1][c]
                + a.rows[r][2] * b.rows[2][c];
        }
        out.rows[r][3] = a.rows[r][0] * b.rows[0][3]
            + a.rows[r][1] * b.rows[1][3]
            + a.rows[r][2] * b.rows[2][3]
            + a.rows[r][3];
    }
    out
}

fn inspect_mmcif_chains(text: &str) -> Vec<char> {
    let tokens = tokenize_mmcif(text);
    let mut chains = Vec::new();
    let mut idx = 0usize;

    while idx < tokens.len() {
        let tok = &tokens[idx];
        if !tok.quoted && tok.text.eq_ignore_ascii_case("loop_") {
            idx += 1;
            let mut tags = Vec::new();
            while idx < tokens.len() && !tokens[idx].quoted && tokens[idx].text.starts_with('_') {
                tags.push(tokens[idx].text.clone());
                idx += 1;
            }
            if tags.is_empty() {
                continue;
            }
            let is_atom_site = tags.iter().any(|t| t.starts_with("_atom_site."));
            let mut row_buf: Vec<&str> = Vec::with_capacity(tags.len());
            while idx < tokens.len() {
                let cur = &tokens[idx];
                if !cur.quoted
                    && (cur.text.starts_with('_')
                        || cur.text.eq_ignore_ascii_case("loop_")
                        || cur.text.starts_with("data_"))
                {
                    break;
                }
                row_buf.push(&cur.text);
                idx += 1;
                if row_buf.len() == tags.len() {
                    if is_atom_site {
                        let comp = find_tag(&tags, &row_buf, "_atom_site.auth_comp_id")
                            .or_else(|| find_tag(&tags, &row_buf, "_atom_site.label_comp_id"))
                            .unwrap_or("");
                        if comp != "HOH" && comp != "WAT" {
                            let asym =
                                clean_cif_opt(find_tag(&tags, &row_buf, "_atom_site.auth_asym_id"))
                                    .or_else(|| {
                                        clean_cif_opt(find_tag(
                                            &tags,
                                            &row_buf,
                                            "_atom_site.label_asym_id",
                                        ))
                                    });
                            if let Some(ch_str) = asym
                                && let Some(ch) = ch_str.chars().next()
                                && ch != ' '
                                && !chains.contains(&ch)
                            {
                                chains.push(ch);
                            }
                        }
                    }
                    row_buf.clear();
                }
            }
        } else {
            idx += 1;
        }
    }

    chains
}

fn find_tag<'a>(tags: &[String], row: &[&'a str], key: &str) -> Option<&'a str> {
    tags.iter()
        .position(|t| t == key)
        .and_then(|pos| row.get(pos).copied())
}

struct MmcifAtomRow {
    group_pdb: String,
    atom_name: String,
    type_symbol: Option<String>,
    alt_id: Option<String>,
    comp_id: String,
    auth_asym_id: String,
    label_asym_id: String,
    seq_id: String,
    x: f32,
    y: f32,
    z: f32,
    model_num: String,
    line_no: usize,
}

fn parse_mmcif_str(text: &str, descriptors: &[AtomDescriptor]) -> Result<PdbStructure> {
    let tokens = tokenize_mmcif(text);
    let mut atom_rows: Vec<MmcifAtomRow> = Vec::new();
    let mut single_kv: HashMap<String, (String, usize)> = HashMap::new();
    let mut oper_map: HashMap<String, BioMatrix> = HashMap::new();
    let mut assembly_chains: HashSet<String> = HashSet::new();
    let mut assembly_opers: Vec<String> = Vec::new();

    let mut idx = 0usize;
    while idx < tokens.len() {
        let tok = &tokens[idx];
        if !tok.quoted && tok.text.eq_ignore_ascii_case("loop_") {
            idx += 1;
            let mut tags = Vec::new();
            while idx < tokens.len() && !tokens[idx].quoted && tokens[idx].text.starts_with('_') {
                tags.push(tokens[idx].text.clone());
                idx += 1;
            }
            if tags.is_empty() {
                continue;
            }

            let is_atom_site = tags.iter().any(|t| t.starts_with("_atom_site."));
            let is_oper_list = tags
                .iter()
                .any(|t| t.starts_with("_pdbx_struct_oper_list."));
            let is_assembly_gen = tags
                .iter()
                .any(|t| t.starts_with("_pdbx_struct_assembly_gen."));

            let mut row_tokens: Vec<&CifToken> = Vec::with_capacity(tags.len());
            while idx < tokens.len() {
                let cur = &tokens[idx];
                if !cur.quoted
                    && (cur.text.starts_with('_')
                        || cur.text.eq_ignore_ascii_case("loop_")
                        || cur.text.starts_with("data_"))
                {
                    break;
                }
                row_tokens.push(cur);
                idx += 1;
                if row_tokens.len() == tags.len() {
                    let row_strs: Vec<&str> = row_tokens.iter().map(|t| t.text.as_str()).collect();
                    let row_line = row_tokens[0].line;
                    if is_atom_site {
                        atom_rows.push(extract_mmcif_atom_row(&tags, &row_strs, row_line)?);
                    } else if is_oper_list {
                        extract_mmcif_oper(&tags, &row_strs, row_line, &mut oper_map)?;
                    } else if is_assembly_gen {
                        let asm_id =
                            find_tag(&tags, &row_strs, "_pdbx_struct_assembly_gen.assembly_id")
                                .unwrap_or("");
                        if asm_id == "1" {
                            if let Some(expr) = find_tag(
                                &tags,
                                &row_strs,
                                "_pdbx_struct_assembly_gen.oper_expression",
                            ) {
                                assembly_opers.push(expr.to_string());
                            }
                            if let Some(chains_str) =
                                find_tag(&tags, &row_strs, "_pdbx_struct_assembly_gen.asym_id_list")
                            {
                                for c in chains_str
                                    .split(',')
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                {
                                    assembly_chains.insert(c.to_string());
                                }
                            }
                        }
                    }
                    row_tokens.clear();
                }
            }
        } else if !tok.quoted && tok.text.starts_with("_atom_site.") {
            let key = tok.text.clone();
            let line = tok.line;
            idx += 1;
            if idx < tokens.len() {
                single_kv.insert(key, (tokens[idx].text.clone(), line));
                idx += 1;
            }
        } else {
            idx += 1;
        }
    }

    if atom_rows.is_empty() && !single_kv.is_empty() {
        let mut tags = Vec::with_capacity(single_kv.len());
        let mut vals = Vec::with_capacity(single_kv.len());
        let mut first_line = 1usize;
        for (k, (v, line)) in &single_kv {
            tags.push(k.clone());
            vals.push(v.as_str());
            first_line = *line;
        }
        atom_rows.push(extract_mmcif_atom_row(&tags, &vals, first_line)?);
    }

    let mut biomats = Vec::new();
    for expr in &assembly_opers {
        for combo in expand_oper_expression(expr) {
            if combo.is_empty() {
                continue;
            }
            let mut acc = BioMatrix::identity();
            let mut valid = true;
            for op_id in combo {
                if let Some(op) = oper_map.get(&op_id) {
                    acc = compose_biomatrix(&acc, op);
                } else {
                    valid = false;
                    break;
                }
            }
            if valid {
                biomats.push(acc);
            }
        }
    }

    let mut atoms = Vec::with_capacity(atom_rows.len().saturating_add(1));
    atoms.push(Atom {
        coord: [0.0; 3],
        type_id: 0,
        su: 9999,
        res: 9999,
    });

    let mut nsu: i32 = 0;
    let mut last_model: Option<String> = None;
    let mut last_chain: Option<String> = None;
    let mut bio_chains: Vec<u8> = Vec::new();

    for ch_str in &assembly_chains {
        if let Some(&b) = ch_str.as_bytes().first()
            && !bio_chains.contains(&b)
        {
            bio_chains.push(b);
        }
    }

    for row in atom_rows {
        if !assembly_chains.is_empty()
            && !assembly_chains.contains(&row.label_asym_id)
            && !assembly_chains.contains(&row.auth_asym_id)
        {
            continue;
        }

        let ires: i32 = row.seq_id.trim().parse::<i32>().unwrap_or(1);
        let chain_key = if !row.auth_asym_id.is_empty() {
            &row.auth_asym_id
        } else {
            &row.label_asym_id
        };
        let chain_byte = chain_key.as_bytes().first().copied().unwrap_or(b' ');

        let header = synthesize_pdb_header(
            &row.group_pdb,
            &row.atom_name,
            row.type_symbol.as_deref(),
            row.alt_id.as_deref(),
            &row.comp_id,
            chain_byte,
        );

        for (desc_idx, desc) in descriptors.iter().enumerate() {
            if !desc.matches(&header, ires) {
                continue;
            }
            if desc.radius == 0.0 {
                break;
            }

            if last_model.as_deref() != Some(&row.model_num)
                || last_chain.as_deref() != Some(chain_key)
            {
                nsu += 1;
                last_model = Some(row.model_num.clone());
                last_chain = Some(chain_key.clone());
            }

            atoms.push(Atom {
                coord: [row.x, row.y, row.z],
                type_id: desc_idx + 1,
                su: nsu,
                res: ires,
            });
            break;
        }
        let _ = row.line_no;
    }

    Ok(PdbStructure {
        atoms,
        num_subunits: nsu,
        bio_chains,
        biomats,
    })
}

fn extract_mmcif_atom_row(tags: &[String], row: &[&str], line_no: usize) -> Result<MmcifAtomRow> {
    let group_pdb = find_tag(tags, row, "_atom_site.group_PDB")
        .unwrap_or("ATOM")
        .to_string();
    let atom_name = clean_cif_opt(find_tag(tags, row, "_atom_site.auth_atom_id"))
        .or_else(|| clean_cif_opt(find_tag(tags, row, "_atom_site.label_atom_id")))
        .unwrap_or("C")
        .to_string();
    let type_symbol =
        clean_cif_opt(find_tag(tags, row, "_atom_site.type_symbol")).map(str::to_string);
    let alt_id = clean_cif_opt(find_tag(tags, row, "_atom_site.label_alt_id")).map(str::to_string);
    let comp_id = clean_cif_opt(find_tag(tags, row, "_atom_site.auth_comp_id"))
        .or_else(|| clean_cif_opt(find_tag(tags, row, "_atom_site.label_comp_id")))
        .unwrap_or("UNK")
        .to_string();
    let auth_asym_id = clean_cif_opt(find_tag(tags, row, "_atom_site.auth_asym_id"))
        .unwrap_or("")
        .to_string();
    let label_asym_id = clean_cif_opt(find_tag(tags, row, "_atom_site.label_asym_id"))
        .unwrap_or("")
        .to_string();
    let seq_id = clean_cif_opt(find_tag(tags, row, "_atom_site.auth_seq_id"))
        .or_else(|| clean_cif_opt(find_tag(tags, row, "_atom_site.label_seq_id")))
        .unwrap_or("1")
        .to_string();
    let model_num = clean_cif_opt(find_tag(tags, row, "_atom_site.pdbx_PDB_model_num"))
        .unwrap_or("1")
        .to_string();

    let x_str =
        find_tag(tags, row, "_atom_site.Cartn_x").ok_or_else(|| IllustrateError::InvalidPdb {
            line: line_no,
            message: "mmCIF _atom_site missing Cartn_x".into(),
        })?;
    let y_str =
        find_tag(tags, row, "_atom_site.Cartn_y").ok_or_else(|| IllustrateError::InvalidPdb {
            line: line_no,
            message: "mmCIF _atom_site missing Cartn_y".into(),
        })?;
    let z_str =
        find_tag(tags, row, "_atom_site.Cartn_z").ok_or_else(|| IllustrateError::InvalidPdb {
            line: line_no,
            message: "mmCIF _atom_site missing Cartn_z".into(),
        })?;

    Ok(MmcifAtomRow {
        group_pdb,
        atom_name,
        type_symbol,
        alt_id,
        comp_id,
        auth_asym_id,
        label_asym_id,
        seq_id,
        x: parse_coord(x_str, line_no)?,
        y: parse_coord(y_str, line_no)?,
        z: parse_coord(z_str, line_no)?,
        model_num,
        line_no,
    })
}

fn extract_mmcif_oper(
    tags: &[String],
    row: &[&str],
    line_no: usize,
    oper_map: &mut HashMap<String, BioMatrix>,
) -> Result<()> {
    let Some(id) = clean_cif_opt(find_tag(tags, row, "_pdbx_struct_oper_list.id")) else {
        return Ok(());
    };
    let mut mat = BioMatrix::identity();
    let fields = [
        [
            "_pdbx_struct_oper_list.matrix[1][1]",
            "_pdbx_struct_oper_list.matrix[1][2]",
            "_pdbx_struct_oper_list.matrix[1][3]",
            "_pdbx_struct_oper_list.vector[1]",
        ],
        [
            "_pdbx_struct_oper_list.matrix[2][1]",
            "_pdbx_struct_oper_list.matrix[2][2]",
            "_pdbx_struct_oper_list.matrix[2][3]",
            "_pdbx_struct_oper_list.vector[2]",
        ],
        [
            "_pdbx_struct_oper_list.matrix[3][1]",
            "_pdbx_struct_oper_list.matrix[3][2]",
            "_pdbx_struct_oper_list.matrix[3][3]",
            "_pdbx_struct_oper_list.vector[3]",
        ],
    ];
    for (r, row_keys) in fields.iter().enumerate() {
        for (c, key) in row_keys.iter().enumerate() {
            if let Some(val) = clean_cif_opt(find_tag(tags, row, key)) {
                mat.rows[r][c] = parse_coord(val, line_no)?;
            }
        }
    }
    oper_map.insert(id.to_string(), mat);
    Ok(())
}

fn pad_80(line: &str) -> String {
    let trimmed = line.trim_end_matches('\r');
    if trimmed.len() >= 80 {
        trimmed.to_string()
    } else {
        format!("{trimmed:<80}")
    }
}

fn parse_coord(slice: &str, line_no: usize) -> Result<f32> {
    slice
        .trim()
        .parse::<f32>()
        .map_err(|e| IllustrateError::InvalidPdb {
            line: line_no,
            message: format!("invalid coordinate '{}': {e}", slice.trim()),
        })
}

fn parse_biomt_row(slice: &str, dst: &mut [f32; 4], line_no: usize) -> Result<()> {
    let toks: Vec<&str> = slice.split_whitespace().collect();
    if toks.len() < 5 {
        return Err(IllustrateError::InvalidPdb {
            line: line_no,
            message: format!("expected BIOMT index + 4 floats, got '{slice}'"),
        });
    }
    for (i, tok) in toks[1..5].iter().enumerate() {
        dst[i] = tok
            .parse::<f32>()
            .map_err(|e| IllustrateError::InvalidPdb {
                line: line_no,
                message: format!("invalid BIOMT float '{tok}': {e}"),
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandDeck;

    #[test]
    fn parses_biomt_and_subunits_from_snippet() {
        let cmd = "\
read
dummy.pdb
ATOM  -C-------A 0,9999, 1.0,0.6,0.6, 1.6
ATOM  -C-------B 0,9999, 1.0,0.6,0.6, 1.6
END
";
        let deck = CommandDeck::parse(cmd).unwrap();
        let pdb = "\
REMARK 350 BIOMOLECULE: 1
REMARK 350 APPLY THE FOLLOWING TO CHAINS: A, B
REMARK 350   BIOMT1   1  1.000000  0.000000  0.000000        0.00000
REMARK 350   BIOMT2   1  0.000000  1.000000  0.000000        0.00000
REMARK 350   BIOMT3   1  0.000000  0.000000  1.000000        0.00000
REMARK 400
ATOM      1  CA  VAL A   1       1.000   2.000   3.000  1.00 10.00           C
ATOM      2  CA  VAL B   1       4.000   5.000   6.000  1.00 10.00           C
ATOM      3  CA  VAL C   1       7.000   8.000   9.000  1.00 10.00           C
";
        let parsed = parse_pdb_reader(pdb.as_bytes(), &deck.descriptors).unwrap();
        assert_eq!(parsed.bio_chains, vec![b'A', b'B']);
        assert_eq!(parsed.biomats.len(), 1);
        assert_eq!(parsed.atom_count(), 2); // Chain C excluded by bio_chains
        assert_eq!(parsed.num_subunits, 2);
        assert_eq!(parsed.atoms[1].su, 1);
        assert_eq!(parsed.atoms[2].su, 2);
    }

    #[test]
    fn parses_mmcif_chains_atoms_omissions_and_assembly() {
        let cif = "\
data_TEST
#
loop_
_pdbx_struct_assembly_gen.assembly_id
_pdbx_struct_assembly_gen.oper_expression
_pdbx_struct_assembly_gen.asym_id_list
1 1,2 A,B
#
loop_
_pdbx_struct_oper_list.id
_pdbx_struct_oper_list.matrix[1][1]
_pdbx_struct_oper_list.matrix[1][2]
_pdbx_struct_oper_list.matrix[1][3]
_pdbx_struct_oper_list.vector[1]
_pdbx_struct_oper_list.matrix[2][1]
_pdbx_struct_oper_list.matrix[2][2]
_pdbx_struct_oper_list.matrix[2][3]
_pdbx_struct_oper_list.vector[2]
_pdbx_struct_oper_list.matrix[3][1]
_pdbx_struct_oper_list.matrix[3][2]
_pdbx_struct_oper_list.matrix[3][3]
_pdbx_struct_oper_list.vector[3]
1 1.0 0.0 0.0 0.0   0.0 1.0 0.0 0.0   0.0 0.0 1.0 0.0
2 -1.0 0.0 0.0 0.0  0.0 1.0 0.0 0.0   0.0 0.0 -1.0 0.0
#
loop_
_atom_site.group_PDB
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_alt_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
_atom_site.auth_seq_id
_atom_site.auth_comp_id
_atom_site.auth_asym_id
_atom_site.auth_atom_id
_atom_site.pdbx_PDB_model_num
ATOM   1 C CA  . VAL A 1 1.0 2.0 3.0 1 VAL A CA  1
ATOM   2 H HA  . VAL A 1 1.5 2.0 3.0 1 VAL A HA  1
ATOM   3 C CA  . ALA B 1 4.0 5.0 6.0 2 ALA B CA  1
HETATM 4 FE FE . HEM A . 7.0 8.0 9.0 101 HEM A FE 1
HETATM 5 O O   . HOH C . 9.0 9.0 9.0 200 HOH A O  1
ATOM   6 C CA  . GLY C 1 10.0 10.0 10.0 3 GLY C CA 1
";
        assert_eq!(inspect_pdb_chains(cif), vec!['A', 'B', 'C']);

        let cmd = "\
read
test.cif
HETATM-----HOH-- 0,9999, 0.5,0.5,0.5, 0.0
ATOM  -H-------- 0,9999, 0.5,0.5,0.5, 0.0
ATOM  -C-------A 0,9999, 1.0,0.6,0.6, 1.6
ATOM  -C-------B 0,9999, 0.6,0.8,1.0, 1.6
HETATMFE---HEM-- 0,9999, 1.0,0.8,0.0, 1.8
END
";
        let deck = CommandDeck::parse(cmd).unwrap();
        let parsed = parse_pdb_reader(cif.as_bytes(), &deck.descriptors).unwrap();
        assert_eq!(parsed.biomats.len(), 2);
        // HA omitted, HOH omitted (label_asym_id=C or HOH), GLY chain C excluded by assembly A,B
        assert_eq!(parsed.atom_count(), 3);
        assert_eq!(parsed.num_subunits, 3); // Chain A, Chain B, then HEM on Chain A
        assert_eq!(parsed.atoms[1].type_id, 3);
        assert_eq!(parsed.atoms[2].type_id, 4);
        assert_eq!(parsed.atoms[3].type_id, 5);
    }
}
