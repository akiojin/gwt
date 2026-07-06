// SPEC-2008 camera-focus / FR-094 (2026-07-03 再改訂) — Fleet Minimap
// (zoom-synced centered radar).
//
// A permanent, always-visible carrier docked in the canvas corner. It mirrors
// the MAIN canvas's frame of reference: the current camera viewport is FIXED at
// the minimap centre, and the world (window cells) MOVES underneath as the
// operator pans — exactly like `#canvas-stage`.
//
// The radar scale is NOT independent state: it is DERIVED from the live
// viewport on every update as
//   scale = frameFraction * min(containerW / bounds.width, containerH / bounds.height)
// Because bounds.width = canvasW / zoom, the scale is automatically
// proportional to the canvas zoom — the minimap is a true miniature of the
// main display. The centred cyan camera frame therefore keeps a CONSTANT px
// size (`frameFraction` of the minimap's limiting dimension) and is always
// visible, while the cells grow/shrink with the canvas zoom. The only operator
// state is `frameFraction` (wheel / +/− buttons): it scales the frame and the
// cells together, so the mirror relationship is preserved.
//
// Efficiency: cells are laid out inside an inner WORLD layer at
// `world * scale` and re-laid ONLY when the derived scale changes (canvas zoom
// or frameFraction change); panning only updates ONE transform on that layer
// (mirroring the canvas stage), never per-cell styles.
//
// Passes:
// - renderCells(): rebuild the cell set (window set / agent color / telemetry
//   change) and force a fresh layout.
// - update(): derive the scale, re-lay the cells when it changed, translate
//   the world layer so the camera centre lands at the minimap centre, and
//   size the centred camera frame. Driven by every applyViewport()
//   (pan / zoom / framing tween / server restore).

// The camera frame occupies this fraction of the minimap's limiting dimension.
const FRAME_FRACTION_DEFAULT = 0.45;
const FRAME_FRACTION_MIN = 0.15;
const FRAME_FRACTION_MAX = 0.9;
const MINIMAP_ZOOM_STEP = 1.25; // per wheel notch / button press.

function finiteOr(value, fallback) {
  return Number.isFinite(value) ? value : fallback;
}

