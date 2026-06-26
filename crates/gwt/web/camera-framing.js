function finiteNumber(value, fallback) {
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
}

function clampRange(value, min, max) {
  const lower = finiteNumber(min, 0);
  const upper = Math.max(lower, finiteNumber(max, lower));
  return Math.min(Math.max(value, lower), upper);
}

function normalizeRect(rect) {
  const left = finiteNumber(rect?.left, finiteNumber(rect?.x, 0));
  const top = finiteNumber(rect?.top, finiteNumber(rect?.y, 0));
  const width = Math.max(finiteNumber(rect?.width, 0), 0);
  const height = Math.max(finiteNumber(rect?.height, 0), 0);
  const right = finiteNumber(rect?.right, left + width);
  const bottom = finiteNumber(rect?.bottom, top + height);
  return {
    left,
    top,
    right,
    bottom,
    width: Math.max(width || right - left, 0),
    height: Math.max(height || bottom - top, 0),
  };
}

function rectsOverlap(a, b) {
  return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}

export function computeCameraFrameArea({
  canvasRect,
  obstructionRects = [],
  margin = 0,
} = {}) {
  const canvas = normalizeRect(canvasRect);
  const canvasWidth = Math.max(canvas.width || canvas.right - canvas.left, 1);
  const canvasHeight = Math.max(canvas.height || canvas.bottom - canvas.top, 1);
  const safeMargin = Math.max(finiteNumber(margin, 0), 0);
  let left = Math.min(safeMargin, canvasWidth - 1);
  let top = Math.min(safeMargin, canvasHeight - 1);
  let right = Math.max(left + 1, canvasWidth - safeMargin);
  let bottom = Math.max(top + 1, canvasHeight - safeMargin);

  for (const rawRect of obstructionRects) {
    const rect = normalizeRect(rawRect);
    if (!rectsOverlap(rect, canvas)) {
      continue;
    }
    const localLeft = clampRange(rect.left - canvas.left, 0, canvasWidth);
    const localRight = clampRange(rect.right - canvas.left, 0, canvasWidth);
    const localTop = clampRange(rect.top - canvas.top, 0, canvasHeight);
    const localBottom = clampRange(rect.bottom - canvas.top, 0, canvasHeight);
    const overlapsVerticalFrame = localTop < bottom && localBottom > top;
    if (overlapsVerticalFrame && localLeft <= safeMargin && localRight > left) {
      left = Math.min(canvasWidth - 1, localRight + safeMargin);
    }
    if (
      overlapsVerticalFrame &&
      localRight >= canvasWidth - safeMargin &&
      localLeft < right
    ) {
      right = Math.max(left + 1, localLeft - safeMargin);
    }

    const spansCanvasWidth =
      localLeft <= safeMargin && localRight >= canvasWidth - safeMargin;
    if (spansCanvasWidth && localTop <= safeMargin && localBottom > top) {
      top = Math.min(canvasHeight - 1, localBottom + safeMargin);
    }
    if (
      spansCanvasWidth &&
      localBottom >= canvasHeight - safeMargin &&
      localTop < bottom
    ) {
      bottom = Math.max(top + 1, localTop - safeMargin);
    }
  }

  right = Math.max(left + 1, right);
  bottom = Math.max(top + 1, bottom);
  return {
    left,
    top,
    width: right - left,
    height: bottom - top,
  };
}

export function computeViewportForWorldRect(
  rect,
  {
    frameArea,
    fillRatio = 0.92,
    minZoom = 0.15,
    maxZoom = 2.4,
  } = {},
) {
  const width = Math.max(finiteNumber(rect?.width, 0), 1);
  const height = Math.max(finiteNumber(rect?.height, 0), 1);
  const frame = {
    left: finiteNumber(frameArea?.left, 0),
    top: finiteNumber(frameArea?.top, 0),
    width: Math.max(finiteNumber(frameArea?.width, 0), 1),
    height: Math.max(finiteNumber(frameArea?.height, 0), 1),
  };
  const safeFillRatio = Math.max(finiteNumber(fillRatio, 1), 0.01);
  const zoom = clampRange(
    Math.min(
      (frame.width * safeFillRatio) / width,
      (frame.height * safeFillRatio) / height,
    ),
    minZoom,
    maxZoom,
  );
  const worldCenterX = finiteNumber(rect?.x, 0) + width / 2;
  const worldCenterY = finiteNumber(rect?.y, 0) + height / 2;
  return {
    x: frame.left + frame.width / 2 - worldCenterX * zoom,
    y: frame.top + frame.height / 2 - worldCenterY * zoom,
    zoom,
  };
}

export function expandWorldRectForLayoutSize(rect, layoutSize = {}) {
  const width = Math.max(
    finiteNumber(rect?.width, 0),
    finiteNumber(layoutSize?.width, 0),
    1,
  );
  const height = Math.max(
    finiteNumber(rect?.height, 0),
    finiteNumber(layoutSize?.height, 0),
    1,
  );
  return {
    x: finiteNumber(rect?.x, 0),
    y: finiteNumber(rect?.y, 0),
    width,
    height,
  };
}
