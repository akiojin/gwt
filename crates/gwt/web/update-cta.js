export function createUpdateCtaController({
  document,
  send,
  confirmUpdate,
  setVersionState = () => {},
}) {
  const shellId = "update-cta-shell";
  let latestVersion = null;
  let status = "idle";

  function removeLegacyUpdateSurfaces() {
    document.querySelectorAll(".update-toast, .update-button").forEach((node) => {
      node.remove();
    });
  }

  function ensureShell() {
    let shell = document.getElementById(shellId);
    if (!shell) {
      shell = document.createElement("div");
      shell.id = shellId;
      shell.className = "update-cta-shell";
      document.body.appendChild(shell);
    }
    return shell;
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
      dismiss.textContent = "\u00d7";
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

  function render(nextStatus, text) {
    status = nextStatus;
    const shell = ensureShell();
    const cta = ensureCta(shell);
    cta.dataset.status = nextStatus;
    cta.className = nextStatus === "available" ? "update-cta" : `update-cta is-${nextStatus}`;
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
    return render("available", `Update available: v${version} - Click to update`);
  }

  function showError(message) {
    const detail = message || "Failed to start the update.";
    return render("error", `Update failed: ${detail} Click to retry.`);
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
    showAvailable(event.latest);
  }

  function dismissCta(event) {
    event?.stopPropagation();
    const shell = document.getElementById(shellId);
    if (shell) {
      shell.remove();
    }
    status = "dismissed";
  }

  function handleClick() {
    if (status === "applying" || !latestVersion) {
      return;
    }
    if (!confirmUpdate(latestVersion)) {
      showAvailable(latestVersion);
      return;
    }
    render("applying", "Applying update...");
    send({ kind: "apply_update" });
  }

  return {
    handleUpdateState,
    showAvailable,
    showError,
  };
}
