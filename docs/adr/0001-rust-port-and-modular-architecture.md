# ADR-0001: Rust Port and Modular Architecture

- **Status**: Accepted
- **Date**: 2026-11-13

## Context
The original `illustrate.f` is a 956-line monolithic Fortran 77 program with global static buffers, `goto`-based control flow, zero-checking fixed-width reads, and flat repository layout (`illustrate.f`, `2hhb.inp`, `2hhb.pdb`, `.travis.yml` all in the root directory). While its rendering output is authoritative, maintaining, testing, or embedding the renderer as a library in modern workflows is difficult.

## Decision
1. **Reorganize the repository** into a standard Cargo project layout:
   - `src/`: Library + binary crate (`lib.rs`, `main.rs`) separated by domain responsibilities (`command`, `pdb`, `transform`, `framebuffer`, `render`, `pnm`, `error`).
   - `examples/`: Sample inputs (`2hhb.inp`, `2hhb.pdb`), kept unmodified.
   - `legacy/`: Original `illustrate.f` preserved for reference and automated regression parity testing.
   - `tests/`: End-to-end integration parity suite comparing Rust output against `gfortran`.
   - `docs/adr/` and `GLOSSARY.md`: Living architecture and domain documentation.
   - `.github/workflows/ci.yml`: Replaces deprecated `.travis.yml`.
2. **Pure in-memory library boundary**:
   `src/lib.rs` exposes `CommandDeck::parse`, `parse_pdb_reader`, `render`, and `run_command_script` without coupling rendering logic to CLI argument parsing or stdout/file I/O.
3. **Hybrid CLI with backward compatibility**:
   Running `illustrate` without positional arguments reads a command deck from `stdin` (`illustrate < examples/2hhb.inp`), while `clap` flags allow specifying an input file path, output overrides (`--output`, `--opacity-output`, `--no-opacity`), and RGBA PNG export (`--png`).
4. **Fallback PDB path resolution**:
   Relative PDB paths in legacy `.inp` files (e.g., `2hhb.pdb` inside `examples/2hhb.inp`) are resolved in order: `CWD -> script directory -> examples/`, keeping original input decks unmodified.

## Consequences
- Every stage (command parsing, PDB classification, matrix transformation, frame buffer rasterization, PNM/PNG encoding) is independently unit-testable.
- Unsafe code is forbidden (`#![forbid(unsafe_code)]` via `Cargo.toml` lints).
- Existing legacy pipelines using `stdin` redirection continue to work without modification.
