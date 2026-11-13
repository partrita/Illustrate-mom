//! Biomolecular non-photorealistic renderer ported from David S. Goodsell's `illustrate.f`.
//!
//! This crate exposes both a reusable in-memory rendering pipeline (`command`, `pdb`, `render`,
//! `pnm`) and a CLI binary compatible with legacy `illustrate < command_file` decks.

pub mod command;
pub mod error;
pub mod framebuffer;
pub mod pdb;
pub mod pnm;
pub mod presets;
pub mod render;
pub mod transform;

use std::path::Path;

pub use command::{AtomDescriptor, CenterMode, CommandDeck, OutlineParams, WorldParams};
pub use error::{IllustrateError, Result};
pub use pdb::{
    Atom, PdbStructure, inspect_pdb_chains, load_pdb_file, parse_pdb_reader, resolve_pdb_path,
};
pub use presets::{PresetKind, StylePreset, build_preset};
pub use render::{RenderResult, render};

/// Parses a command script, resolves and loads its PDB structure, and executes rendering.
pub fn run_command_script(
    script: &str,
    script_dir: Option<&Path>,
) -> Result<(CommandDeck, PdbStructure, RenderResult)> {
    let deck = CommandDeck::parse(script)?;
    let resolved_pdb = resolve_pdb_path(&deck.pdb_path, script_dir)?;
    let pdb = load_pdb_file(&resolved_pdb, &deck.descriptors)?;
    let result = render(&deck, &pdb)?;
    Ok((deck, pdb, result))
}
