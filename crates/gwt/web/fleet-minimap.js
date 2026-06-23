// SPEC-2008 camera-focus / FR-094 — Fleet Minimap (centered-radar model).
//
// A permanent, always-visible carrier docked in the canvas corner. It mirrors
// the MAIN canvas's frame of reference: the current camera viewport is FIXED at
// the minimap centre, and the world (window cells) MOVES underneath as the
// operator pans — exactly like `#canvas-stage`. The cyan camera frame sits in
// the centre; its size reflects the canvas zoom. The minimap also has its OWN
// zoom (minimapScale) so the operator can widen the radar to see the whole
// fleet or tighten it on the immediate neighbourhood.
//
// Efficiency: cells are laid out once per zoom level inside an inner WORLD layer
// at `world * minimapScale`; panning only updates ONE transform on that layer
// (mirroring the canvas stage), never per-cell styles.
//
// Passes:
// - renderCells(): rebuild the cell set + lay them out in the world layer
//   (window set / geometry / agent color / telemetry / radar-zoom change).
// - update(): translate the world layer so the camera centre lands at the
//   minimap centre + size the centred camera frame. Driven by every
//   applyViewport() (pan / zoom / framing tween / server restore).

// Absolute world→minimap-px scale bounds (radar zoom range).
const MINIMAP_SCALE_MIN = 0.004;
const MINIMAP_SCALE_MAX = 0.6;
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

  // Inner world layer: holds the cells at absolute `world * minimapScale`
  // positions. A single transform on this layer pans the radar (mirrors the
  // canvas `#canvas-stage`), so panning never touches per-cell styles.
  const worldLayer = document.createElement("div");
  worldLayer.className = "fleet-minimap__world";
  container.appendChild(worldLayer);

  // Camera-viewport frame: a centred overlay OUTSIDE the world layer, so it
  // never moves on pan. Only its size tracks the live canvas zoom.
  const cameraFrame = document.createElement("div");
  cameraFrame.className = "fleet-minimap__camera";
  cameraFrame.setAttribute("aria-hidden", "true");
  container.appendChild(cameraFrame);

  // Radar zoom controls (overlay; adjust minimapScale).
  container.appendChild(buildZoomControls());

  // FR-045 (anshin): resolve a cell's tooltip / aria-label. Prefer the
  // app-provided activity label (title · detail); fall back to the plain
  // display title when no factory was wired.
  const resolveCellTooltip =
    typeof cellTooltip === "function" ? cellTooltip : windowDisplayTitle;

  // Cells are keyed by window id so unchanged windows keep their node across
  // renders (avoids losing hover/tooltip mid-interaction).
  const cellMap = new Map();
  // Absolute world→minimap-px scale. Initialised to a fit-all value the first
  // time the minimap has windows, then kept stable (operator-controlled zoom).
  let minimapScale = null;

  function centerPx() {
    return { x: container.clientWidth / 2, y: container.clientHeight / 2 };
  }

  // World bounding box of every framable window. Null when there is nothing to
  // map. Used only to seed the initial radar scale.
  function computeWorldBounds(windows) {
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const windowData of windows) {
      const geometry = windowData.geometry;
      if (!geometry) continue;
      const x = finiteOr(Number(geometry.x), 0);
      const y = finiteOr(Number(geometry.y), 0);
      const w = Math.max(finiteOr(Number(geometry.width), 0), 0);
      const h = Math.max(finiteOr(Number(geometry.height), 0), 0);
      minX = Math.min(minX, x);
      minY = Math.min(minY, y);
      maxX = Math.max(maxX, x + w);
      maxY = Math.max(maxY, y + h);
    }
    if (!Number.isFinite(minX) || !Number.isFinite(minY)) {
      return null;
    }
    return { minX, minY, maxX, maxY };
  }

  // Scale that fits the whole fleet into ~85% of the minimap (breathing room).
  function fitAllScale(windows) {
    const bounds = computeWorldBounds(windows);
    if (!bounds) return null;
    const width = Math.max(container.clientWidth, 1);
    const height = Math.max(container.clientHeight, 1);
    const worldWidth = Math.max(bounds.maxX - bounds.minX, 1);
    const worldHeight = Math.max(bounds.maxY - bounds.minY, 1);
    return Math.min(width / worldWidth, height / worldHeight) * 0.85;
  }

  function clampScale(scale) {
    return Math.min(MINIMAP_SCALE_MAX, Math.max(MINIMAP_SCALE_MIN, scale));
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

  function renderCells() {
    const windows = (getWindows() || []).filter((windowData) => windowData?.geometry);
    container.dataset.empty = windows.length === 0 ? "true" : "false";

    if (windows.length === 0) {
      // Remove any stale cells (window set emptied).
      for (const [id, cell] of cellMap) {
        cell.remove();
        cellMap.delete(id);
      }
      update();
      return;
    }

    // Seed the radar scale to fit the whole fleet on the first populated paint;
    // afterwards it is stable and operator-controlled.
    if (minimapScale == null) {
      minimapScale = clampScale(fitAllScale(windows) ?? 0.05);
    }

    const liveIds = new Set();
    const focusedId = getFocusedId();

    for (const windowData of windows) {
      liveIds.add(windowData.id);
      const cell = ensureCell(windowData.id);
      const geometry = windowData.geometry;
      // Absolute world→radar position inside the world layer; panning only
      // translates the layer, never these.
      cell.style.left = `${finiteOr(Number(geometry.x), 0) * minimapScale}px`;
      cell.style.top = `${finiteOr(Number(geometry.y), 0) * minimapScale}px`;
      cell.style.width = `${Math.max(finiteOr(Number(geometry.width), 0) * minimapScale, 2)}px`;
      cell.style.height = `${Math.max(finiteOr(Number(geometry.height), 0) * minimapScale, 2)}px`;

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

    update();
  }

  // Centre the camera in the radar: translate the world layer so the camera's
  // world centre lands at the minimap centre, then size the centred frame.
  function update() {
    if (minimapScale == null) {
      cameraFrame.hidden = true;
      worldLayer.style.transform = "translate(0px, 0px)";
      return;
    }
    const bounds = getVisibleBounds();
    if (!bounds) {
      cameraFrame.hidden = true;
      return;
    }
    const center = centerPx();
    const camCenterX =
      finiteOr(Number(bounds.x), 0) + finiteOr(Number(bounds.width), 0) / 2;
    const camCenterY =
      finiteOr(Number(bounds.y), 0) + finiteOr(Number(bounds.height), 0) / 2;
    // Viewport fixed at the centre, world moves: shift the world layer so the
    // camera centre maps to the minimap centre.
    const translateX = center.x - camCenterX * minimapScale;
    const translateY = center.y - camCenterY * minimapScale;
    worldLayer.style.transform = `translate(${translateX}px, ${translateY}px)`;

    // Centred camera frame, sized to the live viewport at the radar scale.
    const frameWidth = Math.max(finiteOr(Number(bounds.width), 0) * minimapScale, 2);
    const frameHeight = Math.max(finiteOr(Number(bounds.height), 0) * minimapScale, 2);
    cameraFrame.hidden = false;
    cameraFrame.style.left = `${center.x - frameWidth / 2}px`;
    cameraFrame.style.top = `${center.y - frameHeight / 2}px`;
    cameraFrame.style.width = `${frameWidth}px`;
    cameraFrame.style.height = `${frameHeight}px`;
  }

  // Radar zoom: scale the world→px factor, re-lay the cells, recentre.
  function setZoom(factor) {
    if (minimapScale == null || !Number.isFinite(factor) || factor <= 0) {
      return;
    }
    minimapScale = clampScale(minimapScale * factor);
    renderCells();
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
