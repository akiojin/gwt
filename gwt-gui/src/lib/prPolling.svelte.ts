import { invoke } from "@tauri-apps/api/core";
import type { PrStatusInfo, PrStatusResponse, GhCliStatus } from "./types";

const POLL_INTERVAL_MS = 30_000;

export interface PrPollingState {
  statuses: Record<string, PrStatusInfo | null>;
  ghStatus: GhCliStatus | null;
  loading: boolean;
  error: string | null;
}

export function createPrPolling(
  projectPath: string,
  getBranchNames: () => string[],
) {
  let state = $state<PrPollingState>({
    statuses: {},
    ghStatus: null,
    loading: false,
    error: null,
  });

  let timer: ReturnType<typeof setInterval> | null = null;
  let destroyed = false;

  async function refresh() {
    if (destroyed) return;
    state.loading = true;
    state.error = null;
    try {
      const branchNames = getBranchNames();
      if (branchNames.length === 0) {
        state.statuses = {};
        state.loading = false;
        return;
      }
      const result = await invoke<PrStatusResponse>("fetch_pr_status", {
        projectPath,
        branches: branchNames,
      });
      state.statuses = result.statuses;
      state.ghStatus = result.ghStatus;
    } catch (err) {
      state.error = err instanceof Error ? err.message : String(err);
    } finally {
      state.loading = false;
    }
  }

  function start() {
    if (destroyed) return;
    stop();
    refresh();
    timer = setInterval(refresh, POLL_INTERVAL_MS);
  }

  function stop() {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  }

  function handleVisibilityChange() {
    if (document.hidden) {
      stop();
    } else {
      start();
    }
  }

  function destroy() {
    destroyed = true;
    stop();
    document.removeEventListener("visibilitychange", handleVisibilityChange);
  }

  // Auto-attach visibility listener
  document.addEventListener("visibilitychange", handleVisibilityChange);

  return {
    get state() {
      return state;
    },
    refresh,
    start,
    stop,
    destroy,
  };
}
