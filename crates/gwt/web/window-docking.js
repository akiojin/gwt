export const TITLEBAR_DOCK_HIT_HEIGHT = 38;
export const DETACH_TITLEBAR_ANCHOR = Object.freeze({ x: 32, y: 19 });

function finiteNumber(value) {
  return typeof value === "number" && Number.isFinite(value);
}

function normalizedRect(rect) {
  if (!rect || !finiteNumber(rect.left) || !finiteNumber(rect.top)) {
    return null;
  }
  const right = finiteNumber(rect.right)
    ? rect.right
    : rect.left + (finiteNumber(rect.width) ? rect.width : 0);
  const bottom = finiteNumber(rect.bottom)
    ? rect.bottom
    : rect.top + (finiteNumber(rect.height) ? rect.height : 0);
  if (!finiteNumber(right) || !finiteNumber(bottom)) {
    return null;
  }
  return { left: rect.left, top: rect.top, right, bottom };
}

function pointInsideRect(point, rect) {
  if (!point || !finiteNumber(point.x) || !finiteNumber(point.y)) {
    return false;
  }
  const bounds = normalizedRect(rect);
  if (!bounds) {
    return true;
  }
  return (
    point.x >= bounds.left &&
    point.x <= bounds.right &&
    point.y >= bounds.top &&
    point.y <= bounds.bottom
  );
}

export function findTitlebarDockTarget(
  windows,
  point,
  sourceId,
  hitHeight = TITLEBAR_DOCK_HIT_HEIGHT,
) {
  if (!point || !Number.isFinite(point.x) || !Number.isFinite(point.y)) {
    return null;
  }
  const windowList = windows || [];
  const sourceWindow = windowList.find((windowData) => windowData?.id === sourceId);
  if (sourceWindow?.tab_group_id) {
    return null;
  }
  return (
    windowList
      .filter((windowData) => windowData && windowData.id !== sourceId)
      .filter((windowData) => !windowData.tab_group_id || Boolean(windowData.tab_group_active))
      .slice()
      .sort((a, b) => (b.z_index || 0) - (a.z_index || 0))
      .find((windowData) => {
        const geometry = windowData.geometry;
        if (!geometry) return false;
        const titlebarHeight = Math.min(hitHeight, Math.max(0, geometry.height));
        return (
          point.x >= geometry.x &&
          point.x <= geometry.x + geometry.width &&
          point.y >= geometry.y &&
          point.y <= geometry.y + titlebarHeight
        );
      })?.id || null
  );
}

export function clientPointFromDragEvent(event, canvasRect) {
  const point = { x: event?.clientX, y: event?.clientY };
  return pointInsideRect(point, canvasRect) ? point : null;
}

export function resolveDragReleasePoint(event, fallbackPoint, canvasRect) {
  const eventPoint = clientPointFromDragEvent(event, canvasRect);
  const fallback = pointInsideRect(fallbackPoint, canvasRect) ? fallbackPoint : null;
  if (eventPoint?.x === 0 && eventPoint?.y === 0 && fallback) {
    return fallback;
  }
  return eventPoint || fallback;
}

export function detachGeometryFromClientPoint(
  point,
  windowData,
  canvasRect,
  viewport,
  anchor = DETACH_TITLEBAR_ANCHOR,
) {
  if (!pointInsideRect(point, canvasRect)) {
    return null;
  }
  const bounds = normalizedRect(canvasRect);
  if (!bounds) {
    return null;
  }
  const zoom = finiteNumber(viewport?.zoom) && viewport.zoom > 0 ? viewport.zoom : 1;
  const viewportX = finiteNumber(viewport?.x) ? viewport.x : 0;
  const viewportY = finiteNumber(viewport?.y) ? viewport.y : 0;
  const width = windowData?.geometry?.width || 720;
  const height = windowData?.geometry?.height || 420;
  const worldX = -viewportX / zoom + (point.x - bounds.left) / zoom;
  const worldY = -viewportY / zoom + (point.y - bounds.top) / zoom;
  return {
    x: worldX - anchor.x,
    y: worldY - anchor.y,
    width,
    height,
  };
}
