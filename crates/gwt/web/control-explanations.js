// SPEC-4777: shared hover and keyboard explanations over the existing
// Operator popover shell. Surface models retain ownership of their controls.

export function setControlExplanation(node, text, key, focusable = true) {
  node.dataset.issueExplanation = text;
  node.setAttribute("aria-description", text);
  if (key) node.dataset.issueExplanationKey = key;
  if (focusable && !node.matches("button, input, select, textarea, a[href], summary")) {
    node.setAttribute("tabindex", "0");
  }
  return node;
}

export function bindControlExplanations({ document, root, id, placement = "bottom", passThrough = false }) {
  const popup = document.createElement("div");
  popup.className = "op-runtime-health-detail issue-control-explanation";
  popup.id = id;
  popup.setAttribute("role", "tooltip");
  popup.hidden = true;
  if (passThrough) popup.style.pointerEvents = "none";
  document.body.appendChild(popup);
  let anchor = null;
  let focused = null;
  let hovered = null;
  let dragging = false;
  let pointerActive = false;
  let restoringFocus = false;
  let popupHovered = false;
  const target = event => event.target?.closest?.("[data-issue-explanation]");
  const hide = () => {
    if (anchor) {
      const ids = (anchor.getAttribute("aria-describedby") || "").split(/\s+/)
        .filter(id => id && id !== popup.id);
      if (ids.length) anchor.setAttribute("aria-describedby", ids.join(" "));
      else anchor.removeAttribute("aria-describedby");
    }
    anchor = null;
    popup.hidden = true;
    if (passThrough) popupHovered = false;
  };
  const insidePopup = event => {
    const rect = popup.getBoundingClientRect();
    return Number.isFinite(event.clientX) && Number.isFinite(event.clientY)
      && event.clientX >= rect.left && event.clientX <= rect.right
      && event.clientY >= rect.top && event.clientY <= rect.bottom;
  };
  const show = node => {
    if (!node?.isConnected || !root.contains(node) || node.closest("[hidden], .hidden")) { hide(); return; }
    hide();
    anchor = node;
    popup.textContent = node.dataset.issueExplanation;
    popup.hidden = false;
    node.setAttribute("aria-describedby", [node.getAttribute("aria-describedby"), popup.id].filter(Boolean).join(" "));
    // Keep the popup outside a draggable card so its pointer hit area
    // cannot cover the card itself (including its action buttons).
    const rect = (node.closest(".knowledge-row") || node).getBoundingClientRect?.();
    if (!rect) return;
    const viewport = document.defaultView || {};
    const width = viewport.innerWidth || 1024;
    const height = viewport.innerHeight || 768;
    if (placement === "right") {
      const boundary = root.getBoundingClientRect();
      const beside = boundary.right <= width - 8;
      const left = beside ? Math.max(8, boundary.right) : 8;
      const top = beside ? rect.top : Math.max(8, boundary.bottom);
      // Constrain before measuring: wrapping changes the popup's height.
      // A full-width top strip uses its bottom edge to clear every control.
      popup.style.maxWidth = `${Math.max(0, width - left - 8)}px`;
      popup.style.maxHeight = `${Math.max(0, beside ? height - 16 : height - top - 8)}px`;
      const size = popup.getBoundingClientRect();
      popup.style.left = `${left}px`;
      popup.style.top = `${beside ? Math.max(8, Math.min(top, height - size.height - 8)) : top}px`;
      return;
    }
    const size = popup.getBoundingClientRect?.() || { width: 0, height: 0 };
    popup.style.left = `${Math.max(8, Math.min(rect.left, width - size.width - 8))}px`;
    const top = rect.bottom + size.height <= height - 8
      ? rect.bottom - 1 : rect.top - size.height + 1;
    popup.style.top = `${Math.max(8, Math.min(top, height - size.height - 8))}px`;
  };
  const over = event => {
    if (dragging || event.buttons) return;
    // Hiding a dismissed tooltip exposes the control underneath it;
    // that hit-test change is not a new pointer entry by the user.
    if (popup.hidden && popup.contains(event.relatedTarget)) return;
    const node = target(event);
    if (!node || node.contains(event.relatedTarget)) return;
    hovered = node;
    show(node);
  };
  const out = event => {
    if (!hovered) return;
    const boundary = placement === "right" ? root : hovered.closest(".knowledge-row") || hovered;
    if (!boundary.contains(event.target) || boundary.contains(event.relatedTarget)) return;
    hovered = null;
    if (popup.contains(event.relatedTarget)) return;
    if (passThrough && insidePopup(event)) { popupHovered = true; return; }
    if (focused) show(focused); else hide();
  };
  const focus = event => { focused = pointerActive ? null : target(event); if (focused && !restoringFocus) show(focused); };
  const blur = () => { focused = null; if (restoringFocus || popupHovered) return; if (hovered) show(hovered); else hide(); };
  const escape = event => {
    if (event.key !== "Escape" || popup.hidden) return;
    focused = null;
    hovered = null;
    hide();
    event.preventDefault();
    event.stopPropagation();
  };
  const leavePopup = () => { popupHovered = false; if (focused) show(focused); else hide(); };
  const refresh = () => {
    if (!anchor) return;
    const previous = anchor;
    const key = previous.dataset.issueExplanationKey;
    const node = previous.isConnected ? previous
      : key ? root.querySelector(`[data-issue-explanation-key="${key}"]`) : null;
    if (focused === previous) focused = node;
    if (hovered === previous) hovered = node;
    show(node);
  };
  const pointerDown = () => { pointerActive = true; focused = null; hovered = null; hide(); };
  const pointerEnd = () => { pointerActive = false; };
  const pointerMove = event => {
    if (popup.hidden) return;
    popupHovered = insidePopup(event);
    if (root.contains(event.target) || popupHovered) return;
    hovered = null;
    if (focused) show(focused); else hide();
  };
  const outsideFocus = event => {
    if (root.contains(event.target)) return;
    focused = null; hovered = null; hide();
  };
  const outsidePress = event => { if (!root.contains(event.target)) pointerDown(); };
  const dragStart = () => { dragging = true; pointerDown(); };
  const dragEnd = () => { dragging = false; pointerEnd(); };
  const listeners = { mouseover: over, mouseout: out, focusin: focus, focusout: blur,
    pointerdown: pointerDown, dragstart: dragStart, dragend: dragEnd,
    "control-explanations:refresh": refresh };
  for (const [type, listener] of Object.entries(listeners)) root.addEventListener(type, listener);
  popup.addEventListener("mouseenter", () => { popupHovered = true; });
  popup.addEventListener("mouseleave", leavePopup);
  // Dismiss before the app's bubble handler turns Escape into overview.
  document.addEventListener("keydown", escape, true);
  document.addEventListener("scroll", refresh, true);
  document.addEventListener("pointerup", pointerEnd);
  document.addEventListener("pointercancel", pointerEnd);
  if (passThrough) {
    document.addEventListener("pointermove", pointerMove);
    document.addEventListener("focusin", outsideFocus, true);
    document.addEventListener("pointerdown", outsidePress, true);
  }
  document.defaultView?.addEventListener("resize", hide);
  return {
    get visible() { return !popup.hidden; },
    restoreFocus(node, visible) {
      // Restoring a view must not reopen an explanation dismissed with Escape.
      restoringFocus = true;
      try { node?.focus(); } finally { restoringFocus = false; }
      if (!visible) { focused = null; hovered = null; hide(); }
    },
    refresh,
    dispose() {
      hide();
      for (const [type, listener] of Object.entries(listeners)) root.removeEventListener(type, listener);
      document.removeEventListener("scroll", refresh, true);
      document.removeEventListener("keydown", escape, true);
      document.removeEventListener("pointerup", pointerEnd);
      document.removeEventListener("pointercancel", pointerEnd);
      if (passThrough) {
        document.removeEventListener("pointermove", pointerMove);
        document.removeEventListener("focusin", outsideFocus, true);
        document.removeEventListener("pointerdown", outsidePress, true);
      }
      document.defaultView?.removeEventListener("resize", hide);
      popup.remove();
    },
  };
}
