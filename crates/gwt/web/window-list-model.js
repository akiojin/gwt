// SPEC-3038 (2026-06-20) — Command Rail Windows popover model.
//
// The Windows rail badge counts every open window across all project tabs
// (`allProjectWindowIds()` in app.js). This helper groups that same window
// set by owning project tab so the popover matches the badge and supports
// cross-tab focus: each window keeps its combined id (`<tabId>::<rawId>`),
// so selecting a non-active-tab entry switches tabs through the backend
// `focus_window` path.
//
// Active-tab windows are grouped first; remaining tabs keep their appState
// order. `multiProject` is true when more than one tab contributes windows,
// which the renderer uses to decide whether to show per-project group
// headers (single-project shells stay flat).

/**
 * @param {{ tabs?: Array, active_tab_id?: string|null }} appState
 * @param {Array|undefined} snapshotEntries Optional backend `window_list`
 *   snapshot. When non-empty it gates rows to ids still present in the
 *   snapshot (drops windows closed since the snapshot was taken). When
 *   absent/empty every appState window is shown.
 * @returns {{ groups: Array<{tabId: string, tabTitle: string, isActiveTab: boolean, entries: Array}>, count: number, multiProject: boolean }}
 */
export function groupProjectWindowList(appState, snapshotEntries) {
  const tabs = Array.isArray(appState?.tabs) ? appState.tabs : [];
  const activeTabId = appState?.active_tab_id ?? null;

  const gate =
    Array.isArray(snapshotEntries) && snapshotEntries.length > 0
      ? new Set(snapshotEntries.map((entry) => entry?.id))
      : null;

  // Active tab first; Array.prototype.sort is stable, so the remaining tabs
  // keep their appState order.
  const orderedTabs = [...tabs].sort((a, b) => {
    const aRank = a?.id === activeTabId ? 0 : 1;
    const bRank = b?.id === activeTabId ? 0 : 1;
    return aRank - bRank;
  });

  const groups = [];
  let count = 0;
  for (const tab of orderedTabs) {
    const windows = Array.isArray(tab?.workspace?.windows)
      ? tab.workspace.windows
      : [];
    const entries = [];
    for (const windowData of windows) {
      if (!windowData?.id) continue;
      if (gate && !gate.has(windowData.id)) continue;
      entries.push(windowData);
    }
    if (entries.length === 0) continue;
    groups.push({
      tabId: tab?.id ?? "",
      tabTitle: tab?.title ?? "",
      isActiveTab: tab?.id === activeTabId,
      entries,
    });
    count += entries.length;
  }

  return { groups, count, multiProject: groups.length > 1 };
}
