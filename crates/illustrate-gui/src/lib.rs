use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use illustrate::command::{AtomDescriptor, CenterMode, CommandDeck, OutlineParams, WorldParams};
use illustrate::pdb::{inspect_pdb_chains, parse_pdb_reader, resolve_pdb_path};
use illustrate::pnm::{
    encode_png_rgb, encode_png_rgba, encode_png_with_options, save_png_file, save_ppm_file,
};
use illustrate::presets::{PresetKind, build_preset};
use illustrate::render::{
    RenderResult, SMOOTH_MAX_CANVAS, render, render_smooth, smooth_canvas_dimensions,
};
use illustrate::transform::Matrix4;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

pub mod fetch;

const DEFAULT_INP: &str = include_str!("../../../examples/2hhb.inp");
const DEFAULT_PDB: &str = include_str!("../../../examples/2hhb.pdb");

/// Editable DTO representation of [`AtomDescriptor`] for JSON IPC with the webview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomDescriptorDto {
    pub record: String,
    pub pattern: String,
    pub res_low: i32,
    pub res_high: i32,
    pub color: [f32; 3],
    pub radius: f32,
}

impl From<&AtomDescriptor> for AtomDescriptorDto {
    fn from(d: &AtomDescriptor) -> Self {
        Self {
            record: d.record_prefix_str(),
            pattern: d.pattern_str(),
            res_low: d.res_range.0,
            res_high: d.res_range.1,
            color: d.color,
            radius: d.radius,
        }
    }
}

impl AtomDescriptorDto {
    fn to_domain(&self) -> Result<AtomDescriptor, String> {
        AtomDescriptor::from_strings(
            &self.record,
            &self.pattern,
            (self.res_low, self.res_high),
            self.color,
            self.radius,
        )
        .map_err(|e| e.to_string())
    }
}

/// Complete GUI-editable command deck state with explicit Euler angles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuiDeckState {
    pub pdb_name: String,
    pub descriptors: Vec<AtomDescriptorDto>,
    pub translation: [f32; 3],
    pub euler_deg: [f32; 3],
    pub scale: f32,
    pub center_mode: CenterMode,
    pub world: WorldParams,
    pub outline: OutlineParams,
    pub output_name: String,
}

impl GuiDeckState {
    fn from_command_deck(deck: &CommandDeck) -> Self {
        Self {
            pdb_name: deck.pdb_path.display().to_string(),
            descriptors: deck
                .descriptors
                .iter()
                .map(AtomDescriptorDto::from)
                .collect(),
            translation: deck.translation,
            euler_deg: deck.rotation.to_euler_angles_deg(),
            scale: deck.scale,
            center_mode: deck.center_mode,
            world: deck.world.clone(),
            outline: deck.outline.clone(),
            output_name: deck
                .output_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "illustrate.pnm".to_string()),
        }
    }

    fn to_command_deck(&self) -> Result<CommandDeck, String> {
        let descriptors = self
            .descriptors
            .iter()
            .map(AtomDescriptorDto::to_domain)
            .collect::<Result<Vec<_>, _>>()?;

        let rotation =
            Matrix4::from_euler_angles_deg(self.euler_deg[0], self.euler_deg[1], self.euler_deg[2]);

        Ok(CommandDeck {
            pdb_path: PathBuf::from(&self.pdb_name),
            descriptors,
            translation: self.translation,
            rotation,
            scale: self.scale.max(0.5),
            center_mode: self.center_mode,
            world: self.world.clone(),
            outline: self.outline.clone(),
            output_path: Some(PathBuf::from(&self.output_name)),
        })
    }
}

/// Image buffers and rendering metrics returned to the frontend viewport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderResponse {
    pub width: usize,
    pub height: usize,
    pub display_width: usize,
    pub display_height: usize,
    pub atom_count: usize,
    pub subunit_count: i32,
    pub biomat_count: usize,
    pub z_range: (f32, f32),
    pub duration_ms: u64,
    pub fast_preview: bool,
    pub script: String,
    pub rgba_data_url: String,
    pub rgb_data_url: String,
    pub opacity_data_url: String,
}

/// Combined state snapshot returned when initializing, loading files, or switching presets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub deck: GuiDeckState,
    pub chains: Vec<char>,
    pub render: RenderResponse,
    /// Identifier of the style preset most recently applied (e.g. `goodsell_classic`).
    pub preset: String,
}

