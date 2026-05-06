// SPEC-2356 FR-024 — Segmented theme toggle.
// Wires a 3-button radiogroup (AUTO / DARK / LIGHT) to the theme manager so
// every preference is reachable in a single interaction. Replaces the prior
// cycle button whose AUTO state was effectively invisible.

const ORDER = ["auto", "dark", "light"];

export function wireThemeToggle({ doc, themeManager }) {
  const root = doc.getElementById("op-theme-toggle");
  if (!root) return;

  const buttons = ORDER
    .map((value) => root.querySelector(`[data-theme-value="${value}"]`))
    .filter((el) => el);
  if (buttons.length !== ORDER.length) return;

  const indicator = doc.getElementById("op-theme-effective-indicator");

  const render = () => {
    const pref = themeManager.getPreference();
    const eff = themeManager.getEffective();
    for (const btn of buttons) {
      const value = btn.dataset.themeValue;
      const active = value === pref;
      btn.setAttribute("aria-checked", active ? "true" : "false");
      btn.setAttribute("tabindex", active ? "0" : "-1");
    }
    if (indicator) indicator.textContent = eff === "dark" ? "▮" : "▯";
    root.setAttribute(
      "aria-label",
      `Theme: ${pref === "auto" ? `auto (currently ${eff})` : pref}`,
    );
  };

  render();
  themeManager.subscribe(render);

  root.addEventListener("click", (event) => {
    const target = event.target;
    const btn = typeof target?.closest === "function"
      ? target.closest("[data-theme-value]")
      : null;
    if (!btn || !root.contains(btn)) return;
    themeManager.setTheme(btn.dataset.themeValue);
  });

  root.addEventListener("keydown", (event) => {
    const key = event.key;
    if (key !== "ArrowLeft" && key !== "ArrowRight" && key !== "Enter" && key !== " ") return;

    const focused = doc.activeElement;
    const idx = buttons.indexOf(focused);
    if (idx < 0) return;

    if (key === "ArrowLeft" || key === "ArrowRight") {
      event.preventDefault();
      const dir = key === "ArrowLeft" ? -1 : 1;
      const next = buttons[(idx + dir + buttons.length) % buttons.length];
      next.focus();
      return;
    }

    // Enter / Space commits the focused option's preference.
    event.preventDefault();
    themeManager.setTheme(focused.dataset.themeValue);
  });
}
