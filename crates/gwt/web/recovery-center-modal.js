// SPEC-1921 Phase 80 — public-safe Recovery Center modal.

import { createFocusTrap } from "./focus-trap.js";

const ITEM_STATES = new Set(["pending", "acknowledged", "conflicted"]);
const WORKTREE_FORMS = new Set(["ephemeral", "branch-backed", "unknown"]);

function defaultRequestId() {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  return `recovery-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function allowlistedItem(item) {
  if (
    !item
    || typeof item.action_handle !== "string"
    || !item.action_handle.trim()
    || !ITEM_STATES.has(item.state)
    || !WORKTREE_FORMS.has(item.worktree_form)
  ) {
    return null;
  }
  return {
    action_handle: item.action_handle,
    state: item.state,
    worktree_form: item.worktree_form,
    title: typeof item.title === "string" ? item.title : "",
    summary: typeof item.summary === "string" ? item.summary : "",
    updated_at: typeof item.updated_at === "string" ? item.updated_at : "",
  };
}

function stateMatches(filter, state) {
  if (filter === "all") return true;
  if (filter === "attention") return state === "pending" || state === "conflicted";
  return filter === state;
}

function visibleItems(items, stateFilter, worktreeFilter) {
  return items.filter((item) => (
    stateMatches(stateFilter, item.state)
      && (worktreeFilter === "all" || item.worktree_form === worktreeFilter)
  ));
}

function label(value) {
  return value
    .split("-")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join("-");
}

function setSelectValue(select, value) {
  if (!select) return;
  for (const option of select.querySelectorAll("option")) {
    if (option.value === value) option.setAttribute("selected", "");
    else option.removeAttribute("selected");
  }
}

export function createRecoveryCenterController({
  document: doc = globalThis.document,
  modalEl,
  dialogEl,
  send,
  focusBoardEntry,
  createRequestId = defaultRequestId,
}) {
  const body = dialogEl?.querySelector(".modal-body");
  const statusEl = body?.querySelector(".recovery-center-status");
  const listEl = body?.querySelector(".recovery-center-list");
  const stateFilterEl = body?.querySelector("#recovery-center-state-filter");
  const worktreeFilterEl = body?.querySelector("#recovery-center-worktree-filter");
  const countEl = dialogEl?.querySelector(".recovery-center-count");
  const closeButtons = dialogEl?.querySelectorAll("[data-recovery-center-close]") || [];
  let releaseFocusTrap = null;
  let returnFocusTo = null;
  const state = {
    open: false,
    status: "idle",
    items: [],
    requestId: null,
    generation: 0,
    actionRequestId: null,
    selectedHandle: null,
    stateFilter: "attention",
    worktreeFilter: "all",
  };

  function clearRows() {
    if (!listEl) return;
    while (listEl.firstChild) listEl.removeChild(listEl.firstChild);
  }

  function selectRow(handle, focus = false) {
    state.selectedHandle = handle;
    for (const row of listEl?.querySelectorAll(".recovery-center-row") || []) {
      const selected = row.dataset.actionHandle === handle;
      row.classList.toggle("is-selected", selected);
      row.setAttribute("aria-selected", selected ? "true" : "false");
      if (selected && focus && typeof row.focus === "function") {
        try { row.focus({ preventScroll: true }); } catch { row.focus(); }
      }
    }
  }

  function requestBoard(item) {
    if (item.state !== "acknowledged" || state.actionRequestId) return;
    const requestId = String(createRequestId?.() || "").trim();
    if (!requestId) return;
    state.actionRequestId = requestId;
    send?.({
      kind: "open_recovery_center_board_entry",
      request_id: requestId,
      generation: state.generation,
      action_handle: item.action_handle,
    });
  }

  function renderRows() {
    clearRows();
    if (!listEl) return;
    const visible = visibleItems(state.items, state.stateFilter, state.worktreeFilter);
    if (countEl) countEl.textContent = `${visible.length} shown · ${state.items.length} total`;
    if (state.status !== "ready") return;
    if (visible.length === 0) {
      const empty = doc.createElement("p");
      empty.className = "recovery-center-empty";
      empty.setAttribute("role", "status");
      empty.textContent = state.items.length === 0
        ? "No recovery deliveries are available."
        : "No recovery deliveries match these filters.";
      listEl.appendChild(empty);
      return;
    }

    for (const item of visible) {
      const row = doc.createElement("div");
      row.className = "recovery-center-row";
      row.tabIndex = 0;
      row.dataset.actionHandle = item.action_handle;
      row.dataset.state = item.state;
      row.dataset.worktreeForm = item.worktree_form;
      row.setAttribute("role", "option");
      row.setAttribute("aria-selected", "false");

      const heading = doc.createElement("div");
      heading.className = "recovery-center-row__heading";
      const title = doc.createElement("strong");
      title.className = "recovery-center-row__title";
      title.textContent = item.title || "Recovery delivery";
      heading.appendChild(title);
      for (const [className, text] of [
        [`is-${item.state}`, label(item.state)],
        ["is-worktree", label(item.worktree_form)],
      ]) {
        const badge = doc.createElement("span");
        badge.className = `recovery-center-row__badge ${className}`;
        badge.textContent = text;
        heading.appendChild(badge);
      }
      row.appendChild(heading);

      const summary = doc.createElement("p");
      summary.className = "recovery-center-row__summary";
      summary.textContent = item.summary || "Recovery delivery";
      row.appendChild(summary);

      const meta = doc.createElement("div");
      meta.className = "recovery-center-row__meta";
      meta.textContent = item.updated_at;
      row.appendChild(meta);

      if (item.state === "acknowledged") {
        const action = doc.createElement("button");
        action.type = "button";
        action.className = "wizard-button recovery-center-row__board-action";
        action.textContent = "Open Board history";
        action.addEventListener("click", (event) => {
          event.preventDefault();
          event.stopPropagation();
          selectRow(item.action_handle);
          requestBoard(item);
        });
        row.appendChild(action);
      }

      row.addEventListener("click", () => selectRow(item.action_handle));
      row.addEventListener("keydown", (event) => {
        if (event.key === "Enter") {
          selectRow(item.action_handle);
          requestBoard(item);
          event.preventDefault();
          return;
        }
        if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
        const rows = Array.from(listEl.querySelectorAll(".recovery-center-row"));
        const index = rows.indexOf(row);
        const delta = event.key === "ArrowDown" ? 1 : -1;
        const next = rows[Math.max(0, Math.min(rows.length - 1, index + delta))];
        if (next) selectRow(next.dataset.actionHandle, true);
        event.preventDefault();
      });
      listEl.appendChild(row);
    }

    if (!visible.some((item) => item.action_handle === state.selectedHandle)) {
      state.selectedHandle = null;
    } else {
      selectRow(state.selectedHandle);
    }
  }

  function render() {
    if (!modalEl || !dialogEl || !body) return;
    modalEl.classList.toggle("open", state.open);
    modalEl.setAttribute("aria-hidden", state.open ? "false" : "true");
    setSelectValue(stateFilterEl, state.stateFilter);
    setSelectValue(worktreeFilterEl, state.worktreeFilter);
    if (statusEl) {
      statusEl.hidden = state.status === "ready";
      statusEl.className = `recovery-center-status is-${state.status}`;
      statusEl.setAttribute("role", state.status === "error" ? "alert" : "status");
      statusEl.textContent = state.status === "loading"
        ? "Loading recovery deliveries…"
        : state.status === "error"
          ? "Recovery deliveries could not be loaded."
          : "";
    }
    renderRows();
  }

  function startLoad({ resetFilters = false } = {}) {
    if (resetFilters) {
      state.stateFilter = "attention";
      state.worktreeFilter = "all";
    }
    state.status = "loading";
    state.items = [];
    state.generation = 0;
    state.actionRequestId = null;
    state.selectedHandle = null;
    const requestId = String(createRequestId?.() || "").trim();
    state.requestId = requestId || null;
    if (!state.requestId) {
      state.status = "error";
      render();
      return;
    }
    render();
    send?.({ kind: "load_recovery_center", request_id: state.requestId });
  }

  function open() {
    if (!state.open) {
      returnFocusTo = doc?.activeElement || null;
      releaseFocusTrap = createFocusTrap(dialogEl, { document: doc });
    }
    state.open = true;
    startLoad({ resetFilters: true });
    try { dialogEl.focus({ preventScroll: true }); } catch { dialogEl.focus?.(); }
  }

  function close() {
    const wasOpen = state.open;
    state.open = false;
    state.status = "idle";
    state.items = [];
    state.requestId = null;
    state.generation = 0;
    state.actionRequestId = null;
    state.selectedHandle = null;
    render();
    if (releaseFocusTrap) releaseFocusTrap();
    releaseFocusTrap = null;
    if (wasOpen && returnFocusTo?.focus) {
      try { returnFocusTo.focus({ preventScroll: true }); } catch { returnFocusTo.focus(); }
    }
    returnFocusTo = null;
  }

  function handleState(event) {
    if (!state.open || !state.requestId || event?.request_id !== state.requestId) return;
    if (event.status !== "ready") {
      state.status = "error";
      state.items = [];
      state.generation = 0;
      render();
      return;
    }
    state.status = "ready";
    state.generation = Number.isSafeInteger(event.generation) ? event.generation : 0;
    state.items = Array.isArray(event.items)
      ? event.items.map(allowlistedItem).filter(Boolean)
      : [];
    render();
  }

  function handleBoardEntry(event) {
    if (
      !state.open
      || !state.actionRequestId
      || event?.request_id !== state.actionRequestId
      || event?.generation !== state.generation
    ) return;
    state.actionRequestId = null;
    if (typeof event.board_entry_id !== "string" || !event.board_entry_id.trim()) return;
    const entryId = event.board_entry_id;
    close();
    focusBoardEntry?.(entryId);
  }

  function reconnect() {
    state.items = [];
    state.requestId = null;
    state.generation = 0;
    state.actionRequestId = null;
    state.selectedHandle = null;
    if (state.open) startLoad();
    else render();
  }

  stateFilterEl?.addEventListener("change", () => {
    state.stateFilter = stateFilterEl.value;
    renderRows();
  });
  worktreeFilterEl?.addEventListener("change", () => {
    state.worktreeFilter = worktreeFilterEl.value;
    renderRows();
  });
  for (const button of closeButtons) button.addEventListener("click", close);
  modalEl?.addEventListener("click", (event) => {
    if (event.target === modalEl) close();
  });
  doc?.addEventListener("keydown", (event) => {
    if (state.open && event.key === "Escape") {
      close();
      event.preventDefault();
    }
  });
  render();

  return { open, close, reconnect, handleState, handleBoardEntry, render };
}