/// Current on-disk session format version (see ADR-0005).
const SESSION_VERSION: u32 = 1;

/// Serializable form of a [`StudioSession`] stored as `session.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSession {
    version: u32,
    pdb_text: String,
    deck: GuiDeckState,
    preset: String,
}

/// Identifiers of the built-in style presets shown in the preset gallery.
pub const PRESET_IDS: &[&str] = &[
    "goodsell_classic",
    "subunit_pastel",
    "monochrome_1992",
    "cpk_soft",
];

/// Preset applied to freshly loaded sessions.
const DEFAULT_PRESET: &str = "goodsell_classic";

/// Maps any identifier to a known preset id (unknown ids fall back to the default).
fn canonical_preset_id(id: &str) -> String {
    PRESET_IDS
        .iter()
        .find(|known| **known == id)
        .copied()
        .unwrap_or(DEFAULT_PRESET)
        .to_string()
}

/// Linear downscale factor applied to preset gallery thumbnails.
const THUMB_FACTOR: f32 = 0.25;

/// Downscales an explicit canvas size or padding value for thumbnail rendering.
fn scale_canvas_size(size: i32) -> i32 {
    let scaled = (size as f32 * THUMB_FACTOR) as i32;
    if size > 0 {
        scaled.max(32)
    } else {
        scaled.min(-2)
    }
}

/// Smallest accepted export width in pixels.
const MIN_EXPORT_WIDTH: u32 = 64;

/// User-selected PNG export settings from the export dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PngExportOptions {
    /// Target image width in pixels; `None` exports at the current canvas size (1x).
    pub width_px: Option<u32>,
    /// Keep the opacity channel as PNG alpha (`true`) or write an opaque RGB image.
    pub transparent: bool,
    /// Print resolution recorded in the PNG `pHYs` chunk.
    pub dpi: Option<u32>,
}

/// Small preview image of one preset applied to the current structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetThumbnail {
    pub id: String,
    pub data_url: String,
}

/// Mutable server-side studio session protected by a `Mutex` in Tauri state.
pub struct StudioSession {
    pdb_text: String,
    chains: Vec<char>,
    deck_state: GuiDeckState,
    last_full_deck: Option<CommandDeck>,
    last_full_render: Option<RenderResult>,
    active_preset: String,
    startup_notice: Option<String>,
}

impl StudioSession {
    /// Initializes an empty session with no loaded structure.
    #[must_use]
    pub fn new_empty() -> Self {
        let preset = build_preset(PresetKind::GoodsellClassic, &[], (-30, -30));
        let deck_state = GuiDeckState {
            pdb_name: String::new(),
            descriptors: preset
                .descriptors
                .iter()
                .map(AtomDescriptorDto::from)
                .collect(),
            translation: [0.0, 0.0, 0.0],
            euler_deg: [0.0, 0.0, 0.0],
            scale: 12.0,
            center_mode: CenterMode::Auto,
            world: preset.world,
            outline: preset.outline,
            output_name: "illustrate.pnm".to_string(),
        };
        Self {
            pdb_text: String::new(),
            chains: Vec::new(),
            deck_state,
            last_full_deck: None,
            last_full_render: None,
            active_preset: DEFAULT_PRESET.to_string(),
            startup_notice: None,
        }
    }

    /// Initializes a session preloaded with the bundled `2hhb` hemoglobin example.
    pub fn new_default() -> Result<Self, String> {
        let deck = CommandDeck::parse(DEFAULT_INP).map_err(|e| e.to_string())?;
        let chains = inspect_pdb_chains(DEFAULT_PDB);
        let deck_state = GuiDeckState::from_command_deck(&deck);
        Ok(Self {
            pdb_text: DEFAULT_PDB.to_string(),
            chains,
            deck_state,
            last_full_deck: Some(deck),
            last_full_render: None,
            active_preset: DEFAULT_PRESET.to_string(),
            startup_notice: None,
        })
    }

