# Domain Glossary (`GLOSSARY.md`)

Ubiquitous domain vocabulary for **Illustrate**, shared across code, tests, and Architecture Decision Records (ADRs).

---

## Core Input & Selection Concepts

### Command Deck (`CommandDeck`)
An ordered sequence of 3-character-prefixed **Command Cards** (`read`, `center`, `trans`, `xrot`, `yrot`, `zrot`, `scale`, `world`, `illustrate`, `calculate`) and their parameter lines that configure molecule loading, orientation, shading, outlines, and output.

### Command Card
A control keyword parsed by its leading 3 characters (`rea`, `cen`, `tra`, `xro`, `yro`, `zro`, `sca`, `wor`, `ill`, `cal`), followed by one or more parameter lines.

### Atom Descriptor (`AtomDescriptor`)
A selection and rendering rule inside a `read` block consisting of:
- `record_prefix` (6 chars): `"ATOM  "` or `"HETATM"` matched against PDB columns 1–6.
- `pattern` (10 chars): matched against PDB columns 13–22 (`'-'` is a wildcard).
- `res_range`: inclusive residue number bounds `[low, high]`.
- `color`: RGB reflectance in `[0.0, 1.0]`.
- `radius`: atomic radius in Angstroms (`0.0` omits matching atoms).
Descriptors are evaluated sequentially; the **first matching descriptor** assigns the atom's type.

---

## Molecular Structure & Geometry

### PDB Structure (`PdbStructure`)
The parsed macromolecular coordinate set (`Atom` records loaded from `.pdb`/`.ent` or `.cif`/`.mmcif`) together with biological assembly metadata (`bio_chains`, `biomats`) and automatic **Subunit** assignments. Sentinel index `0` represents the background (`type_id = 0`, `su = 9999`, `res = 9999`).

### Subunit (`su`)
An automatically assigned integer identifier incremented on every `MODEL` record and on every transition of the PDB chain identifier (column 22) between saved atoms. Used to draw inter-subunit outlines.

### Biological Assembly (`BIOMT` / `BioMatrix`)
3x4 affine transformation matrices parsed from `REMARK 350 BIOMT1/2/3` under `BIOMOLECULE: 1`, combined with the chain filter from `REMARK 350 ... CHAINS:`. If no `BIOMT` records are present, a single identity matrix is used.

### Left-Handed Coordinate & Rotation Convention
Illustrate renders in screen space with origin at upper-left, `+x` pointing down (vertical rows `ix`), `+y` pointing left-to-right (horizontal columns `iy`), and `+z` pointing toward the viewer (`z = 0` clipping plane). Sequential rotation cards (`xrot`, `yrot`, `zrot`) concatenate via `catenate` so rotations apply effectively last-to-first, prior to centering and translation.

### Centering (`CenterMode`)
- `Auto` (`aut`): centers X/Y on the transformed bounding-box midpoint and shifts Z so the frontmost atom sphere sits `1.0` Angstrom behind the `z = 0` image plane (`ztranc = -zmax - radius_max - 1.0`).
- `Center` (`cen`): centers X, Y, and Z on the bounding-box midpoint.
- `None`: applies no automatic centering.

---

## Rendering & Non-Photorealistic Pipeline

### Frame Buffer (`FrameBuffer`)
A 2D padded grid spanning `1..=ixsize` (rows) by `1..=iysize` (columns) surrounded by a 10-pixel zero-initialized guard band (`-10..=size + 10`) so outline convolution stencils at image borders execute safely without bounds branches.

### Spherical Shading Map (`sphdat`)
A precomputed list of `(dx, dy, dz)` pixel offsets within a sphere of radius `radius * scale`, used to stamp z-depths onto the depth buffer (`zpix`) for each atom of a given descriptor type.

### Z-Buffer (`zpix`)
Per-pixel maximum depth value (`f32`). Background pixels are initialized to `-10000.0` before sphere rasterization and clamped to `<= 0.0` before post-processing.

### Conical Shadow (Soft Ambient Occlusion)
An ambient-occlusion-like darkening pass controlled by `icone`, `pcone`, `coneangle`, `rcone`, and `pshadowmax`. Each foreground pixel samples neighbors on a 5-pixel grid within radius 50 pixels (`rtable`) and attenuates brightness when neighboring z-depths rise above a cone of angle `coneangle`.

### Depth Cueing (Fog)
Linear depth-based interpolation between the shaded atom color and `rfog`, controlled by front transparency `pfogh` (at `zpix_max`) and back transparency `pfogl` (at `zpix_min`).

### Outlines (`OutlineParams`)
Three non-photorealistic ink-line passes combined via `max` opacity before compositing black ink (`(1.0 - l_opacity) * rcolor`):
1. **Contour Outlines (`l_opacity`)**: Computed from local z-buffer derivatives using `ikernel` (`1..=4`) and thresholds `l_low..l_high`.
2. **Subunit Outlines (`r_opacity`)**: Drawn across boundaries where neighboring pixels belong to different `su` or `bio` indices (`r_low..r_high`).
3. **Residue Outlines (`g_opacity`)**: Drawn across boundaries where neighboring pixels differ in residue sequence number by more than `resdiff` (`g_low..g_high`).

### PNM / Opacity Output
Netpbm P3 ASCII PPM output (`20i4` scanline formatting) produced for both the rendered color image (`calculate` filename) and the silhouette/ink alpha mask (`opacity.pnm`), with optional RGBA PNG export.

---

## Desktop Studio Concepts (*Molecule of the Month*)

### Molecule of the Month Studio (`StudioSession`)
The Tauri v2 desktop workspace (`crates/illustrate-gui`, binary `molecule-of-the-month`) wrapping the core `illustrate` library. Maintains the active PDB text, detected chains, `GuiDeckState` (with explicit `[xrot, yrot, zrot]` Euler angles), and cached full-resolution `RenderResult`.

### Two-Tier Rendering (`Fast Preview` vs `Full Quality`)
Interactive rendering strategy used during continuous mouse drags or slider scrubbing:
- **Fast Preview**: Downscales canvas resolution (~45% scale) and disables conical shadows (`icone = 0`) for low-latency feedback.
- **Full Quality**: Renders at full configured resolution with conical shadows enabled upon mouse/slider release or explicit script application.

### Signature Color Preset (`PresetKind`)
Built-in style generators (`GoodsellClassic`, `SubunitPastel`, `Monochrome1992`, `CpkSoft`) that inspect a PDB's chains and synthesize `AtomDescriptor`, `WorldParams`, and `OutlineParams` configurations.

### Viewport Layer (`RGBA`, `RGB`, `Opacity`)
Display mode selector in the studio canvas showing either the transparent RGBA composite (`rgb` + `opacity` alpha), opaque RGB image (`rgb`), or grayscale silhouette/ink mask (`opacity`).
