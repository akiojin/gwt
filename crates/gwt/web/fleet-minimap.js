// SPEC-2008 camera-focus / FR-094 — Fleet Minimap.
//
// A permanent, always-visible carrier element docked in the canvas corner.
// It keeps a one-glance view of every canvas window in WORLD position even
// while the camera is focused (framed) on a single window, and overlays the
// current camera viewport as a cyan frame. Clicking a cell flies the camera
// to that window (frameWindow).
//
// The minimap lives in an OVERLAY layer OUTSIDE `#canvas-stage`, so it is not
// affected by the stage transform (translate/scale). It performs its own
// linear mapping from the world bounding box of all windows into the minimap
// rect, so it never inherits the camera zoom.
//
// Rendering is split into two cheap passes so each can be driven from its
// natural trigger:
// - renderCells(): rebuild the window cells. Driven by workspace render
//   (window set / geometry / agent color / telemetry change).
// - updateCameraFrame(): reposition the camera-viewport frame. Driven by
//   every `applyViewport()` (pan / zoom / framing tween / server restore).
//
// Both passes share `computeWorldBounds()` + `mapRect()` so the cells and the
// camera frame always use the same coordinate transform.

const MINIMAP_PADDING = 6; // inner px margin so cells/frame never touch edges.

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
      updateCameraFrame() {},
    };
  }

  // Camera-viewport frame overlay. Built once and repositioned in place so
  // pan/zoom never thrash the DOM.
  const cameraFrame = document.createElement("div");
  cameraFrame.className = "fleet-minimap__camera";
  cameraFrame.setAttribute("aria-hidden", "true");
  container.appendChild(cameraFrame);

  // FR-045 (anshin): resolve a cell's tooltip / aria-label. Prefer the
  // app-provided activity label (title · detail); fall back to the plain
  // display title when no factory was wired.
  const resolveCellTooltip =
    typeof cellTooltip === "function" ? cellTooltip : windowDisplayTitle;

  // Cells are keyed by window id so unchanged windows keep their node across
  // renders (avoids losing hover/tooltip mid-interaction).
  const cellMap = new Map();
  // The world→minimap transform from the most recent renderCells(), reused by
  // updateCameraFrame() so the frame and the cells never disagree.
  let lastTransform = null;

  function innerSize() {
    const width = Math.max(container.clientWidth - MINIMAP_PADDING * 2, 1);
    const height = Math.max(container.clientHeight - MINIMAP_PADDING * 2, 1);
    return { width, height };
  }

  // World bounding box of every framable window. Returns null when there is
  // nothing to map (the minimap then renders empty).
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

  // Uniform scale that fits the world bbox into the minimap inner rect,
  // centered (letterboxed) so window aspect ratios are preserved.
  function buildTransform(worldBounds) {
    const { width, height } = innerSize();
    const worldWidth = Math.max(worldBounds.maxX - worldBounds.minX, 1);
    const worldHeight = Math.max(worldBounds.maxY - worldBounds.minY, 1);
    const scale = Math.min(width / worldWidth, height / worldHeight);
    const scaledWidth = worldWidth * scale;
    const scaledHeight = worldHeight * scale;
    // Center the scaled world inside the inner rect.
    const offsetX = MINIMAP_PADDING + (width - scaledWidth) / 2;
    const offsetY = MINIMAP_PADDING + (height - scaledHeight) / 2;
    return { scale, offsetX, offsetY, minX: worldBounds.minX, minY: worldBounds.minY };
  }

  function mapRect(transform, rect) {
    const x = finiteOr(Number(rect.x), 0);
    const y = finiteOr(Number(rect.y), 0);
    const w = Math.max(finiteOr(Number(rect.width), 0), 0);
    const h = Math.max(finiteOr(Number(rect.height), 0), 0);
    return {
      left: transform.offsetX + (x - transform.minX) * transform.scale,
      top: transform.offsetY + (y - transform.minY) * transform.scale,
      width: Math.max(w * transform.scale, 1),
      height: Math.max(h * transform.scale, 1),
    };
  }

  function ensureCell(windowId) {
    let cell = cellMap.get(windowId);
    if (!cell) {
      cell = document.createElement("button");
      cell.type = "button";
      cell.className = "fleet-minimap__cell";
      cell.dataset.windowId = windowId;
      cell.addEventListener("click", () => {
        frameWindow(windowId);
      });
      cellMap.set(windowId, cell);
      container.appendChild(cell);
    }
    return cell;
  }

  function renderCells() {
    const windows = (getWindows() || []).filter((windowData) => windowData?.geometry);
    const worldBounds = computeWorldBounds(windows);
    const liveIds = new Set();

    container.dataset.empty = windows.length === 0 ? "true" : "false";

    if (!worldBounds) {
      lastTransform = null;
      // Remove any stale cells (window set emptied).
      for (const [id, cell] of cellMap) {
        cell.remove();
        cellMap.delete(id);
      }
      updateCameraFrame();
      return;
    }

    const transform = buildTransform(worldBounds);
    lastTransform = transform;
    const focusedId = getFocusedId();

    for (const windowData of windows) {
      liveIds.add(windowData.id);
      const cell = ensureCell(windowData.id);
      const placement = mapRect(transform, windowData.geometry);
      cell.style.left = `${placement.left}px`;
      cell.style.top = `${placement.top}px`;
      cell.style.width = `${placement.width}px`;
      cell.style.height = `${placement.height}px`;

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

    updateCameraFrame();
  }

  function updateCameraFrame() {
    if (!lastTransform) {
      cameraFrame.hidden = true;
      return;
    }
    const bounds = getVisibleBounds();
    if (!bounds) {
      cameraFrame.hidden = true;
      return;
    }
    const placement = mapRect(lastTransform, bounds);
    cameraFrame.hidden = false;
    cameraFrame.style.left = `${placement.left}px`;
    cameraFrame.style.top = `${placement.top}px`;
    cameraFrame.style.width = `${placement.width}px`;
    cameraFrame.style.height = `${placement.height}px`;
  }

  return { renderCells, updateCameraFrame };
}
