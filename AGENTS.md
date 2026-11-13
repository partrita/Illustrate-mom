# AGENTS.md — Repository Guardrails & Engineering Rules

This document defines the non-negotiable architectural invariants, coding standards, and verification workflows for **Illustrate** (`illustrate` core/CLI and `crates/illustrate-gui` "Molecule of the Month" desktop studio). Follow every rule below before modifying any file in this repository.

---

## 1. Golden Rule: Preserve Pixel-Exact Legacy Fortran Parity (`ADR-0002`)

The default Rust rendering pipeline (`illustrate::render::render` and `illustrate::run_command_script`) **must** produce **0 pixel differences** and **byte-for-byte identical** Netpbm `P3` ASCII output (`2hhb.pnm` and `opacity.pnm`) compared to `gfortran -finit-local-zero legacy/illustrate.f`.

- **Never alter default floating-point precision or constants**:
  - Keep all coordinate, transformation, depth-buffer, shadow, fog, and outline math in single-precision `f32`.
  - Keep `LEGACY_PI = 3.141592` in `src/transform.rs`. Do **not** replace it with `std::f32::consts::PI` or `f64` math in legacy-compatible code paths.
- **Preserve coordinate and rotation conventions (`src/transform.rs`)**:
  - Screen-space origin is upper-left: `+x` points down (vertical rows `ix`), `+y` points left-to-right (horizontal columns `iy`), and `+z` points toward the viewer (clipped at `z = 0`).
  - Rotation cards (`xrot`, `yrot`, `zrot`) negate the angle (`-angle * LEGACY_PI / 180.0`) and concatenate via `catenate`, applying sequential rotations effectively last-to-first prior to centering and translation.
- **Preserve intentional Fortran algorithmic quirks**:
  - **PDB & `REMARK 350` parsing (`src/pdb.rs`)**: Fixed 80-character padded column slicing; `BIOMOLECULE: 1` matches columns `12..25` (matching both `REMARK 300` and `REMARK 350`) and exits sub-loops on the first record where columns `14..19` are six spaces (`"      "`). First matching `AtomDescriptor` wins; `radius == 0.0` omits the atom.
  - **Contour outline kernels 3 & 4 (`src/render.rs`)**: The inner neighborhood loop samples around `(ix, iy)` (not `(ixc, iyc)`), and the 3×3 accumulator `l_buf` intentionally persists across adjacent pixel iterations without resetting to `0.0`.
  - **Canvas sizing & PNM formatting (`src/render.rs`, `src/pnm.rs`)**: Default `render()` clamps dimensions to `3000` and rounds down to even integers (`(size.min(3000) / 2) * 2`). `pnm::write_ppm_p3` must maintain exact Netpbm `P3` formatting (`2i5` width/height header and `20i4` scanlines).
- **Opt-in extensions only**:
  - Any modernized or higher-resolution behavior (such as `render_smooth` and `smooth_canvas_dimensions` in `src/render.rs`) must live in dedicated opt-in functions or flags—never mutate the default `render()` behavior.

---

## 2. Protected Reference & Fixture Files (`ADR-0001`)

- **Do NOT modify, reformat, move, or delete**:
  - `legacy/illustrate.f` — Authoritative Fortran 77 reference source compiled by `tests/parity_2hhb.rs`.
  - `examples/2hhb.inp` and `examples/2hhb.pdb` — Canonical Hemoglobin benchmark inputs embedded via `include_str!` in `crates/illustrate-gui/src/lib.rs` and verified by `tests/parity_2hhb.rs`.
- **Preserve legacy CLI & path resolution compatibility**:
  - Running `illustrate` with no positional arguments must continue reading a command deck from `stdin` (`cargo run --release < examples/2hhb.inp`).
  - `pdb::resolve_pdb_path` must resolve relative PDB filenames in the order: `CWD -> script directory -> examples/`.

---

## 3. Workspace Boundaries & Zero-Node GUI Architecture (`ADR-0001`, `ADR-0004`)

