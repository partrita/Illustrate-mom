# ADR-0004: Tauri v2 Desktop GUI Architecture ("Molecule of the Month")

- **Status**: Accepted
- **Date**: 2026-11-13

## Context
Legacy `illustrate` workflows required hand-editing fixed-column `.inp` scripts and running CLI commands to inspect output `.pnm` files. To provide an interactive visual studio without duplicating rendering logic or introducing a heavy Node.js build pipeline, we need a native desktop UI tightly coupled to the Rust core library.

## Decision
1. **Cargo Workspace Separation (`crates/illustrate-gui`)**:
   - Promote the root `Cargo.toml` to a workspace containing `.` (`illustrate` core + CLI) and `crates/illustrate-gui` (`molecule-of-the-month` binary).
   - Core CLI builds and parity tests remain unaffected by GTK/WebKitGUI dependencies.
2. **Zero-Node Static Frontend (`ui/`) with `withGlobalTauri: true`**:
   - Author the UI in `crates/illustrate-gui/ui/` (`index.html`, `styles.css`, `app.js`) using modern CSS Grid/Variables and Vanilla JS invoking `window.__TAURI__.core.invoke`.
   - `cargo build` / `cargo test` compiles the entire desktop application deterministically without `npm` or JS bundlers.
3. **Two-Tier Rendering & Bidirectional `.inp` Synchronization**:
   - Extend `illustrate::transform::Matrix4` with `from_euler_angles_deg` and `to_euler_angles_deg`, and `CommandDeck` with `serde` derives and `to_script()`.
   - During continuous drag/slider gestures, the GUI invokes `render_scene(..., fast_preview = true)` (downscaled resolution, `icone = 0`), automatically scheduling a full-resolution `fast_preview = false` render when interaction settles.
   - Four built-in **Molecule of the Month** style presets (`GoodsellClassic`, `SubunitPastel`, `Monochrome1992`, `CpkSoft`) live in `illustrate::presets` so both the GUI and future CLI/library callers share them.

## Consequences
- Users can load `.pdb` or `.inp` files, drag to rotate, edit atom descriptors visually, inspect RGB/RGBA/Opacity layers, and export PNG/PNM/`.inp` outputs in one desktop app.
- All session logic (`StudioSession`) is decoupled from Tauri window handles and directly covered by unit tests.