export function createFleetMinimap({
  container,
  getWindows,
  getVisibleBounds,
  getFocusedId,
  frameWindow,
  windowDisplayTitle,
  // FR-045 (anshin): optional factory for the per-cell tooltip / aria-label so
  // the minimap can surface each agent's live activity (title · detail) at a
  // glance. Falls back to windowDisplayTitle for back-compat.
  cellTooltip,
  cellAgentColor,
  cellTelemetryState,
}) {
  if (!container) {
    // No container in the DOM (e.g. a stripped test page) — return a no-op
    // surface so callers never have to null-check the minimap.
    return {
      renderCells() {},
      update() {},
      updateCameraFrame() {},
      setZoom() {},
    };
  }

  // Inner world layer: holds the cells at absolute `world * scale` positions.
  // A single transform on this layer pans the radar (mirrors the canvas
  // `#canvas-stage`), so panning never touches per-cell styles.
  const worldLayer = document.createElement("div");
  worldLayer.className = "fleet-minimap__world";
  container.appendChild(worldLayer);

  // Camera-viewport frame: a centred overlay OUTSIDE the world layer, so it
  // never moves on pan. Its px size is constant across canvas zoom
  // (frameFraction of the minimap) — only radar zoom (+/−) changes it.
  const cameraFrame = document.createElement("div");
  cameraFrame.className = "fleet-minimap__camera";
  cameraFrame.setAttribute("aria-hidden", "true");
  container.appendChild(cameraFrame);

  // Radar zoom controls (overlay; adjust frameFraction).
  container.appendChild(buildZoomControls());

  // FR-045 (anshin): resolve a cell's tooltip / aria-label. Prefer the
  // app-provided activity label (title · detail); fall back to the plain
  // display title when no factory was wired.
  const resolveCellTooltip =
    typeof cellTooltip === "function" ? cellTooltip : windowDisplayTitle;

  // Cells are keyed by window id so unchanged windows keep their node across
  // renders (avoids losing hover/tooltip mid-interaction).
  const cellMap = new Map();
  // The only persistent radar state: how much of the minimap the camera frame
  // occupies. The world→px scale itself is derived from the live viewport.
  let frameFraction = FRAME_FRACTION_DEFAULT;
  // Scale used at the last cell layout; cells re-lay only when it changes.
  let layoutScale = null;
  let hasWindows = false;

  function centerPx() {
    return { x: container.clientWidth / 2, y: container.clientHeight / 2 };
  }

  function clampFraction(fraction) {
    return Math.min(FRAME_FRACTION_MAX, Math.max(FRAME_FRACTION_MIN, fraction));
  }

  // Zoom-synced world→minimap-px scale, derived from the live viewport: the
  // camera frame ends up at `frameFraction` of the limiting dimension, so it
  // always fits, and the scale is proportional to the canvas zoom.
  function deriveScale(bounds) {
    const width = Math.max(container.clientWidth, 1);
    const height = Math.max(container.clientHeight, 1);
    const boundsWidth = Math.max(finiteOr(Number(bounds.width), 0), 1);
    const boundsHeight = Math.max(finiteOr(Number(bounds.height), 0), 1);
    return frameFraction * Math.min(width / boundsWidth, height / boundsHeight);
  }

  function ensureCell(windowId) {
    let cell = cellMap.get(windowId);
    if (!cell) {
      cell = document.createElement("button");
      cell.type = "button";
      cell.className = "fleet-minimap__cell";
      cell.dataset.windowId = windowId;
      cell.addEventListener("click", (event) => {
        event.stopPropagation();
        frameWindow(windowId);
      });
      cellMap.set(windowId, cell);
      worldLayer.appendChild(cell);
    }
    return cell;
  }

  // Absolute world→radar positions inside the world layer; panning only
  // translates the layer, never these.
  function positionCells(scale) {
    const windows = (getWindows() || []).filter((windowData) => windowData?.geometry);
    for (const windowData of windows) {
      const cell = cellMap.get(windowData.id);
      if (!cell) continue;
      const geometry = windowData.geometry;
      cell.style.left = `${finiteOr(Number(geometry.x), 0) * scale}px`;
      cell.style.top = `${finiteOr(Number(geometry.y), 0) * scale}px`;
      cell.style.width = `${Math.max(finiteOr(Number(geometry.width), 0) * scale, 2)}px`;
      cell.style.height = `${Math.max(finiteOr(Number(geometry.height), 0) * scale, 2)}px`;
    }
  }

  function renderCells() {
    const windows = (getWindows() || []).filter((windowData) => windowData?.geometry);
    hasWindows = windows.length > 0;
    container.dataset.empty = hasWindows ? "false" : "true";

    if (!hasWindows) {
      // Remove any stale cells (window set emptied).
      for (const [id, cell] of cellMap) {
        cell.remove();
        cellMap.delete(id);
      }
      update();
      return;
    }

    const liveIds = new Set();
    const focusedId = getFocusedId();

    for (const windowData of windows) {
      liveIds.add(windowData.id);
      const cell = ensureCell(windowData.id);

      // Agent color via the shared data-agent-color → --current-agent map.
      const agentColor = cellAgentColor(windowData);
      if (agentColor) {
        cell.dataset.agentColor = agentColor;
      } else {
        delete cell.dataset.agentColor;
      }

      // Living Telemetry semantic state (active/idle/blocked/done) drives the
      // dot color + pulse via CSS; absent for non-agent surfaces.
      const telemetry = cellTelemetryState(windowData);
      if (telemetry) {
        cell.dataset.telemetry = telemetry;
      } else {
        delete cell.dataset.telemetry;
      }

      cell.classList.toggle("is-focused", windowData.id === focusedId);
      const tooltip = resolveCellTooltip(windowData);
      cell.setAttribute("aria-label", tooltip);
      cell.title = tooltip;
    }

    // Drop cells for windows that left the workspace.
    for (const [id, cell] of cellMap) {
      if (!liveIds.has(id)) {
        cell.remove();
        cellMap.delete(id);
      }
    }

    // The window set (or its geometry) may have changed — force a fresh layout.
    layoutScale = null;
    update();
  }

  // Derive the zoom-synced scale, re-lay the cells when it changed, then
  // centre the camera in the radar: translate the world layer so the camera's
  // world centre lands at the minimap centre, and size the centred frame.
  function update() {
    const bounds = hasWindows ? getVisibleBounds() : null;
    if (!bounds) {
      cameraFrame.hidden = true;
      worldLayer.style.transform = "translate(0px, 0px)";
      return;
    }
    const scale = deriveScale(bounds);
    if (scale !== layoutScale) {
      positionCells(scale);
      layoutScale = scale;
    }

    const center = centerPx();
    const camCenterX =
      finiteOr(Number(bounds.x), 0) + finiteOr(Number(bounds.width), 0) / 2;
    const camCenterY =
      finiteOr(Number(bounds.y), 0) + finiteOr(Number(bounds.height), 0) / 2;
    // Viewport fixed at the centre, world moves: shift the world layer so the
    // camera centre maps to the minimap centre.
    const translateX = center.x - camCenterX * scale;
    const translateY = center.y - camCenterY * scale;
    worldLayer.style.transform = `translate(${translateX}px, ${translateY}px)`;

    // Centred camera frame. Its size is zoom-invariant by construction:
    // bounds * scale = frameFraction * container on the limiting dimension.
    const frameWidth = Math.max(finiteOr(Number(bounds.width), 0) * scale, 2);
    const frameHeight = Math.max(finiteOr(Number(bounds.height), 0) * scale, 2);
    cameraFrame.hidden = false;
    cameraFrame.style.left = `${center.x - frameWidth / 2}px`;
    cameraFrame.style.top = `${center.y - frameHeight / 2}px`;
    cameraFrame.style.width = `${frameWidth}px`;
    cameraFrame.style.height = `${frameHeight}px`;
  }

  // Radar zoom: adjust the frame fraction — the camera frame and the cells
  // scale together, so the minimap stays a truthful miniature.
  function setZoom(factor) {
    if (!Number.isFinite(factor) || factor <= 0) {
      return;
    }
    frameFraction = clampFraction(frameFraction * factor);
    update();
  }

  function buildZoomControls() {
    const wrap = document.createElement("div");
    wrap.className = "fleet-minimap__zoom";
    const make = (label, ariaLabel, factor) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "fleet-minimap__zoom-button";
      button.textContent = label;
      button.setAttribute("aria-label", ariaLabel);
      button.addEventListener("click", (event) => {
        event.stopPropagation();
        setZoom(factor);
      });
      return button;
    };
    wrap.append(
      make("+", "Zoom minimap in", MINIMAP_ZOOM_STEP),
      make("−", "Zoom minimap out", 1 / MINIMAP_ZOOM_STEP),
    );
    return wrap;
  }

  // Wheel over the minimap zooms the radar (not the page).
  container.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      setZoom(event.deltaY < 0 ? MINIMAP_ZOOM_STEP : 1 / MINIMAP_ZOOM_STEP);
    },
    { passive: false },
  );

  // `updateCameraFrame` kept as an alias for existing callers (applyViewport).
  return { renderCells, update, updateCameraFrame: update, setZoom };
}
