# ADR-0005: RCSB Structure Download and Session Persistence in the GUI

- **Status**: Accepted
- **Date**: 2026-11-13

## Context
The "Molecule of the Month" GUI (ADR-0004) is aimed at researchers producing figures. Two frictions dominate first use:
1. Getting a structure requires locating and downloading a PDB/mmCIF file by hand.
2. Closing the app discards the camera, preset, and atom-rule work in progress.

Both need capabilities the GUI did not have: outbound network access and writing application data to disk.

## Decision
1. **Network access is confined to `crates/illustrate-gui`.**
   - Add `ureq` (rustls TLS) only to `crates/illustrate-gui/Cargo.toml`. The root `illustrate` crate stays free of network dependencies (ADR-0001).
   - A PDB ID (`^[0-9][A-Za-z0-9]{3}$`) is downloaded from `https://files.rcsb.org/download/<ID>.pdb`; on HTTP 404 the `.cif` file is tried (large structures have no legacy `.pdb`).
   - Downloads run off the UI thread (`spawn_blocking`) and are cached on disk under the Tauri app cache directory (`pdb/<ID>.pdb|.cif`) so a repeated ID never hits the network.
   - The fetch logic takes an injectable fetcher so URL building, validation, fallback, and caching are covered by headless `cargo test` without network.
2. **Session persistence is a versioned JSON file in the Tauri app data directory** (`session.json`).
   - Stores: structure name, structure text (a copy, so it survives the original file moving), `GuiDeckState`, and the active preset id.
   - Written when the main window requests close; read at startup.
   - If the file is missing, unreadable, or has an unknown `version`, the app starts on the empty "load a structure" screen (with a non-blocking notice if the file was broken). No "resume?" prompt is shown.
3. **Undo history is not persisted.**

## Consequences
- The app can now make outbound HTTPS requests; only the RCSB file host is contacted, and only on explicit user action.
- `AGENTS.md` invariants are unaffected: default `render()` output, the `2hhb` fixtures, Zero-Node frontend, and `#![forbid(unsafe_code)]` are unchanged.
- Failures (offline, unknown ID) fall back to the existing file-open flow and are reported in the status bar.
- Session JSON embeds the structure text, so it can be several MB for large assemblies.
