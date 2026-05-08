export function createUpdateCtaController({
  document,
  send,
  confirmUpdate,
  setVersionState = () => {},
}) {
  let latestVersion = null;
  let status = "idle";

  function ensureCta() {
    let cta = document.getElementById("update-cta");
    if (!cta) {
      cta = document.createElement("div");
      cta.id = "update-cta";
      cta.className = "update-cta";
      cta.setAttribute("role", "group");
      cta.setAttribute("aria-live", "polite");
      document.body.appendChild(cta);
    }
    return cta;
  }

  function ensureAction(cta) {
    let action = cta.querySelector("[data-update-cta-action]");
    if (!action) {
      action = document.createElement("button");
      action.type = "button";
      action.className = "update-cta__action";
      action.dataset.updateCtaAction = "true";
      cta.appendChild(action);
    }
    action.onclick = handleClick;
    return action;
  }

  function ensureDismiss(cta) {
    let dismiss = cta.querySelector("[data-update-cta-dismiss]");
    if (!dismiss) {
      dismiss = document.createElement("button");
      dismiss.type = "button";
      dismiss.className = "update-cta__dismiss";
      dismiss.dataset.updateCtaDismiss = "true";
      dismiss.textContent = "\u00d7";
      dismiss.title = "Dismiss update notification";
      dismiss.setAttribute("aria-label", "Dismiss update notification");
      dismiss.onclick = dismissCta;
      cta.appendChild(dismiss);
    }
    return dismiss;
  }

  function removeDismiss(cta) {
    const dismiss = cta.querySelector("[data-update-cta-dismiss]");
    if (dismiss) {
      dismiss.remove();
    }
  }

  function render(nextStatus, text) {
    status = nextStatus;
    const cta = ensureCta();
    cta.dataset.status = nextStatus;
    cta.className = nextStatus === "available" ? "update-cta" : `update-cta is-${nextStatus}`;
    cta.title = text;
    cta.setAttribute("aria-label", text);
    const action = ensureAction(cta);
    action.disabled = nextStatus === "applying";
    action.textContent = text;
    action.title = text;
    action.setAttribute("aria-label", text);
    if (nextStatus === "applying") {
      removeDismiss(cta);
    } else {
      ensureDismiss(cta);
    }
    return cta;
  }

  function showAvailable(version) {
    if (!version) return null;
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
    setVersionState(event.current, event.latest);
    if (status === "applying" && latestVersion === event.latest) {
      return;
    }
    showAvailable(event.latest);
  }

  function dismissCta(event) {
    event?.stopPropagation();
    const cta = document.getElementById("update-cta");
    if (cta) {
      cta.remove();
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