    /// Writes the session to `path` as JSON; does nothing when no structure is loaded.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if !self.has_structure() {
            return Ok(());
        }
        let persisted = PersistedSession {
            version: SESSION_VERSION,
            pdb_text: self.pdb_text.clone(),
            deck: self.deck_state.clone(),
            preset: self.active_preset.clone(),
        };
        let json = serde_json::to_string(&persisted).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(path, json).map_err(|e| e.to_string())
    }

    /// Reads a session previously written by [`StudioSession::save_to`].
    pub fn restore_from(path: &Path) -> Result<Self, String> {
        let json = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let persisted: PersistedSession =
            serde_json::from_str(&json).map_err(|e| format!("corrupt session file: {e}"))?;
        if persisted.version != SESSION_VERSION {
            return Err(format!("unsupported session version {}", persisted.version));
        }
        if persisted.pdb_text.trim().is_empty() {
            return Err("session file contains no structure".to_string());
        }
        persisted.deck.to_command_deck()?;
        let chains = inspect_pdb_chains(&persisted.pdb_text);
        Ok(Self {
            pdb_text: persisted.pdb_text,
            chains,
            deck_state: persisted.deck,
            last_full_deck: None,
            last_full_render: None,
            active_preset: persisted.preset,
            startup_notice: None,
        })
    }

    /// Builds the startup session: restores `path` if possible, otherwise an empty session.
    ///
    /// A missing file silently yields the empty state; a broken one also sets a startup notice.
    #[must_use]
    pub fn startup(path: Option<&Path>) -> Self {
        let mut notice = None;
        if let Some(path) = path
            && path.exists()
        {
            match Self::restore_from(path) {
                Ok(session) => return session,
                Err(e) => {
                    notice = Some(format!(
                        "Could not restore the last session ({e}); starting empty."
                    ));
                }
            }
        }
        let mut session = Self::new_empty();
        session.startup_notice = notice;
        session
    }

    /// Returns and clears the pending startup notice, if any.
    pub fn take_startup_notice(&mut self) -> Option<String> {
        self.startup_notice.take()
    }

    /// Returns `true` if molecular structure data has been loaded into the session.
    #[must_use]
    pub fn has_structure(&self) -> bool {
        !self.pdb_text.trim().is_empty()
    }

    fn execute_render(
        &mut self,
        gui_deck: GuiDeckState,
        fast_preview: bool,
    ) -> Result<RenderResponse, String> {
        if !self.has_structure() {
            return Err(
                "No molecular structure loaded. Open a PDB or mmCIF file first.".to_string(),
            );
        }

        let start = Instant::now();
        let full_deck = gui_deck.to_command_deck()?;
        let script = full_deck.to_script(Some(gui_deck.euler_deg));

        let mut render_deck = full_deck.clone();
        let preview_factor = if fast_preview { 0.5_f32 } else { 1.0_f32 };
        if fast_preview {
            render_deck.scale = (render_deck.scale * preview_factor).max(2.0);
            render_deck.world.icone = 0;
            if render_deck.world.ixsize > 0 {
                render_deck.world.ixsize =
                    ((render_deck.world.ixsize as f32 * preview_factor) as i32).max(64);
            } else {
                render_deck.world.ixsize =
                    ((render_deck.world.ixsize as f32 * preview_factor) as i32).min(-8);
            }
            if render_deck.world.iysize > 0 {
                render_deck.world.iysize =
                    ((render_deck.world.iysize as f32 * preview_factor) as i32).max(64);
            } else {
                render_deck.world.iysize =
                    ((render_deck.world.iysize as f32 * preview_factor) as i32).min(-8);
            }
        }

        let pdb = parse_pdb_reader(self.pdb_text.as_bytes(), &render_deck.descriptors)
            .map_err(|e| e.to_string())?;
        let (display_width, display_height) = if fast_preview {
            smooth_canvas_dimensions(&full_deck, &pdb).map_err(|e| e.to_string())?
        } else {
            (0, 0)
        };
        let result = render_smooth(&render_deck, &pdb).map_err(|e| e.to_string())?;
        let (display_width, display_height) = if fast_preview {
            (display_width, display_height)
        } else {
            (result.width, result.height)
        };

        let rgba_png = encode_png_rgba(&result).map_err(|e| e.to_string())?;
        let rgb_png =
            encode_png_rgb(result.width, result.height, &result.rgb).map_err(|e| e.to_string())?;
        let op_png = encode_png_rgb(result.width, result.height, &result.opacity)
            .map_err(|e| e.to_string())?;

        let duration_ms = start.elapsed().as_millis() as u64;
        let response = RenderResponse {
            width: result.width,
            height: result.height,
            display_width,
            display_height,
            atom_count: pdb.atom_count(),
            subunit_count: pdb.num_subunits,
            biomat_count: pdb.biomats.len().max(1),
            z_range: result.z_range,
            duration_ms,
            fast_preview,
            script,
            rgba_data_url: format!("data:image/png;base64,{}", BASE64.encode(rgba_png)),
            rgb_data_url: format!("data:image/png;base64,{}", BASE64.encode(rgb_png)),
            opacity_data_url: format!("data:image/png;base64,{}", BASE64.encode(op_png)),
        };

        self.deck_state = gui_deck;
        if !fast_preview {
            self.last_full_deck = Some(full_deck);
            self.last_full_render = Some(result);
        }

        Ok(response)
    }

    /// Returns a full snapshot of the current session, rendering if needed.
    pub fn snapshot(&mut self) -> Result<SessionSnapshot, String> {
        let deck = self.deck_state.clone();
        let render = self.execute_render(deck.clone(), false)?;
        Ok(SessionSnapshot {
            deck,
            chains: self.chains.clone(),
            render,
            preset: self.active_preset.clone(),
        })
    }

    /// Parses a `.inp` script, updates state (optionally swapping structure text), and renders.
    pub fn load_script(
        &mut self,
        script: &str,
        script_dir: Option<&Path>,
    ) -> Result<SessionSnapshot, String> {
        let deck = CommandDeck::parse(script).map_err(|e| e.to_string())?;
        if let Ok(resolved) = resolve_pdb_path(&deck.pdb_path, script_dir)
            && let Ok(pdb_content) = fs::read_to_string(&resolved)
        {
            self.chains = inspect_pdb_chains(&pdb_content);
            self.pdb_text = pdb_content;
        }
        if !self.has_structure() {
            return Err(format!(
                "Could not locate '{}' referenced by the .inp script. Load a PDB/mmCIF structure first.",
                deck.pdb_path.display()
            ));
        }
        self.last_full_render = None;
        self.deck_state = GuiDeckState::from_command_deck(&deck);
        self.snapshot()
    }

    /// Loads new PDB or mmCIF text and applies the selected Molecule of the Month preset.
    pub fn load_pdb(
        &mut self,
        filename: String,
        pdb_content: String,
        preset_id: &str,
    ) -> Result<SessionSnapshot, String> {
        let chains = inspect_pdb_chains(&pdb_content);
        let preset = build_preset(PresetKind::from_id(preset_id), &chains, (-30, -30));
        let stem = Path::new(&filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("molecule");
        let output_name = format!("{stem}.pnm");

        self.pdb_text = pdb_content;
        self.chains = chains;
        self.last_full_render = None;
        self.active_preset = canonical_preset_id(preset_id);
        self.deck_state = GuiDeckState {
            pdb_name: filename,
            descriptors: preset
                .descriptors
                .iter()
                .map(AtomDescriptorDto::from)
                .collect(),
            translation: [0.0, 0.0, 0.0],
            euler_deg: [0.0, 0.0, 0.0],
            scale: 12.0,
            center_mode: CenterMode::Auto,
            world: preset.world,
            outline: preset.outline,
            output_name,
        };

        self.snapshot()
    }

    /// Renders the current deck at the requested export size and writes a PNG.
    ///
    /// Sizes above the legacy 3000 px limit are produced through the opt-in smooth renderer,
    /// up to [`SMOOTH_MAX_CANVAS`] pixels per edge.
    pub fn export_png_with_options(
        &mut self,
        path: &Path,
        options: &PngExportOptions,
    ) -> Result<String, String> {
        if !self.has_structure() {
            return Err("No molecular structure loaded.".to_string());
        }
        let mut deck = self.deck_state.to_command_deck()?;
        let pdb = parse_pdb_reader(self.pdb_text.as_bytes(), &deck.descriptors)
            .map_err(|e| e.to_string())?;
        let (base_w, base_h) = smooth_canvas_dimensions(&deck, &pdb).map_err(|e| e.to_string())?;

        let limit = SMOOTH_MAX_CANVAS as u32;
        let target_w = options.width_px.unwrap_or(base_w as u32);
        if !(MIN_EXPORT_WIDTH..=limit).contains(&target_w) {
            return Err(format!(
                "export width must be between {MIN_EXPORT_WIDTH} and {limit} px"
            ));
        }
        let factor = target_w as f32 / base_w as f32;
        if (base_h as f32 * factor).round() > limit as f32 {
            return Err(format!("export height would exceed {limit} px"));
        }
        if (factor - 1.0).abs() > f32::EPSILON {
            deck.scale *= factor;
            deck.world.ixsize = (deck.world.ixsize as f32 * factor).round() as i32;
            deck.world.iysize = (deck.world.iysize as f32 * factor).round() as i32;
        }

        let result = render_smooth(&deck, &pdb).map_err(|e| e.to_string())?;
        let bytes = encode_png_with_options(&result, options.transparent, options.dpi)
            .map_err(|e| e.to_string())?;
        fs::write(path, bytes).map_err(|e| e.to_string())?;
        Ok(format!(
            "Saved {}×{} px PNG to {}",
            result.width,
            result.height,
            path.display()
        ))
    }

    /// Renders a small thumbnail of the current structure for every built-in preset.
    ///
    /// The session state is left untouched; thumbnails use the current camera
    /// orientation with a reduced scale and no conical shadow.
    pub fn preset_thumbnails(&self) -> Result<Vec<PresetThumbnail>, String> {
        if !self.has_structure() {
            return Err("No molecular structure loaded.".to_string());
        }
        let pad = (self.deck_state.world.ixsize, self.deck_state.world.iysize);
        let mut thumbs = Vec::with_capacity(PRESET_IDS.len());
        for id in PRESET_IDS {
            let preset = build_preset(PresetKind::from_id(id), &self.chains, pad);
            let mut gui = self.deck_state.clone();
            gui.descriptors = preset
                .descriptors
                .iter()
                .map(AtomDescriptorDto::from)
                .collect();
            gui.world = preset.world;
            gui.outline = preset.outline;

            let mut deck = gui.to_command_deck()?;
            deck.scale = (deck.scale * THUMB_FACTOR).max(1.0);
            deck.world.icone = 0;
            deck.world.ixsize = scale_canvas_size(deck.world.ixsize);
            deck.world.iysize = scale_canvas_size(deck.world.iysize);

            let pdb = parse_pdb_reader(self.pdb_text.as_bytes(), &deck.descriptors)
                .map_err(|e| e.to_string())?;
            let result = render(&deck, &pdb).map_err(|e| e.to_string())?;
            let png = encode_png_rgba(&result).map_err(|e| e.to_string())?;
            thumbs.push(PresetThumbnail {
                id: (*id).to_string(),
                data_url: format!("data:image/png;base64,{}", BASE64.encode(png)),
            });
        }
        Ok(thumbs)
    }

    /// Applies a color/outline preset to the current PDB structure while keeping camera settings.
    pub fn apply_preset(&mut self, preset_id: &str) -> Result<SessionSnapshot, String> {
        let pad = (self.deck_state.world.ixsize, self.deck_state.world.iysize);
        let preset = build_preset(PresetKind::from_id(preset_id), &self.chains, pad);
        self.deck_state.descriptors = preset
            .descriptors
            .iter()
            .map(AtomDescriptorDto::from)
            .collect();
        self.deck_state.world = preset.world;
        self.deck_state.outline = preset.outline;
        self.active_preset = canonical_preset_id(preset_id);
        self.snapshot()
    }

    /// Exports the latest full-resolution render or command script to disk.
    pub fn export_to_path(&mut self, format: &str, path: &Path) -> Result<String, String> {
        if self.last_full_render.is_none() {
            let deck = self.deck_state.clone();
            self.execute_render(deck, false)?;
        }

        match format {
            "png" => {
                let res = self
                    .last_full_render
                    .as_ref()
                    .ok_or_else(|| "no rendered image available".to_string())?;
                save_png_file(path, res).map_err(|e| e.to_string())?;
                Ok(format!("Saved RGBA PNG to {}", path.display()))
            }
            "pnm" => {
                let res = self
                    .last_full_render
                    .as_ref()
                    .ok_or_else(|| "no rendered image available".to_string())?;
                save_ppm_file(path, res.width, res.height, &res.rgb).map_err(|e| e.to_string())?;
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("illustrate");
                let opacity_path = path.with_file_name(format!("{stem}_opacity.pnm"));
                save_ppm_file(&opacity_path, res.width, res.height, &res.opacity)
                    .map_err(|e| e.to_string())?;
                Ok(format!(
                    "Saved PNM to {} and {}",
                    path.display(),
                    opacity_path.display()
                ))
            }
            "inp" => {
                let deck = self.deck_state.to_command_deck()?;
                let script = deck.to_script(Some(self.deck_state.euler_deg));
                fs::write(path, script).map_err(|e| e.to_string())?;
                Ok(format!("Saved command deck to {}", path.display()))
            }
            other => Err(format!("unsupported export format '{other}'")),
        }
    }
}

