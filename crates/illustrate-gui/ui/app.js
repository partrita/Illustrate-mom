const invoke = window.__TAURI__.core.invoke;

const state = {
  deck: null,
  chains: [],
  layer: "rgba",
  lastRender: null,
  zoom: 1,
  panX: 0,
  panY: 0,
  renderInFlight: false,
  pendingRender: null,
  previewTimer: null,
  committed: null,
  undoStack: [],
  redoStack: [],
  thumbToken: 0,
};

const HISTORY_LIMIT = 50;

const el = (id) => document.getElementById(id);

function rgbFloatToHex([r, g, b]) {
  const toHex = (v) =>
    Math.max(0, Math.min(255, Math.round(v * 255)))
      .toString(16)
      .padStart(2, "0");
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

function hexToRgbFloat(hex) {
  const clean = hex.replace("#", "");
  const r = parseInt(clean.slice(0, 2), 16) / 255;
  const g = parseInt(clean.slice(2, 4), 16) / 255;
  const b = parseInt(clean.slice(4, 6), 16) / 255;
  return [r, g, b];
}

function setStatus(msg) {
  el("status-msg").textContent = msg;
}

function applyViewportTransform() {
  const img = el("render-img");
  img.style.transform = `translate(${state.panX}px, ${state.panY}px) scale(${state.zoom})`;
}

function fitToStage() {
  if (!state.lastRender) return;
  const stage = el("stage");
  const sw = stage.clientWidth - 32;
  const sh = stage.clientHeight - 32;
  const rw = state.lastRender.display_width || state.lastRender.width;
  const rh = state.lastRender.display_height || state.lastRender.height;
  if (rw > 0 && rh > 0) {
    state.zoom = Math.min(1, sw / rw, sh / rh);
    state.panX = 0;
    state.panY = 0;
    applyViewportTransform();
  }
}

function syncEmptyState() {
  state.deck = null;
  state.chains = [];
  state.lastRender = null;
  el("empty-state").classList.remove("hidden");
  const img = el("render-img");
  img.classList.add("hidden");
  img.removeAttribute("src");
  img.style.width = "";
  img.style.height = "";
  el("hud-file").textContent = "No file loaded";
  el("hud-chains").textContent = "Chains: —";
  el("status-dims").textContent = "— × — px";
  el("status-atoms").textContent = "Atoms: 0";
  el("status-subunits").textContent = "Subunits: 0";
  el("status-z").textContent = "Z: —";
  el("status-timing").textContent = "Idle";
  el("rules-list").innerHTML = "";
  el("script-editor").value = "";
}

function updateViewportImage(renderResp) {
  state.lastRender = renderResp;
  el("empty-state").classList.add("hidden");

  const img = el("render-img");
  img.classList.remove("hidden");
  const dispW = renderResp.display_width || renderResp.width;
  const dispH = renderResp.display_height || renderResp.height;
  img.style.width = `${dispW}px`;
  img.style.height = `${dispH}px`;

  if (state.layer === "rgb") {
    img.src = renderResp.rgb_data_url;
  } else if (state.layer === "opacity") {
    img.src = renderResp.opacity_data_url;
  } else {
    img.src = renderResp.rgba_data_url;
  }

  el("status-dims").textContent = `${dispW} × ${dispH} px`;
  el("status-atoms").textContent = `Atoms: ${renderResp.atom_count.toLocaleString()}`;
  el("status-subunits").textContent = `Subunits: ${renderResp.subunit_count}`;
  el("status-z").textContent = `Z: ${renderResp.z_range[0].toFixed(1)} .. ${renderResp.z_range[1].toFixed(1)}`;
  el("status-timing").textContent = `${renderResp.fast_preview ? "Preview" : "Full"} • ${renderResp.duration_ms} ms`;

  if (document.activeElement !== el("script-editor")) {
    el("script-editor").value = renderResp.script;
  }
}

function syncControlsFromDeck() {
  const d = state.deck;
  if (!d) return;

  const setPair = (id, val) => {
    el(id).value = val;
    el(`${id}-num`).value = Number(val).toFixed(id === "pcone" ? 4 : 1);
  };

  setPair("xrot", d.euler_deg[0]);
  setPair("yrot", d.euler_deg[1]);
  setPair("zrot", d.euler_deg[2]);
  setPair("scale", d.scale);

  el("center-mode").value = d.center_mode;
  el("tran-x").value = d.translation[0];
  el("tran-y").value = d.translation[1];
  el("tran-z").value = d.translation[2];

  el("rback-color").value = rgbFloatToHex(d.world.rback);
  el("rfog-color").value = rgbFloatToHex(d.world.rfog);
  setPair("pfogh", d.world.pfogh);
  setPair("pfogl", d.world.pfogl);
  el("ixsize").value = d.world.ixsize;
  el("iysize").value = d.world.iysize;

  el("icone").checked = d.world.icone !== 0;
  setPair("pcone", d.world.pcone);
  setPair("coneangle", d.world.coneangle);
  el("rcone").value = d.world.rcone;
  el("pshadowmax").value = d.world.pshadowmax;

  el("outline-enabled").checked = d.outline.enabled;
  el("ikernel").value = String(d.outline.ikernel || 4);
  el("l-low").value = d.outline.l_low;
  el("l-high").value = d.outline.l_high;
  el("l-diff-min").value = d.outline.l_diff_min;
  el("l-diff-max").value = d.outline.l_diff_max;
  el("r-low").value = d.outline.r_low;
  el("r-high").value = d.outline.r_high;
  el("g-low").value = d.outline.g_low;
  el("g-high").value = d.outline.g_high;
  el("resdiff").value = d.outline.resdiff;

  el("hud-file").textContent = d.pdb_name || "Loaded structure";
  el("hud-chains").textContent =
    state.chains.length > 0 ? `Chains: ${state.chains.join(", ")}` : "Chains: None";

  renderRulesList();
}

function renderRulesList() {
  const list = el("rules-list");
  list.innerHTML = "";
  if (!state.deck) return;
  const rules = state.deck.descriptors;

  rules.forEach((rule, idx) => {
    const card = document.createElement("div");
    card.className = "rule-card";

    const top = document.createElement("div");
    top.className = "rule-top";

    const recSelect = document.createElement("select");
    recSelect.className = "select";
    ["ATOM  ", "HETATM"].forEach((optVal) => {
      const opt = document.createElement("option");
      opt.value = optVal;
      opt.textContent = optVal.trim();
      if (rule.record === optVal) opt.selected = true;
      recSelect.appendChild(opt);
    });
    recSelect.onchange = () => {
      rule.record = recSelect.value;
      scheduleRender(false);
    };

    const patInput = document.createElement("input");
    patInput.className = "rule-pattern";
    patInput.maxLength = 10;
    patInput.value = rule.pattern;
    patInput.title = "10-char PDB cols 13..22 ('-' is wildcard)";
    patInput.onchange = () => {
      rule.pattern = patInput.value.padEnd(10, "-").slice(0, 10);
      patInput.value = rule.pattern;
      scheduleRender(false);
    };

    const colorInput = document.createElement("input");
    colorInput.type = "color";
    colorInput.value = rgbFloatToHex(rule.color);
    colorInput.oninput = () => {
      rule.color = hexToRgbFloat(colorInput.value);
      scheduleRender(true);
    };
    colorInput.onchange = () => {
      rule.color = hexToRgbFloat(colorInput.value);
      scheduleRender(false);
    };

    top.append(recSelect, patInput, colorInput);

    const bottom = document.createElement("div");
    bottom.className = "rule-bottom";

    const lowInput = document.createElement("input");
    lowInput.type = "number";
    lowInput.className = "num-input full";
    lowInput.value = rule.res_low;
    lowInput.title = "Min residue";
    lowInput.onchange = () => {
      rule.res_low = parseInt(lowInput.value, 10) || 0;
      scheduleRender(false);
    };

    const highInput = document.createElement("input");
    highInput.type = "number";
    highInput.className = "num-input full";
    highInput.value = rule.res_high;
    highInput.title = "Max residue";
    highInput.onchange = () => {
      rule.res_high = parseInt(highInput.value, 10) || 9999;
      scheduleRender(false);
    };

    const radInput = document.createElement("input");
    radInput.type = "number";
    radInput.step = "0.1";
    radInput.min = "0";
    radInput.max = "5";
    radInput.className = "num-input full";
    radInput.value = rule.radius;
    radInput.title = "Radius (Å); 0 = omit";
    radInput.onchange = () => {
      rule.radius = parseFloat(radInput.value) || 0;
      scheduleRender(false);
    };

    const actions = document.createElement("div");
    actions.className = "rule-actions";

    const upBtn = document.createElement("button");
    upBtn.className = "btn btn-xs";
    upBtn.textContent = "↑";
    upBtn.disabled = idx === 0;
    upBtn.onclick = () => {
      [rules[idx - 1], rules[idx]] = [rules[idx], rules[idx - 1]];
      renderRulesList();
      scheduleRender(false);
    };

    const downBtn = document.createElement("button");
    downBtn.className = "btn btn-xs";
    downBtn.textContent = "↓";
    downBtn.disabled = idx === rules.length - 1;
    downBtn.onclick = () => {
      [rules[idx + 1], rules[idx]] = [rules[idx], rules[idx + 1]];
      renderRulesList();
      scheduleRender(false);
    };

    const delBtn = document.createElement("button");
    delBtn.className = "btn btn-xs";
    delBtn.textContent = "✕";
    delBtn.onclick = () => {
      rules.splice(idx, 1);
      renderRulesList();
      scheduleRender(false);
    };

    actions.append(upBtn, downBtn, delBtn);
    bottom.append(lowInput, highInput, radInput, actions);
    card.append(top, bottom);
    list.appendChild(card);
  });
}

async function runRender(fastPreview) {
  if (!state.deck) return;
  if (state.renderInFlight) {
    state.pendingRender = fastPreview;
    return;
  }
  state.renderInFlight = true;
  try {
    const resp = await invoke("render_scene", {
      deck: state.deck,
      fastPreview,
    });
    updateViewportImage(resp);
    if (!fastPreview) commitHistory();
    setStatus(fastPreview ? "Interactive preview…" : "Rendered full quality");
  } catch (err) {
    setStatus(`Error: ${err}`);
  } finally {
    state.renderInFlight = false;
    if (state.pendingRender !== null) {
      const nextMode = state.pendingRender;
      state.pendingRender = null;
      runRender(nextMode);
    }
  }
}

function scheduleRender(fastPreview) {
  if (!state.deck) return;
  if (fastPreview) {
    clearTimeout(state.previewTimer);
    runRender(true);
    state.previewTimer = setTimeout(() => runRender(false), 260);
  } else {
    clearTimeout(state.previewTimer);
    runRender(false);
  }
}

function updateHistoryButtons() {
  el("btn-undo").disabled = state.undoStack.length === 0;
  el("btn-redo").disabled = state.redoStack.length === 0;
}

function resetHistory() {
  state.undoStack = [];
  state.redoStack = [];
  state.committed = state.deck ? JSON.stringify(state.deck) : null;
  updateHistoryButtons();
}

function commitHistory() {
  if (!state.deck) return;
  const now = JSON.stringify(state.deck);
  if (state.committed !== null && now !== state.committed) {
    state.undoStack.push(state.committed);
    if (state.undoStack.length > HISTORY_LIMIT) state.undoStack.shift();
    state.redoStack = [];
  }
  state.committed = now;
  updateHistoryButtons();
}

function restoreDeck(json) {
  state.deck = JSON.parse(json);
  state.committed = json;
  syncControlsFromDeck();
  updateHistoryButtons();
  scheduleRender(false);
}

function undo() {
  if (state.undoStack.length === 0 || !state.deck) return;
  state.redoStack.push(JSON.stringify(state.deck));
  restoreDeck(state.undoStack.pop());
  setStatus("Undo");
}

function redo() {
  if (state.redoStack.length === 0 || !state.deck) return;
  state.undoStack.push(JSON.stringify(state.deck));
  restoreDeck(state.redoStack.pop());
  setStatus("Redo");
}

function setActivePreset(id) {
  el("select-preset").value = id;
  document.querySelectorAll(".preset-card").forEach((card) => {
    card.classList.toggle("active", card.dataset.preset === id);
  });
}

async function refreshPresetThumbnails() {
  const token = ++state.thumbToken;
  const cards = document.querySelectorAll(".preset-card");
  cards.forEach((c) => c.classList.add("loading"));
  try {
    const thumbs = await invoke("preset_thumbnails");
    if (token !== state.thumbToken) return;
    thumbs.forEach((t) => {
      const thumb = document.querySelector(`.preset-card[data-preset="${t.id}"] .thumb`);
      if (thumb) thumb.style.backgroundImage = `url("${t.data_url}")`;
    });
  } catch (_err) {
    // Thumbnails are decorative; keep placeholders on failure.
  } finally {
    if (token === state.thumbToken) {
      cards.forEach((c) => c.classList.remove("loading"));
    }
  }
}

async function applyPreset(id) {
  if (!state.deck) {
    setStatus("Load a PDB or mmCIF file first");
    return;
  }
  try {
    const snap = await invoke("apply_color_preset", { preset: id });
    setActivePreset(id);
    applySnapshot(snap, false);
    setStatus(`Applied preset: ${document.querySelector(`.preset-card[data-preset="${id}"] .label`).textContent}`);
  } catch (err) {
    setStatus(`Preset error: ${err}`);
  }
}

function applySnapshot(snapshot, shouldFit = false) {
  state.deck = snapshot.deck;
  state.chains = snapshot.chains;
  syncControlsFromDeck();
  updateViewportImage(snapshot.render);
  if (shouldFit) {
    resetHistory();
    refreshPresetThumbnails();
    requestAnimationFrame(fitToStage);
  } else {
    commitHistory();
  }
}

async function loadStructureFileObject(file) {
  if (!file) return;
  const content = await file.text();
  if (file.name.toLowerCase().endsWith(".inp")) {
    const snap = await invoke("parse_and_render_script", { script: content });
    applySnapshot(snap, true);
    setStatus(`Loaded script ${file.name}`);
  } else {
    const snap = await invoke("load_pdb_data", {
      filename: file.name,
      content,
      preset: el("select-preset").value,
    });
    applySnapshot(snap, true);
    setStatus(`Loaded ${file.name}`);
  }
}

async function handleOpenStructure() {
  try {
    const snap = await invoke("pick_and_load_pdb", {
      preset: el("select-preset").value,
    });
    if (snap) {
      applySnapshot(snap, true);
      setStatus(`Loaded ${snap.deck.pdb_name}`);
    }
  } catch (err) {
    setStatus(`Opening browser picker (${err})…`);
    el("file-input-structure").click();
  }
}

async function handleOpenScript() {
  try {
    const snap = await invoke("pick_and_load_script");
    if (snap) {
      applySnapshot(snap, true);
      setStatus("Loaded .inp command deck");
    }
  } catch (err) {
    setStatus(`Opening browser picker (${err})…`);
    el("file-input-inp").click();
  }
}

function bindControls() {
  const bindSliderPair = (id, getter, setter, preview = true) => {
    const range = el(id);
    const num = el(`${id}-num`);
    range.oninput = () => {
      if (!state.deck) return;
      const val = parseFloat(range.value);
      num.value = val;
      setter(val);
      scheduleRender(preview);
    };
    range.onchange = () => {
      if (!state.deck) return;
      const val = parseFloat(range.value);
      setter(val);
      scheduleRender(false);
    };
    num.onchange = () => {
      if (!state.deck) return;
      const val = parseFloat(num.value);
      if (!Number.isNaN(val)) {
        range.value = val;
        setter(val);
        scheduleRender(false);
      }
    };
  };

  bindSliderPair("xrot", () => state.deck.euler_deg[0], (v) => (state.deck.euler_deg[0] = v));
  bindSliderPair("yrot", () => state.deck.euler_deg[1], (v) => (state.deck.euler_deg[1] = v));
  bindSliderPair("zrot", () => state.deck.euler_deg[2], (v) => (state.deck.euler_deg[2] = v));
  bindSliderPair("scale", () => state.deck.scale, (v) => (state.deck.scale = v));
  bindSliderPair("pfogh", () => state.deck.world.pfogh, (v) => (state.deck.world.pfogh = v));
  bindSliderPair("pfogl", () => state.deck.world.pfogl, (v) => (state.deck.world.pfogl = v));
  bindSliderPair("pcone", () => state.deck.world.pcone, (v) => (state.deck.world.pcone = v));
  bindSliderPair("coneangle", () => state.deck.world.coneangle, (v) => (state.deck.world.coneangle = v));

  el("center-mode").onchange = () => {
    if (!state.deck) return;
    state.deck.center_mode = el("center-mode").value;
    scheduleRender(false);
  };
  el("tran-x").onchange = () => {
    if (!state.deck) return;
    state.deck.translation[0] = parseFloat(el("tran-x").value) || 0;
    scheduleRender(false);
  };
  el("tran-y").onchange = () => {
    if (!state.deck) return;
    state.deck.translation[1] = parseFloat(el("tran-y").value) || 0;
    scheduleRender(false);
  };
  el("tran-z").onchange = () => {
    if (!state.deck) return;
    state.deck.translation[2] = parseFloat(el("tran-z").value) || 0;
    scheduleRender(false);
  };

  el("rback-color").oninput = () => {
    if (!state.deck) return;
    state.deck.world.rback = hexToRgbFloat(el("rback-color").value);
    scheduleRender(true);
  };
  el("rback-color").onchange = () => {
    if (!state.deck) return;
    state.deck.world.rback = hexToRgbFloat(el("rback-color").value);
    scheduleRender(false);
  };
  el("rfog-color").oninput = () => {
    if (!state.deck) return;
    state.deck.world.rfog = hexToRgbFloat(el("rfog-color").value);
    scheduleRender(true);
  };
  el("rfog-color").onchange = () => {
    if (!state.deck) return;
    state.deck.world.rfog = hexToRgbFloat(el("rfog-color").value);
    scheduleRender(false);
  };

  el("ixsize").onchange = () => {
    if (!state.deck) return;
    state.deck.world.ixsize = parseInt(el("ixsize").value, 10) || -30;
    scheduleRender(false);
  };
  el("iysize").onchange = () => {
    if (!state.deck) return;
    state.deck.world.iysize = parseInt(el("iysize").value, 10) || -30;
    scheduleRender(false);
  };

  el("icone").onchange = () => {
    if (!state.deck) return;
    state.deck.world.icone = el("icone").checked ? 1 : 0;
    scheduleRender(false);
  };
  el("rcone").onchange = () => {
    if (!state.deck) return;
    state.deck.world.rcone = parseFloat(el("rcone").value) || 0;
    scheduleRender(false);
  };
  el("pshadowmax").onchange = () => {
    if (!state.deck) return;
    state.deck.world.pshadowmax = parseFloat(el("pshadowmax").value) || 0.2;
    scheduleRender(false);
  };

  el("outline-enabled").onchange = () => {
    if (!state.deck) return;
    state.deck.outline.enabled = el("outline-enabled").checked;
    scheduleRender(false);
  };
  el("ikernel").onchange = () => {
    if (!state.deck) return;
    state.deck.outline.ikernel = parseInt(el("ikernel").value, 10) || 4;
    scheduleRender(false);
  };
  el("l-low").onchange = () => {
    if (!state.deck) return;
    state.deck.outline.l_low = parseFloat(el("l-low").value) || 3;
    scheduleRender(false);
  };
  el("l-high").onchange = () => {
    if (!state.deck) return;
    state.deck.outline.l_high = parseFloat(el("l-high").value) || 10;
    scheduleRender(false);
  };
  el("l-diff-min").onchange = () => {
    if (!state.deck) return;
    state.deck.outline.l_diff_min = parseFloat(el("l-diff-min").value) || 0;
    scheduleRender(false);
  };
  el("l-diff-max").onchange = () => {
    if (!state.deck) return;
    state.deck.outline.l_diff_max = parseFloat(el("l-diff-max").value) || 5;
    scheduleRender(false);
  };
  el("r-low").onchange = () => {
    if (!state.deck) return;
    state.deck.outline.r_low = parseFloat(el("r-low").value) || 3;
    scheduleRender(false);
  };
  el("r-high").onchange = () => {
    if (!state.deck) return;
    state.deck.outline.r_high = parseFloat(el("r-high").value) || 10;
    scheduleRender(false);
  };
  el("g-low").onchange = () => {
    if (!state.deck) return;
    state.deck.outline.g_low = parseFloat(el("g-low").value) || 3;
    scheduleRender(false);
  };
  el("g-high").onchange = () => {
    if (!state.deck) return;
    state.deck.outline.g_high = parseFloat(el("g-high").value) || 8;
    scheduleRender(false);
  };
  el("resdiff").onchange = () => {
    if (!state.deck) return;
    state.deck.outline.resdiff = parseFloat(el("resdiff").value) || 6000;
    scheduleRender(false);
  };

  // Layer segmented buttons
  document.querySelectorAll(".seg-btn").forEach((btn) => {
    btn.onclick = () => {
      document.querySelectorAll(".seg-btn").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      state.layer = btn.dataset.layer;
      if (state.lastRender) updateViewportImage(state.lastRender);
    };
  });

  // Add atom rule
  el("btn-add-rule").onclick = () => {
    if (!state.deck) {
      setStatus("Load a PDB or mmCIF file before editing atom rules");
      return;
    }
    state.deck.descriptors.push({
      record: "ATOM  ",
      pattern: "----------",
      res_low: 0,
      res_high: 9999,
      color: [0.85, 0.85, 0.85],
      radius: 1.5,
    });
    renderRulesList();
    scheduleRender(false);
  };

  // Script actions
  el("btn-apply-script").onclick = async () => {
    try {
      const snap = await invoke("parse_and_render_script", {
        script: el("script-editor").value,
      });
      applySnapshot(snap, false);
      setStatus("Applied command deck script");
    } catch (err) {
      setStatus(`Script error: ${err}`);
    }
  };

  el("btn-copy-script").onclick = async () => {
    await navigator.clipboard.writeText(el("script-editor").value);
    setStatus("Copied .inp command deck to clipboard");
  };

  // Toolbar & empty-state file actions
  el("btn-load-pdb").onclick = handleOpenStructure;
  el("btn-empty-load-pdb").onclick = handleOpenStructure;
  el("btn-load-inp").onclick = handleOpenScript;

  el("file-input-structure").onchange = async (e) => {
    const file = e.target.files?.[0];
    e.target.value = "";
    try {
      await loadStructureFileObject(file);
    } catch (err) {
      setStatus(`Load failed: ${err}`);
    }
  };

  el("file-input-inp").onchange = async (e) => {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    try {
      const content = await file.text();
      const snap = await invoke("parse_and_render_script", { script: content });
      applySnapshot(snap, true);
      setStatus(`Loaded ${file.name}`);
    } catch (err) {
      setStatus(`Load .inp failed: ${err}`);
    }
  };

  document.querySelectorAll(".preset-card").forEach((card) => {
    card.onclick = () => applyPreset(card.dataset.preset);
  });

  el("btn-undo").onclick = undo;
  el("btn-redo").onclick = redo;

  const toggleHelp = (show) => {
    const dlg = el("help-dialog");
    const visible = show ?? dlg.classList.contains("hidden");
    dlg.classList.toggle("hidden", !visible);
  };
  el("btn-help").onclick = () => toggleHelp();
  el("btn-help-close").onclick = () => toggleHelp(false);
  el("help-dialog").onclick = (e) => {
    if (e.target === el("help-dialog")) toggleHelp(false);
  };

  const doExport = async (format) => {
    if (!state.deck) {
      setStatus("Nothing to export — load a PDB or mmCIF file first");
      return;
    }
    try {
      const msg = await invoke("pick_and_export", { format });
      if (msg) setStatus(msg);
    } catch (err) {
      setStatus(`Export failed: ${err}`);
    }
  };

  el("btn-export-png").onclick = () => doExport("png");
  el("btn-export-pnm").onclick = () => doExport("pnm");
  el("btn-export-inp").onclick = () => doExport("inp");

  window.addEventListener("keydown", (e) => {
    const tag = (e.target.tagName || "").toLowerCase();
    const typing = tag === "input" || tag === "textarea" || tag === "select";
    const mod = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    if (mod && key === "z") {
      if (typing) return;
      e.preventDefault();
      if (e.shiftKey) redo();
      else undo();
    } else if (mod && key === "y") {
      if (typing) return;
      e.preventDefault();
      redo();
    } else if (mod && key === "e") {
      e.preventDefault();
      doExport("png");
    } else if (!mod && !typing && key === "r") {
      if (!state.deck) return;
      state.deck.euler_deg = [0, 0, 0];
      syncControlsFromDeck();
      scheduleRender(false);
      setStatus("Rotation reset");
    } else if (!mod && !typing && e.key === "?") {
      toggleHelp();
    } else if (e.key === "Escape") {
      toggleHelp(false);
    }
  });

  el("btn-theme").onclick = () => {
    const root = document.documentElement;
    root.dataset.theme = root.dataset.theme === "dark" ? "light" : "dark";
  };

  el("btn-zoom-fit").onclick = fitToStage;
  el("btn-zoom-100").onclick = () => {
    state.zoom = 1;
    state.panX = 0;
    state.panY = 0;
    applyViewportTransform();
  };
  el("btn-render-full").onclick = () => scheduleRender(false);

  // Interactive viewport drag-to-rotate, pan & wheel zoom
  const stage = el("stage");
  let dragMode = null;
  let lastX = 0;
  let lastY = 0;

  stage.addEventListener("mousedown", (e) => {
    if (!state.deck) return;
    if (e.target.closest("#empty-state")) return;
    lastX = e.clientX;
    lastY = e.clientY;
    if (e.button === 1 || e.shiftKey) {
      dragMode = "pan";
    } else if (e.button === 0) {
      dragMode = "rotate";
    }
  });

  window.addEventListener("mousemove", (e) => {
    if (!dragMode || !state.deck) return;
    const dx = e.clientX - lastX;
    const dy = e.clientY - lastY;
    lastX = e.clientX;
    lastY = e.clientY;

    if (dragMode === "pan") {
      state.panX += dx;
      state.panY += dy;
      applyViewportTransform();
    } else if (dragMode === "rotate") {
      const wrap = (deg) => {
        let d = ((deg + 180) % 360 + 360) % 360 - 180;
        return Math.round(d * 10) / 10;
      };
      // +x is vertical (down), +y is horizontal (right) in Goodsell's screen coordinates
      state.deck.euler_deg[0] = wrap(state.deck.euler_deg[0] + dx * 0.45);
      state.deck.euler_deg[1] = wrap(state.deck.euler_deg[1] - dy * 0.45);
      el("xrot").value = state.deck.euler_deg[0];
      el("xrot-num").value = state.deck.euler_deg[0].toFixed(1);
      el("yrot").value = state.deck.euler_deg[1];
      el("yrot-num").value = state.deck.euler_deg[1].toFixed(1);
      clearTimeout(state.previewTimer);
      runRender(true);
    }
  });

  window.addEventListener("mouseup", () => {
    if (dragMode === "rotate") {
      scheduleRender(false);
    }
    dragMode = null;
  });

  stage.addEventListener(
    "wheel",
    (e) => {
      if (!state.deck) return;
      e.preventDefault();
      const factor = e.deltaY < 0 ? 1.1 : 0.9;
      state.zoom = Math.max(0.2, Math.min(5.0, state.zoom * factor));
      applyViewportTransform();
    },
    { passive: false }
  );

  // Drag & drop .pdb / .cif / .mmcif / .inp files
  stage.addEventListener("dragover", (e) => {
    e.preventDefault();
    el("drop-overlay").classList.remove("hidden");
  });
  stage.addEventListener("dragleave", (e) => {
    if (!stage.contains(e.relatedTarget)) {
      el("drop-overlay").classList.add("hidden");
    }
  });
  stage.addEventListener("drop", async (e) => {
    e.preventDefault();
    el("drop-overlay").classList.add("hidden");
    const file = e.dataTransfer?.files?.[0];
    try {
      await loadStructureFileObject(file);
    } catch (err) {
      setStatus(`Drop failed: ${err}`);
    }
  });
}

window.addEventListener("DOMContentLoaded", async () => {
  bindControls();
  try {
    const snap = await invoke("get_initial_state");
    if (snap) {
      applySnapshot(snap, true);
      setStatus("Ready — drag canvas to rotate, Shift+drag to pan, scroll to zoom");
    } else {
      syncEmptyState();
      setStatus("Open a .pdb, .cif, .mmcif, or .inp file to begin");
    }
  } catch (err) {
    setStatus(`Initialization error: ${err}`);
  }
});
