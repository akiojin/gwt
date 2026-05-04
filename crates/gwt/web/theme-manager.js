// SPEC-2356 — Operator Design System: Theme Manager.
// Owns the `Theme Preference` and `Effective Theme` entities defined in data-model.md.
// Plain ESM module; no DOM access at top level so it stays unit-testable under Node.

const STORAGE_KEY = "gwt:ui:theme";
const VALID = new Set(["dark", "light", "auto"]);

export function createThemeManager(env) {
  const storage = env.storage;
  const matchMedia = env.matchMedia;
  const setDocumentTheme = env.setDocumentTheme;

  let preference = normalize(storage.get(STORAGE_KEY));
  const subscribers = new Set();
  const mql = matchMedia("(prefers-color-scheme: dark)");
  let lastEffective = computeEffective(preference, mql);
  applyDocument(lastEffective);

  const onSchemeChange = () => {
    if (preference !== "auto") return;
    const next = computeEffective("auto", mql);
    if (next !== lastEffective) {
      lastEffective = next;
      applyDocument(next);
      notify(next);
    }
  };
  mql.addEventListener?.("change", onSchemeChange);

  function applyDocument(eff) {
    setDocumentTheme?.(eff);
  }

  function notify(eff) {
    for (const fn of subscribers) {
      try { fn(eff); } catch (e) { console.error("theme subscriber threw", e); }
    }
  }

  return {
    getPreference() { return preference; },
    getEffective() { return lastEffective; },
    setTheme(next) {
      const normalized = normalize(next);
      preference = normalized;
      if (normalized === "auto") storage.delete(STORAGE_KEY);
      else storage.set(STORAGE_KEY, normalized);
      const eff = computeEffective(preference, mql);
      if (eff !== lastEffective) {
        lastEffective = eff;
        applyDocument(eff);
        notify(eff);
      } else {
        applyDocument(eff); // make sure document attr is correct even when no change
      }
    },
    subscribe(fn) {
      subscribers.add(fn);
      return () => subscribers.delete(fn);
    },
  };
}

function normalize(value) {
  if (typeof value !== "string") return "auto";
  const v = value.toLowerCase();
  return VALID.has(v) ? v : "auto";
}

function computeEffective(preference, mql) {
  if (preference === "auto") return mql?.matches ? "dark" : "light";
  return preference;
}

// Browser-side helper. Intentionally not invoked from this module so tests
// can stub the env. The HTML bootstrap script and `app.js` create the manager
// using `createBrowserEnv()` below.
export function createBrowserEnv(doc, win) {
  const root = doc.documentElement;
  return {
    storage: {
      get(k) { try { return win.localStorage.getItem(k); } catch { return null; } },
      set(k, v) { try { win.localStorage.setItem(k, v); } catch { /* ignore */ } },
      delete(k) { try { win.localStorage.removeItem(k); } catch { /* ignore */ } },
    },
    matchMedia: (q) => win.matchMedia(q),
    setDocumentTheme: (t) => { root.setAttribute("data-theme", t); },
  };
}