/// Managed Tauri state wrapper.
pub struct AppState(pub Mutex<StudioSession>);

#[tauri::command]
fn get_initial_state(state: State<'_, AppState>) -> Result<Option<SessionSnapshot>, String> {
    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    if !session.has_structure() {
        return Ok(None);
    }
    session.snapshot().map(Some)
}

/// Filesystem location of `session.json`, managed as Tauri state.
pub struct SessionPath(pub Option<PathBuf>);

#[tauri::command]
fn take_startup_notice(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    Ok(session.take_startup_notice())
}

#[tauri::command]
async fn fetch_pdb_id(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    preset: String,
) -> Result<SessionSnapshot, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("pdb");
    let fetched = tauri::async_runtime::spawn_blocking(move || {
        fetch::fetch_structure(&id, &cache_dir, &fetch::http_fetch)
    })
    .await
    .map_err(|e| format!("download task failed: {e}"))??;

    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    session.load_pdb(fetched.filename, fetched.content, &preset)
}

#[tauri::command]
fn reset_default_2hhb(state: State<'_, AppState>) -> Result<SessionSnapshot, String> {
    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    *session = StudioSession::new_default()?;
    session.snapshot()
}

#[tauri::command]
fn render_scene(
    state: State<'_, AppState>,
    deck: GuiDeckState,
    fast_preview: bool,
) -> Result<RenderResponse, String> {
    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    session.execute_render(deck, fast_preview)
}

