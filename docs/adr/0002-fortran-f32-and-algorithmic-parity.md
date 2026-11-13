# ADR-0002: Fortran `f32` Precision and Algorithmic Parity

- **Status**: Accepted
- **Date**: 2026-11-13

## Context
During inspection of `legacy/illustrate.f`, several 1990s Fortran implementation quirks were identified that directly affect pixel values:
1. All floating-point arithmetic uses single precision (`real*4` / `f32`) and `LEGACY_PI = 3.141592`.
2. Rotation commands (`xrot`, `yrot`, `zrot`) negate the angle (`-angle * 3.141592 / 180.0`) and accumulate into `rm` via `catenate`, applying sequential rotations effectively last-to-first.
3. In `REMARK 350` biological assembly parsing, the `BIOMOLECULE: 1` loop matches both `REMARK 300 BIOMOLECULE: 1` and `REMARK 350 BIOMOLECULE: 1` (because column slice `12:25` equals `"BIOMOLECULE: 1"`), exiting each sub-loop on the first record whose columns `14:19` are six spaces.
4. In contour outline kernels 3 and 4 (`illustrate.f` lines 805–836):
   - The inner neighborhood loop samples around `(ix, iy)` rather than `(ixc, iyc)`.
   - The 3x3 accumulator matrix `l(-1:1, -1:1)` is declared at program scope and is not reset to `0.0` before accumulating `rd` for kernels 3 and 4, carrying clamped `[0.0, 1.0]` values across adjacent pixel iterations.
5. Output `.pnm` files are formatted as Netpbm `P3` ASCII PPM with `2i5` headers and `20i4` scanline rows.

## Decision
Preserve **exact `f32` arithmetic and original Fortran algorithmic semantics** as the default rendering behavior, including:
- `f32` precision, `LEGACY_PI = 3.141592`, left-handed rotation sign, and `catenate` accumulation order.
- Exact `BIOMOLECULE: 1` and `ATOM`/`HETATM` fixed-column (`80`-char padded) parsing rules.
- Exact kernel 1–4 equations and persistent `l_buf` accumulator semantics.
- Exact `P3` `2i5` / `20i4` ASCII formatting in `pnm::write_ppm_p3`.

## Consequences
- Automated parity tests verify **0 pixel differences** (and byte-for-byte identical `2hhb.pnm` and `opacity.pnm` files) against `gfortran legacy/illustrate.f`.
- Any future "modernized" or "corrected" kernel behavior (e.g., resetting `l_buf` or centering kernels 3/4 on `(ixc, iyc)`) must be introduced behind an explicit opt-in flag rather than altering default output.
