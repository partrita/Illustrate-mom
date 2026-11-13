# ADR-0003: Dynamic Padded Frame Buffer

- **Status**: Accepted
- **Date**: 2026-11-13

## Context
The Fortran implementation declares large static arrays at program startup:
- `pix(-10:3008, -10:3008, 4)` (~145 MB)
- `zpix(-10:3008, -10:3008)` (~36 MB)
- `atom(-10:3008, -10:3008)` and `bio(-10:3008, -10:3008)` (~72 MB)
- `coord(3, 350000)`, `type(350000)`, `res(0:350000)`, `su(0:350000)`

The negative/extra indices (`-10..3008`) act as an implicit zero-initialized guard band so 5x5 and 3x3 outline stencils near image edges (`ix = 2..3`) never read out of bounds. However, static ~260 MB allocations waste memory for small images and impose a hard 350,000-atom ceiling.

## Decision
1. **Dynamic `Vec`-backed `FrameBuffer`**:
   Allocate only `(ixsize + 2 * FRAME_PADDING + 1) * (iysize + 2 * FRAME_PADDING + 1)` elements per render, where `FRAME_PADDING = 10`.
   - Active coordinates `(1..=ixsize, 1..=iysize)` are initialized to `zpix = -10000.0`, `bio = 1`, `atom = 0`, `pix = [0.0; 4]`.
   - Guard-band coordinates outside the active frame are zero-initialized (`zpix = 0.0`, `bio = 0`, `atom = 0`, `pix = [0.0; 4]`), matching Fortran static initialization.
2. **Dynamic `Vec<Atom>` with sentinel index `0`**:
   Store an arbitrary number of atoms in `PdbStructure::atoms`, reserving index `0` as the background sentinel (`su = 9999`, `res = 9999`, `type_id = 0`).
3. **Preserve default 3000-pixel clamp and even alignment**:
   Maintain `ixsize = (ixsize.min(3000) / 2) * 2` and `iysize = (iysize.min(3000) / 2) * 2` for full compatibility with existing command decks.

## Consequences
- Memory consumption scales proportionally to actual image dimensions and atom count.
- The 350,000-atom limit is removed while retaining memory-safe bounds checking (`debug_assert!`) and identical border behavior.