#[tauri::command]
fn parse_and_render_script(
    state: State<'_, AppState>,
    script: String,
) -> Result<SessionSnapshot, String> {
    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    session.load_script(&script, None)
}

#[tauri::command]
fn load_pdb_data(
    state: State<'_, AppState>,
    filename: String,
    content: String,
    preset: String,
) -> Result<SessionSnapshot, String> {
    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    session.load_pdb(filename, content, &preset)
}

#[tauri::command]
fn apply_color_preset(
    state: State<'_, AppState>,
    preset: String,
) -> Result<SessionSnapshot, String> {
    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    session.apply_preset(&preset)
}

#[tauri::command]
async fn preset_thumbnails(state: State<'_, AppState>) -> Result<Vec<PresetThumbnail>, String> {
    let session = state.0.lock().map_err(|e| e.to_string())?;
    session.preset_thumbnails()
}

#[tauri::command]
fn export_output(
    state: State<'_, AppState>,
    format: String,
    path: String,
) -> Result<String, String> {
    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    session.export_to_path(&format, Path::new(&path))
}

#[tauri::command]
async fn pick_and_load_pdb(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    preset: String,
) -> Result<Option<SessionSnapshot>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter(
                "Molecular Structure (PDB / mmCIF)",
                &["pdb", "ent", "cif", "mmcif", "PDB", "ENT", "CIF", "MMCIF"],
            )
            .blocking_pick_file()
    })
    .await
    .map_err(|e| format!("file dialog failed: {e}"))?;

    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path = file_path
        .into_path()
        .map_err(|e| format!("invalid file path: {e}"))?;
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("structure.pdb")
        .to_string();

    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    session.load_pdb(filename, content, &preset).map(Some)
}

