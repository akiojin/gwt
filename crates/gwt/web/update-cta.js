// SPEC-2041 Phase 19 — Post-click modal & restart UX (FR-052..066).
//
// The CTA hands user clicks to #update-modal. The modal owns three states:
// `downloading` (progress bar + Cancel), `ready` (Later / Restart now),
// `failed` (Stage / Reason / Log + Open log / Retry / Close). The CTA itself
// has four observable statuses: `available` | `applying` | `ready` | `error`.

export function createUpdateCtaController({
  document,
  send,
  // Phase 14's window.confirm gating is gone; the modal supersedes it.
  // The option survives only so existing callers do not break.
  confirmUpdate: _confirmUpdate,
  setVersionState = () => {},
  // SPEC #2780 — when present, the "View release notes" button in the
  // ready/failed modal calls this with the latest version, letting the
  // user inspect the changelog before committing to a restart.
  openReleaseNotes = null,
}) {
  const shellId = "update-cta-shell";
  const modalId = "update-modal";
  let latestVersion = null;
  let pendingVersion = null;
  let status = "idle";
  let lastProgress = null;

  function removeLegacyUpdateSurfaces() {
    document.querySelectorAll(".update-toast, .update-button").forEach((node) => {
      node.remove();
    });
  }

  // User verification 2026-06-12: the SPEC-2356 sidebar Update section was
  // undiscoverable, so the CTA returns to its previous fixed bottom-right
  // home — the shell mounts on <body> and the CSS floats it.
  function ctaMountParent() {
    return document.body;
  }

  function ensureShell() {
    let shell = document.getElementById(shellId);
    if (!shell) {
      shell = document.createElement("div");
      shell.id = shellId;
      shell.className = "update-cta-shell";
    }
    const parent = ctaMountParent();
    if (shell.parentElement !== parent) {
      parent.appendChild(shell);
    }
    return shell;
  }

  // SPEC-2356 — announce update availability so the operator shell can peek /
  // badge the auto-hidden sidebar; the user notices without hovering.
  function announceUpdateAvailable() {
    try {
      document.dispatchEvent(new CustomEvent("op:update-available"));
    } catch {
      /* no-op */
    }
  }

  function announceUpdateDismissed() {
    try {
      document.dispatchEvent(new CustomEvent("op:update-dismissed"));
    } catch {
      /* no-op */
    }
  }

  function ensureCta(shell) {
    let cta = document.getElementById("update-cta");
    if (cta && cta.tagName !== "BUTTON") {
      cta.remove();
      cta = null;
    }
    if (!cta) {
      cta = document.createElement("button");
      cta.id = "update-cta";
      cta.type = "button";
      cta.className = "update-cta";
      cta.dataset.updateCtaAction = "true";
      cta.setAttribute("aria-live", "polite");
      shell.appendChild(cta);
    } else if (cta.parentElement !== shell) {
      shell.appendChild(cta);
    }
    cta.onclick = handleClick;
    return cta;
  }

  function ensureDismiss(shell) {
    let dismiss = document.querySelector("[data-update-cta-dismiss]");
    if (!dismiss) {
      dismiss = document.createElement("button");
      dismiss.type = "button";
      dismiss.className = "update-cta__dismiss";
      dismiss.dataset.updateCtaDismiss = "true";
      dismiss.textContent = "×";
      dismiss.title = "Dismiss update notification";
      dismiss.setAttribute("aria-label", "Dismiss update notification");
      dismiss.onclick = dismissCta;
      shell.appendChild(dismiss);
    } else if (dismiss.parentElement !== shell) {
      shell.appendChild(dismiss);
    }
    return dismiss;
  }

  function removeDismiss() {
    const dismiss = document.querySelector("[data-update-cta-dismiss]");
    if (dismiss) {
      dismiss.remove();
    }
  }

  function renderCta(nextStatus, text) {
    status = nextStatus;
    const shell = ensureShell();
    const cta = ensureCta(shell);
    cta.dataset.status = nextStatus;
    const stateClass =
      nextStatus === "available" ? "update-cta" : `update-cta is-${nextStatus}`;
    cta.className = stateClass;
    cta.title = text;
    cta.setAttribute("aria-label", text);
    cta.disabled = nextStatus === "applying";
    cta.textContent = text;
    if (nextStatus === "applying") {
      removeDismiss();
    } else {
      ensureDismiss(shell);
    }
    return cta;
  }

  function showAvailable(version) {
    if (!version) return null;
    removeLegacyUpdateSurfaces();
    latestVersion = version;
    const cta = renderCta("available", `Update available: v${version} - Click to update`);
    announceUpdateAvailable();
    return cta;
  }

  function showReadyPending(version) {
    if (!version) return null;
    removeLegacyUpdateSurfaces();
    pendingVersion = version;
    const cta = renderCta("ready", `Update v${version} ready — Restart now`);
    announceUpdateAvailable();
    return cta;
  }

  function showError(message) {
    const detail = message || "Failed to start the update.";
    return renderCta("error", `Update failed: ${detail} Click to retry.`);
  }

  function updateFailureDetail(payload) {
    return payload?.reason || payload?.message || "Unknown reason";
  }

  function handleUpdateState(event) {
    if (!event || event.state !== "available") {
      return;
    }
    removeLegacyUpdateSurfaces();
    setVersionState(event.current, event.latest);
    if (status === "applying" && latestVersion === event.latest) {
      return;
    }
    if (status === "ready" && pendingVersion === event.latest) {
      return;
    }
    showAvailable(event.latest);
  }

  function handleUpdateProgress(payload) {
    if (!payload) return;
    lastProgress = payload;
    const modal = document.getElementById(modalId);
    if (!modal || modal.dataset.state !== "downloading") {
      return;
    }
    updateProgressDisplay(payload);
  }

  function handleUpdateReady(payload) {
    if (!payload || !payload.version) return;
    pendingVersion = payload.version;
    if (status === "applying") {
      renderModalReady(payload.version);
    }
  }

  function handleUpdateApplyError(payload) {
    if (!payload) return;
    // SPEC-2041 Phase 19 (FR-064 follow-up, CodeRabbit review on PR #2635):
    // Later can fail when commit_update_later_pending detects the persisted
    // manifest is gone. The CTA was already morphed to `ready` by then, so
    // surface the failure modal even in the `ready` state — silently dropping
    // the error would leave the user believing Restart is safe when it isn't.
    if (status === "applying" || status === "ready") {
      showError(updateFailureDetail(payload));
      renderModalFailed(payload);
    }
  }

  function handleUpdateApplyPendingPersisted(payload) {
    if (!payload || !payload.version) return;
    showReadyPending(payload.version);
  }

  function dismissCta(event) {
    event?.stopPropagation();
    closeModal();
    const shell = document.getElementById(shellId);
    if (shell) {
      shell.remove();
    }
    status = "dismissed";
    announceUpdateDismissed();
  }

  function handleClick() {
    if (status === "applying") return;
    if (status === "ready" && pendingVersion) {
      // Re-open the modal at the ready panel; no second download.
      renderCta("applying", "Applying update...");
      renderModalReady(pendingVersion);
      return;
    }
    const version = latestVersion || pendingVersion;
    if (!version) return;
    latestVersion = version;
    renderCta("applying", "Applying update...");
    renderModalDownloading(version);
    send({ kind: "apply_update_start" });
  }

  // ---- Modal builders (uses createElement/appendChild only) ----

  function ensureModal() {
    let modal = document.getElementById(modalId);
    if (!modal) {
      modal = document.createElement("div");
      modal.id = modalId;
      modal.className = "update-modal";
      modal.setAttribute("role", "dialog");
      modal.setAttribute("aria-modal", "true");
      modal.setAttribute("aria-labelledby", "update-modal-title");
      document.body.appendChild(modal);
    }
    return modal;
  }

  function clearChildren(node) {
    while (node.firstChild) {
      node.removeChild(node.firstChild);
    }
  }

  function el(tag, props = {}, children = []) {
    const node = document.createElement(tag);
    for (const [k, v] of Object.entries(props)) {
      if (v == null) continue;
      if (k === "className") {
        node.className = v;
      } else if (k === "text") {
        node.textContent = v;
      } else if (k === "data") {
        for (const [dk, dv] of Object.entries(v)) {
          node.dataset[dk] = String(dv);
        }
      } else if (k === "attrs") {
        for (const [ak, av] of Object.entries(v)) {
          node.setAttribute(ak, String(av));
        }
      } else if (k === "onClick") {
        node.addEventListener("click", v);
      } else {
        node.setAttribute(k, String(v));
      }
    }
    for (const child of children) {
      if (child == null) continue;
      node.appendChild(child);
    }
    return node;
  }

  function closeModal() {
    const modal = document.getElementById(modalId);
    if (modal) {
      modal.remove();
    }
  }

  function renderModalDownloading(version) {
    const modal = ensureModal();
    // SPEC-2041 Phase 19 (Playwright `update-modal.spec.ts` follow-up): keep
    // renderModal* idempotent. Repeated identical events (live backend
    // re-polling update state, retried `apply_update_start`, etc.) used to
    // detach the action buttons mid-click while the test was driving the
    // post-ready flow. Detect "no-op" calls by checking the state + the
    // recorded version dataset, and skip the clearChildren/appendChild
    // churn.
    if (modal.dataset.state === "downloading" && modal.dataset.version === String(version || "")) {
      return;
    }
    modal.dataset.state = "downloading";
    modal.dataset.version = String(version || "");
    clearChildren(modal);

    const fill = el("div", { className: "update-modal__progress-fill" });
    const progress = el(
      "div",
      {
        className: "update-modal__progress",
        attrs: {
          role: "progressbar",
          "aria-valuemin": "0",
          "aria-valuemax": "100",
          "aria-valuenow": "0",
        },
        data: { updateModalProgress: "true" },
      },
      [fill],
    );
    const counter = el("p", {
      className: "update-modal__bytes",
      text: "0.0 MB / 0.0 MB",
      data: { updateModalByteCounter: "true" },
    });
    const cancel = el("button", {
      type: "button",
      className: "update-modal__btn update-modal__btn--secondary",
      text: "Cancel",
      data: { updateModalCancel: "true" },
      onClick: onCancelDownload,
    });

    const panel = el(
      "div",
      { className: "update-modal__panel", data: { state: "downloading" } },
      [
        el("h2", { id: "update-modal-title", text: "Updating gwt" }),
        el("p", {
          className: "update-modal__version",
          text: `Downloading v${version || ""}`,
          data: { updateModalVersion: "true" },
        }),
        progress,
        counter,
        el("div", { className: "update-modal__actions" }, [cancel]),
      ],
    );
    modal.appendChild(panel);

    if (lastProgress) {
      updateProgressDisplay(lastProgress);
    }
  }

  function renderModalReady(version) {
    const modal = ensureModal();
    // See renderModalDownloading: idempotency guard for repeated events.
    if (modal.dataset.state === "ready" && modal.dataset.version === String(version || "")) {
      return;
    }
    modal.dataset.state = "ready";
    modal.dataset.version = String(version || "");
    clearChildren(modal);

    const later = el("button", {
      type: "button",
      className: "update-modal__btn update-modal__btn--secondary",
      text: "Later",
      data: { updateModalLater: "true" },
      onClick: onApplyLater,
    });
    const restartNow = el("button", {
      type: "button",
      className: "update-modal__btn update-modal__btn--primary",
      text: "Restart now",
      data: { updateModalRestartNow: "true" },
      onClick: onApplyRestartNow,
    });

    // SPEC #2780 — let users preview release notes before they commit.
    const releaseNotesLink = openReleaseNotes
      ? el("button", {
          type: "button",
          className: "update-modal__link",
          text: "View release notes",
          data: { updateModalReleaseNotes: "true" },
          onClick: () => openReleaseNotes(version),
        })
      : null;

    const actionChildren = [later, restartNow];
    const panelChildren = [
      el("h2", { id: "update-modal-title", text: "Update ready" }),
      el("p", {
        className: "update-modal__version",
        text: `gwt v${version} is ready to install.`,
      }),
      el("p", {
        className: "update-modal__hint",
        text: "Restart now to launch the new version.",
      }),
      releaseNotesLink,
      el("div", { className: "update-modal__actions" }, actionChildren),
    ].filter(Boolean);

    const panel = el(
      "div",
      { className: "update-modal__panel", data: { state: "ready" } },
      panelChildren,
    );
    modal.appendChild(panel);
  }

  function renderModalFailed({ stage, reason, log_path, message }) {
    const modal = ensureModal();
    modal.dataset.state = "failed";
    delete modal.dataset.version;
    clearChildren(modal);

    // Phase 19 promotes structured `reason`, but `message` is still on the
    // wire contract for legacy callers (see UpdateApplyError optional fields
    // in protocol.rs). Fall through so older or partial emitters still
    // surface a useful failure (CodeRabbit review on PR #2630).
    const displayReason = reason || message || "Unknown reason";

    const dl = el("dl", { className: "update-modal__details" }, [
      el("dt", { text: "Stage" }),
      el("dd", {
        text: stage || "Unknown stage",
        data: { updateModalStage: "true" },
      }),
      el("dt", { text: "Reason" }),
      el("dd", {
        text: displayReason,
        data: { updateModalReason: "true" },
      }),
      el("dt", { text: "Log" }),
      el("dd", {
        text: log_path || "",
        data: { updateModalLog: "true" },
      }),
    ]);

    const openLog = el("button", {
      type: "button",
      className: "update-modal__btn update-modal__btn--secondary",
      text: "Open log",
      data: { updateModalOpenLog: "true" },
      onClick: () => {
        const message = { kind: "open_update_log" };
        if (log_path) message.log_path = log_path;
        send(message);
      },
    });
    const retry = el("button", {
      type: "button",
      className: "update-modal__btn update-modal__btn--secondary",
      text: "Retry",
      data: { updateModalRetry: "true" },
      onClick: onRetryFailed,
    });
    const closeBtn = el("button", {
      type: "button",
      className: "update-modal__btn update-modal__btn--secondary",
      text: "Close",
      data: { updateModalClose: "true" },
      onClick: onCloseFailed,
    });

    const panel = el(
      "div",
      { className: "update-modal__panel", data: { state: "failed" } },
      [
        el("h2", { id: "update-modal-title", text: "⚠ Update failed" }),
        dl,
        el("div", { className: "update-modal__actions" }, [openLog, retry, closeBtn]),
      ],
    );
    modal.appendChild(panel);
  }

  function updateProgressDisplay({ downloaded, total }) {
    const modal = document.getElementById(modalId);
    if (!modal) return;
    const progress = modal.querySelector("[data-update-modal-progress]");
    const counter = modal.querySelector("[data-update-modal-byte-counter]");
    if (progress) {
      const percent = total
        ? Math.max(0, Math.min(100, Math.round((downloaded / total) * 100)))
        : 0;
      progress.setAttribute("aria-valuenow", String(percent));
      const fill = progress.querySelector(".update-modal__progress-fill");
      if (fill) {
        fill.style.width = `${percent}%`;
      }
    }
    if (counter) {
      counter.textContent = `${formatBytes(downloaded)} / ${formatBytes(total)}`;
    }
  }

  function onCancelDownload() {
    send({ kind: "cancel_update_download" });
    lastProgress = null;
    closeModal();
    if (latestVersion) {
      showAvailable(latestVersion);
    }
  }

  function onApplyLater() {
    send({ kind: "apply_update_later" });
    closeModal();
    if (pendingVersion) {
      showReadyPending(pendingVersion);
    }
  }

  function onApplyRestartNow() {
    send({ kind: "apply_update_restart_now" });
    // Modal is intentionally left in place; the parent process will exit.
  }

  function onRetryFailed() {
    lastProgress = null;
    renderCta("applying", "Applying update...");
    renderModalDownloading(latestVersion || pendingVersion || "");
    send({ kind: "apply_update_start" });
  }

  function onCloseFailed() {
    closeModal();
    if (latestVersion) {
      showAvailable(latestVersion);
    }
  }

  function formatBytes(bytes) {
    const mb = (bytes ?? 0) / (1024 * 1024);
    return `${mb.toFixed(1)} MB`;
  }

  // SPEC #2780 v2 Amendment (FR-014, Codex review on PR #2917): Release
  // Notes window drives the apply pipeline for an arbitrary version. The
  // modal must be in `downloading` state before subsequent
  // `UpdateProgress` / `UpdateReady` events arrive, otherwise the
  // existing handlers above drop them silently (because `status !==
  // "applying"`). External callers invoke this to transition the CTA into
  // the same state `handleClick()` produces for the standard latest-update
  // path, without sending `apply_update_start` themselves.
  function beginDownloadingFor(version) {
    if (!version) return;
    pendingVersion = null;
    lastProgress = null;
    latestVersion = version;
    renderCta("applying", "Applying update...");
    renderModalDownloading(version);
  }

  return {
    handleUpdateState,
    handleUpdateProgress,
    handleUpdateReady,
    handleUpdateApplyError,
    handleUpdateApplyPendingPersisted,
    showAvailable,
    showReadyPending,
    showError,
    beginDownloadingFor,
  };
}
