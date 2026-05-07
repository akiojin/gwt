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
      cta = document.createElement("button");
      cta.id = "update-cta";
      cta.type = "button";
      cta.className = "update-cta";
      cta.setAttribute("aria-live", "polite");
      document.body.appendChild(cta);
    }
    cta.onclick = handleClick;
    return cta;
  }

  function render(nextStatus, text) {
    status = nextStatus;
    const cta = ensureCta();
    cta.dataset.status = nextStatus;
    cta.className = nextStatus === "available" ? "update-cta" : `update-cta is-${nextStatus}`;
    cta.disabled = nextStatus === "applying";
    cta.textContent = text;
    cta.title = text;
    cta.setAttribute("aria-label", text);
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