#[tauri::command]
async fn pick_and_load_script(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<SessionSnapshot>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Illustrate Command Deck", &["inp", "txt", "INP", "TXT"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| format!("file dialog failed: {e}"))?;

    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path = file_path
        .into_path()
        .map_err(|e| format!("invalid file path: {e}"))?;
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let parent = path.parent();

    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    session.load_script(&content, parent).map(Some)
}

#[tauri::command]
async fn pick_and_export(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    format: String,
) -> Result<Option<String>, String> {
    let (filter_name, ext, default_name) = match format.as_str() {
        "png" => ("PNG Image", "png", "illustration.png"),
        "pnm" => ("Netpbm PPM", "pnm", "illustration.pnm"),
        "inp" => ("Illustrate Script", "inp", "illustration.inp"),
        other => return Err(format!("unsupported format '{other}'")),
    };

    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter(filter_name, &[ext])
            .set_file_name(default_name)
            .blocking_save_file()
    })
    .await
    .map_err(|e| format!("export dialog failed: {e}"))?;

    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path = file_path
        .into_path()
        .map_err(|e| format!("invalid export path: {e}"))?;

    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    session.export_to_path(&format, &path).map(Some)
}

#[tauri::command]
async fn pick_and_export_png(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    options: PngExportOptions,
) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("PNG Image", &["png"])
            .set_file_name("illustration.png")
            .blocking_save_file()
    })
    .await
    .map_err(|e| format!("export dialog failed: {e}"))?;

    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path = file_path
        .into_path()
        .map_err(|e| format!("invalid export path: {e}"))?;

    let mut session = state.0.lock().map_err(|e| e.to_string())?;
    session.export_png_with_options(&path, &options).map(Some)
}