- **Keep the root `illustrate` crate lightweight and UI-agnostic**:
  - `src/lib.rs` is a pure in-memory library (`command`, `pdb`, `presets`, `transform`, `framebuffer`, `render`, `pnm`, `error`).
  - Do **not** leak `clap` CLI printing into library modules, and do **not** add Tauri/GTK/WebKit dependencies to the root `Cargo.toml`.
- **Keep `crates/illustrate-gui` Zero-Node and self-contained**:
  - The frontend in `crates/illustrate-gui/ui/` (`index.html`, `styles.css`, `app.js`) uses static HTML/CSS and Vanilla JS calling `window.__TAURI__.core.invoke` (`withGlobalTauri: true`).
  - Do **not** introduce `package.json`, `node_modules`, npm/pnpm/yarn, TypeScript build steps, or JS bundlers (Vite, Webpack, etc.). `cargo build --workspace` and `cargo test --workspace` must compile the entire workspace deterministically with Cargo alone.
- **Maintain headless GUI session testability**:
  - Keep `StudioSession` and `GuiDeckState` in `crates/illustrate-gui/src/lib.rs` decoupled from live Tauri window handles so all state transitions, presets, and bidirectional `.inp` serialization remain covered by standard `cargo test`.
- **Preserve Two-Tier Rendering in the Studio**:
  - Continuous mouse drags and slider scrubs must use `fast_preview = true` (downscaled canvas, conical shadows disabled with `icone = 0`), followed by a full-quality (`fast_preview = false`) render on release.

---

## 4. Safety, Memory Model & Code Quality (`ADR-0003`)

- **`#![forbid(unsafe_code)]` is mandatory**:
  - Both `Cargo.toml` and `crates/illustrate-gui/Cargo.toml` enforce `unsafe_code = "forbid"`. Never add `unsafe` blocks or weaken workspace lint levels.
- **FrameBuffer & PdbStructure invariants (`src/framebuffer.rs`, `src/pdb.rs`)**:
  - `FrameBuffer` uses `FRAME_PADDING = 10` (`-10..=size + 10`) so 3×3 and 5×5 outline stencils near image borders never go out of bounds. Active pixels (`1..=ixsize, 1..=iysize`) initialize to `zpix = -10000.0`, `bio = 1`, `atom = 0`, `pix = [0.0; 4]`; guard-band pixels initialize to all zeros.
  - `PdbStructure::atoms` reserves index `0` as the background sentinel (`type_id = 0`, `su = 9999`, `res = 9999`). Real atoms are 1-indexed (`1..=atom_count()`).
- **Error handling**:
  - Return structured `illustrate::Result<T>` (`IllustrateError` in `src/error.rs`) across library APIs. Avoid `.unwrap()` or `.expect()` in non-test library and GUI runtime paths.

---

## 5. Repository Hygiene & Documentation

- **Use ubiquitous domain terminology**: Follow the exact naming and definitions in [`GLOSSARY.md`](GLOSSARY.md) (`CommandDeck`, `AtomDescriptor`, `PdbStructure`, `Subunit`, `BioMatrix`, `FrameBuffer`, `Conical Shadow`, `OutlineParams`, `StudioSession`, `PresetKind`).
- **Architecture Decision Records (`docs/adr/`)**: Before making structural changes or altering any decision documented in `docs/adr/0001`–`0004`, document and justify the change in a new ADR.
- **Keep the working tree clean**: Never commit scratch renders (`*.pnm`, `opacity.pnm`, temporary `*.png` outputs), compiled binaries (`illustrate_ref`), or editor artifacts.

---

## 6. Pre-Commit Verification Checklist

Every change **must** pass the full CI verification suite locally before completion:

```bash
# 1. Verify formatting across the entire workspace
cargo fmt --all -- --check

# 2. Run Clippy across all workspace members and targets with warnings denied
cargo clippy --workspace --all-targets -- -D warnings

# 3. Run all unit tests, GUI session tests, and legacy gfortran parity tests
cargo test --workspace --all-targets
```
