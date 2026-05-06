export const TITLEBAR_DOCK_HIT_HEIGHT = 38;

export function findTitlebarDockTarget(
  windows,
  point,
  sourceId,
  hitHeight = TITLEBAR_DOCK_HIT_HEIGHT,
) {
  return (
    (windows || [])
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