/// Launches the **Molecule of the Month** Tauri v2 desktop application.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let session_path = app
                .path()
                .app_data_dir()
                .ok()
                .map(|dir| dir.join("session.json"));
            let session = StudioSession::startup(session_path.as_deref());
            app.manage(AppState(Mutex::new(session)));
            app.manage(SessionPath(session_path));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let app = window.app_handle();
                if let (Some(state), Some(path)) = (
                    app.try_state::<AppState>(),
                    app.try_state::<SessionPath>().and_then(|p| p.0.clone()),
                ) && let Ok(session) = state.0.lock()
                {
                    let _ = session.save_to(&path);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_initial_state,
            take_startup_notice,
            fetch_pdb_id,
            reset_default_2hhb,
            render_scene,
            parse_and_render_script,
            load_pdb_data,
            apply_color_preset,
            preset_thumbnails,
            export_output,
            pick_and_load_pdb,
            pick_and_load_script,
            pick_and_export,
            pick_and_export_png,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Molecule of the Month");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_session_starts_without_structure() {
        let mut session = StudioSession::new_empty();
        assert!(!session.has_structure());
        assert!(session.snapshot().is_err());
    }

    #[test]
    fn session_round_trips_through_disk() {
        let path = std::env::temp_dir().join("illustrate_session_roundtrip.json");
        let mut session = StudioSession::new_default().expect("default session");
        session.deck_state.euler_deg = [10.0, 20.0, 30.0];
        session.active_preset = "cpk_soft".to_string();
        session.save_to(&path).expect("save");

        let restored = StudioSession::startup(Some(&path));
        assert_eq!(restored.deck_state.euler_deg, [10.0, 20.0, 30.0]);
        assert_eq!(restored.active_preset, "cpk_soft");
        assert!(restored.has_structure());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn broken_session_falls_back_to_empty_with_notice() {
        let path = std::env::temp_dir().join("illustrate_session_broken.json");
        fs::write(&path, "{ not json").expect("write");
        let mut session = StudioSession::startup(Some(&path));
        assert!(!session.has_structure());
        assert!(session.take_startup_notice().is_some());
        assert!(session.take_startup_notice().is_none());
        let _ = fs::remove_file(&path);

        let mut fresh = StudioSession::startup(Some(Path::new("/nonexistent/session.json")));
        assert!(!fresh.has_structure());
        assert!(fresh.take_startup_notice().is_none());
    }

    #[test]
    fn export_png_honors_size_alpha_and_dpi() {
        let mut session = StudioSession::new_default().expect("default session");
        let dir = std::env::temp_dir();
        let path = dir.join("illustrate_export_options_test.png");

        let opts = PngExportOptions {
            width_px: Some(400),
            transparent: false,
            dpi: Some(300),
        };
        session
            .export_png_with_options(&path, &opts)
            .expect("export");
        let bytes = fs::read(&path).expect("read png");
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let reader = decoder.read_info().expect("png header");
        let info = reader.info();
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert!((396..=404).contains(&info.width), "width {}", info.width);
        let dims = info.pixel_dims.expect("pHYs chunk");
        assert_eq!(dims.xppu, 11811);
        let _ = fs::remove_file(&path);

        let too_big = PngExportOptions {
            width_px: Some(SMOOTH_MAX_CANVAS as u32 + 2),
            transparent: true,
            dpi: None,
        };
        assert!(session.export_png_with_options(&path, &too_big).is_err());
    }

    #[test]
    fn preset_thumbnails_cover_all_presets_without_mutating_session() {
        let mut session = StudioSession::new_default().expect("default session");
        let before = session.deck_state.clone();
        let thumbs = session.preset_thumbnails().expect("thumbnails");
        assert_eq!(thumbs.len(), PRESET_IDS.len());
        for (thumb, id) in thumbs.iter().zip(PRESET_IDS) {
            assert_eq!(thumb.id, *id);
            assert!(thumb.data_url.starts_with("data:image/png;base64,"));
        }
        assert_eq!(session.deck_state, before);
        assert!(StudioSession::new_empty().preset_thumbnails().is_err());
        let _ = session.snapshot();
    }

    #[test]
    fn studio_session_renders_default_2hhb_and_switches_presets() {
        let mut session = StudioSession::new_default().expect("default session");
        let snap = session.snapshot().expect("initial 2hhb render");

        assert!(snap.render.width >= 900);
        assert_eq!(snap.render.width, snap.render.height);
        assert_eq!(snap.render.display_width, snap.render.width);
        assert_eq!(snap.render.display_height, snap.render.height);
        assert_eq!(snap.render.atom_count, 4556);
        assert_eq!(snap.render.subunit_count, 8);
        assert!(
            snap.render
                .rgba_data_url
                .starts_with("data:image/png;base64,")
        );

        let mut rotated_deck = snap.deck.clone();
        rotated_deck.euler_deg = [32.0, -47.0, 115.0];
        let fast = session
            .execute_render(rotated_deck.clone(), true)
            .expect("fast preview render");
        assert!(fast.fast_preview);
        assert!(fast.width < snap.render.width);
        assert_eq!(fast.display_width, snap.render.display_width);
        assert_eq!(fast.display_height, snap.render.display_height);

        let full_rotated = session
            .execute_render(rotated_deck, false)
            .expect("full rotated render");
        assert_eq!(full_rotated.width, snap.render.width);
        assert_eq!(full_rotated.height, snap.render.height);

        let mono = session
            .apply_preset("monochrome_1992")
            .expect("apply monochrome preset");
        assert_ne!(mono.render.rgb_data_url, snap.render.rgb_data_url);
    }

    #[test]
    fn studio_session_loads_mmcif_structure() {
        let cif = "\
data_MINI
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
ATOM 1 C CA . ALA A 1 0.0 0.0 0.0 1 ALA A CA 1
ATOM 2 N N  . ALA A 1 1.4 0.0 0.0 1 ALA A N  1
ATOM 3 C CA . GLY B 1 0.0 3.0 0.0 2 GLY B CA 1
";
        let mut session = StudioSession::new_empty();
        let snap = session
            .load_pdb("mini.cif".to_string(), cif.to_string(), "subunit_pastel")
            .expect("load mmcif");
        assert!(session.has_structure());
        assert_eq!(snap.chains, vec!['A', 'B']);
        assert_eq!(snap.render.atom_count, 3);
        assert_eq!(snap.render.subunit_count, 2);
    }

    #[test]
    fn studio_session_parses_edited_script_and_exports() {
        let mut session = StudioSession::new_default().expect("default session");
        let mut snap = session.snapshot().unwrap();
        snap.deck.euler_deg = [15.0, -20.0, 45.0];
        let resp = session.execute_render(snap.deck, false).unwrap();

        let reparsed = session.load_script(&resp.script, None).unwrap();
        assert_eq!(reparsed.render.atom_count, 4556);

        let tmp = std::env::temp_dir().join(format!("motm_export_{}.png", std::process::id()));
        let msg = session.export_to_path("png", &tmp).unwrap();
        assert!(msg.contains("Saved RGBA PNG"));
        assert!(tmp.exists());
        let _ = fs::remove_file(&tmp);
    }
}
