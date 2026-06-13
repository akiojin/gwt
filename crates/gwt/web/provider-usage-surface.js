// SPEC-3064 Phase 3 (E1) — provider usage & rate limits surface (SPEC-2970)
// extracted from app.js. Owns the latest provider usage snapshot, the
// usage formatter/render helpers, the consolidated status-strip hover
// popover (window.__gwtShowUsageHover / window.__gwtHideUsageHover), and
// the Settings "Usage & Limits" panel. Pure movement from app.js: the
// behavior, DOM output, and WS protocol are unchanged; the moved code keeps
// its original app.js indentation.
//
// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - renderWorkspaceWindows(): re-render the Workspace Overview (Kanban)
//   Work surface after a usage snapshot lands. Late-bound: app.js
//   constructs that surface after this factory runs, so the dep closes
//   over the binding instead of receiving the surface object.
export function createProviderUsageSurface({ send, renderWorkspaceWindows }) {
      // ---- Provider usage & rate limits (SPEC-2970) ----
      let latestProviderUsage = { accounts: [], sessions: [], consumption: [] };

      const USAGE_PROVIDER_NAME = { codex: "Codex", claude_code: "Claude Code" };
      const USAGE_WINDOW_LABEL = {
        five_hour: "5-hour",
        weekly: "Weekly",
        opus_weekly: "Opus weekly",
        sonnet_weekly: "Sonnet weekly",
        code_review_weekly: "Code review weekly",
      };

      function usageStateReason(state) {
        if (!state) return "";
        switch (state.kind) {
          case "disabled":
            return "Enable in Settings";
          case "no_data":
            return "No data yet";
          case "unavailable":
            return state.reason ? `Unavailable — ${state.reason}` : "Unavailable";
          case "stale":
            return `stale ${Math.round((state.age_secs || 0) / 60)}m`;
          default:
            return "";
        }
      }

      function usageFmtResetAt(iso) {
        if (!iso) return "";
        const d = new Date(iso);
        if (Number.isNaN(d.getTime())) return "";
        return d.toLocaleString();
      }

      function usageFmtTokens(n) {
        if (n == null) return "—";
        if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
        if (n >= 1000) return `${Math.round(n / 1000)}k`;
        return String(n);
      }

      function applyProviderUsageUi(snapshot) {
        latestProviderUsage = snapshot || { accounts: [], sessions: [], consumption: [] };
        try {
          window.__operatorShell?.applyProviderUsage?.(latestProviderUsage);
        } catch (e) {
          console.warn("usage pill update failed", e);
        }
        try {
          refreshUsageHoverIfOpen();
        } catch {
          /* no-op */
        }
        // Re-render regardless of session count: when a snapshot drops back to
        // sessions:[] (agent stopped, rollout/transcript unreadable, settings
        // change) the Work surface must clear its stale token/context instead
        // of keeping the previous poll's values. SPEC-2359 Phase W-12 Slice 3
        // (FR-351): the sidebar Active Works overview is gone, so usage now
        // refreshes through the Workspace Overview (Kanban) Work surface.
        try {
          renderWorkspaceWindows();
        } catch {
          /* no-op */
        }
      }

      function usageForSession(sessionId) {
        return (
          (latestProviderUsage.sessions || []).find(
            (s) => s.session_id === sessionId,
          ) || null
        );
      }

      function buildUsageBar(percent) {
        const wrap = document.createElement("div");
        wrap.className = "op-usage-bar";
        const fill = document.createElement("div");
        fill.className = "op-usage-bar__fill";
        const pct = Math.max(0, Math.min(100, Math.round(percent)));
        fill.style.width = `${pct}%`;
        if (pct >= 90) fill.dataset.level = "high";
        else if (pct >= 70) fill.dataset.level = "mid";
        wrap.appendChild(fill);
        return wrap;
      }

      function renderUsageAccountRow(account) {
        const row = document.createElement("div");
        row.className = "op-usage-account";
        row.dataset.provider = account.provider;
        const head = document.createElement("div");
        head.className = "op-usage-account__head";
        const name = document.createElement("span");
        name.className = "op-usage-account__name";
        name.textContent = USAGE_PROVIDER_NAME[account.provider] || account.provider;
        head.appendChild(name);
        if (account.plan) {
          const plan = document.createElement("span");
          plan.className = "op-usage-account__plan";
          plan.textContent = account.plan;
          head.appendChild(plan);
        }
        const reason = usageStateReason(account.state);
        if (reason) {
          const isDisabled = (account.state && account.state.kind) === "disabled";
          const r = document.createElement(isDisabled ? "button" : "span");
          r.className = "op-usage-account__reason";
          r.textContent = reason;
          if (isDisabled) {
            r.type = "button";
            r.classList.add("op-usage-account__reason--action");
            r.addEventListener("click", (e) => {
              e.stopPropagation();
              if (typeof window.__gwtHideUsageHover === "function") {
                window.__gwtHideUsageHover();
              }
              document.dispatchEvent(
                new CustomEvent("settings:open", { detail: { target: "usage" } }),
              );
            });
          }
          head.appendChild(r);
        }
        row.appendChild(head);
        for (const w of account.windows || []) {
          const line = document.createElement("div");
          line.className = "op-usage-window";
          const label = document.createElement("span");
          label.className = "op-usage-window__label";
          label.textContent = USAGE_WINDOW_LABEL[w.kind] || w.kind;
          const pct = document.createElement("span");
          pct.className = "op-usage-window__pct";
          pct.textContent = `${Math.round(w.used_percent)}%`;
          line.appendChild(label);
          line.appendChild(buildUsageBar(w.used_percent));
          line.appendChild(pct);
          if (w.resets_at) {
            const reset = document.createElement("span");
            reset.className = "op-usage-window__reset";
            reset.textContent = `↻ ${usageFmtResetAt(w.resets_at)}`;
            line.appendChild(reset);
          }
          row.appendChild(line);
        }
        return row;
      }

      function consumptionTotal(b) {
        if (!b) return 0;
        return (b.input || 0) + (b.output || 0) + (b.cached || 0);
      }

      function fmtConsumptionBreakdown(b) {
        if (!b) return "—";
        return `in ${usageFmtTokens(b.input || 0)} · out ${usageFmtTokens(
          b.output || 0,
        )} · cached ${usageFmtTokens(b.cached || 0)}`;
      }

      function renderConsumptionChart(days) {
        const chart = document.createElement("div");
        chart.className = "op-usage-chart";
        const totals = days.map((d) => consumptionTotal(d.breakdown));
        const max = Math.max(1, ...totals);
        days.forEach((d, i) => {
          const col = document.createElement("div");
          col.className = "op-usage-chart__col";
          if (i === days.length - 1) col.dataset.today = "true";
          const bar = document.createElement("div");
          bar.className = "op-usage-chart__bar";
          const total = totals[i];
          bar.style.height = `${Math.max(2, Math.round((total / max) * 100))}%`;
          bar.title = `${d.date}: ${usageFmtTokens(total)} tokens`;
          col.appendChild(bar);
          chart.appendChild(col);
        });
        return chart;
      }

      function usageFmtResetShort(iso) {
        if (!iso) return "";
        const d = new Date(iso);
        if (Number.isNaN(d.getTime())) return "";
        return d.toLocaleString(undefined, {
          month: "numeric",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        });
      }

      function usageConsumptionFor(provider) {
        return (
          (latestProviderUsage.consumption || []).find((c) => c.provider === provider) || null
        );
      }

      // One rate-limit window as an aligned row: label · bar · % · reset.
      function buildUsageWindowRow(w) {
        const row = document.createElement("div");
        row.className = "op-usage-win";
        const lbl = document.createElement("span");
        lbl.className = "op-usage-win__lbl";
        lbl.textContent = USAGE_WINDOW_LABEL[w.kind] || w.kind;
        const bar = buildUsageBar(w.used_percent);
        bar.classList.add("op-usage-win__bar");
        const pct = document.createElement("span");
        pct.className = "op-usage-win__pct";
        pct.textContent = `${Math.round(w.used_percent)}%`;
        const reset = document.createElement("span");
        reset.className = "op-usage-win__reset";
        reset.textContent = w.resets_at ? `↻ ${usageFmtResetShort(w.resets_at)}` : "";
        row.appendChild(lbl);
        row.appendChild(bar);
        row.appendChild(pct);
        row.appendChild(reset);
        return row;
      }

      // Consumption as an aligned 4-column grid (period × in/out/cached).
      function buildUsageConsumptionGrid(pc) {
        const grid = document.createElement("div");
        grid.className = "op-usage-cgrid";
        const t = pc.today || {};
        const w = pc.this_week || {};
        const cells = [
          ["hdr", "tokens"],
          ["colh", "in"],
          ["colh", "out"],
          ["colh", "cached"],
          ["rowh", "Today"],
          ["num", usageFmtTokens(t.input || 0)],
          ["num", usageFmtTokens(t.output || 0)],
          ["num", usageFmtTokens(t.cached || 0)],
          ["rowh", "Week"],
          ["num", usageFmtTokens(w.input || 0)],
          ["num", usageFmtTokens(w.output || 0)],
          ["num", usageFmtTokens(w.cached || 0)],
        ];
        for (const [kind, text] of cells) {
          const cell = document.createElement("span");
          cell.className = `op-usage-cgrid__${kind}`;
          cell.textContent = text;
          grid.appendChild(cell);
        }
        return grid;
      }

      // A provider card: header (icon · name · plan) + rate-limit windows (or a
      // degraded reason) + consumption grid + 7-day sparkline. Grouping all of
      // one provider's data together is the key readability win.
      function buildUsageProviderCard(account) {
        const card = document.createElement("div");
        card.className = "op-usage-card";
        card.dataset.provider = account.provider;

        const head = document.createElement("div");
        head.className = "op-usage-card__head";
        const icon = document.createElement("span");
        icon.className = "op-usage-card__icon";
        icon.textContent = account.provider === "claude_code" ? "◇" : "⬡";
        const name = document.createElement("span");
        name.className = "op-usage-card__name";
        name.textContent = USAGE_PROVIDER_NAME[account.provider] || account.provider;
        head.appendChild(icon);
        head.appendChild(name);
        if (account.plan) {
          const plan = document.createElement("span");
          plan.className = "op-usage-card__plan";
          plan.textContent = account.plan;
          head.appendChild(plan);
        }
        card.appendChild(head);

        const windows = account.windows || [];
        if (windows.length) {
          const wins = document.createElement("div");
          wins.className = "op-usage-wins";
          for (const w of windows) wins.appendChild(buildUsageWindowRow(w));
          card.appendChild(wins);
        } else {
          const reason = usageStateReason(account.state);
          if (reason) {
            const isDisabled = (account.state && account.state.kind) === "disabled";
            const r = document.createElement(isDisabled ? "button" : "div");
            r.className = "op-usage-card__reason";
            r.textContent = reason;
            if (isDisabled) {
              r.type = "button";
              r.classList.add("op-usage-card__reason--action");
              r.addEventListener("click", (e) => {
                e.stopPropagation();
                if (typeof window.__gwtHideUsageHover === "function") {
                  window.__gwtHideUsageHover();
                }
                document.dispatchEvent(
                  new CustomEvent("settings:open", { detail: { target: "usage" } }),
                );
              });
            }
            card.appendChild(r);
          }
        }

        const pc = usageConsumptionFor(account.provider);
        if (pc) {
          const cwrap = document.createElement("div");
          cwrap.className = "op-usage-card__cons";
          cwrap.appendChild(buildUsageConsumptionGrid(pc));
          if (Array.isArray(pc.days) && pc.days.length) {
            cwrap.appendChild(renderConsumptionChart(pc.days));
          }
          card.appendChild(cwrap);
        }
        return card;
      }

      // SPEC-2970 — the full usage detail as provider cards, appended to a
      // container. The hover popover is the single surface for all usage info
      // (the click-open modal was removed per UX feedback). The per-session
      // list was removed per UX feedback — it grew to hundreds of rows and
      // overwhelmed the popover; per-session token usage still drives each
      // agent's inline footer (see usageForSession).
      function buildUsageFullSections(container) {
        for (const account of latestProviderUsage.accounts || []) {
          container.appendChild(buildUsageProviderCard(account));
        }
      }

      // ---- Consolidated usage hover popover (SPEC-2970 UX) ----
      // Hovering the status-bar USAGE cell shows EVERYTHING at once (both
      // providers' windows with bars + full consumption with charts +
      // sessions). The click-open modal was removed per UX feedback — the hover
      // popover is the single surface. Move the cursor into it to scroll/read.
      let usageHoverEl = null;
      let usageHoverHideTimer = null;
      let usageHoverAnchor = null;

      function buildUsageHoverBody() {
        const wrap = document.createElement("div");
        wrap.className = "op-usage-hover__body";
        const head = document.createElement("div");
        head.className = "op-usage-hover__head";
        head.textContent = "Usage & Limits";
        wrap.appendChild(head);
        buildUsageFullSections(wrap);
        return wrap;
      }

      function positionUsageHover() {
        if (!usageHoverEl || !usageHoverAnchor) return;
        const r = usageHoverAnchor.getBoundingClientRect();
        const w = usageHoverEl.offsetWidth;
        const left = Math.max(8, Math.min(r.left, window.innerWidth - w - 8));
        usageHoverEl.style.left = `${left}px`;
        usageHoverEl.style.bottom = `${Math.max(8, window.innerHeight - r.top + 6)}px`;
      }

      function cancelUsageHoverHide() {
        if (usageHoverHideTimer) {
          clearTimeout(usageHoverHideTimer);
          usageHoverHideTimer = null;
        }
      }

      function refreshUsageHoverIfOpen() {
        if (!usageHoverEl || usageHoverEl.hidden) return;
        while (usageHoverEl.firstChild) usageHoverEl.removeChild(usageHoverEl.firstChild);
        usageHoverEl.appendChild(buildUsageHoverBody());
        positionUsageHover();
      }

      window.__gwtShowUsageHover = (anchor) => {
        cancelUsageHoverHide();
        usageHoverAnchor = anchor || usageHoverAnchor;
        if (!usageHoverEl) {
          usageHoverEl = document.createElement("div");
          usageHoverEl.className = "op-usage-hover";
          usageHoverEl.addEventListener("mouseenter", cancelUsageHoverHide);
          usageHoverEl.addEventListener("mouseleave", () => window.__gwtHideUsageHover());
          document.body.appendChild(usageHoverEl);
        }
        while (usageHoverEl.firstChild) usageHoverEl.removeChild(usageHoverEl.firstChild);
        usageHoverEl.appendChild(buildUsageHoverBody());
        usageHoverEl.hidden = false;
        usageHoverEl.style.visibility = "hidden";
        requestAnimationFrame(() => {
          positionUsageHover();
          if (usageHoverEl) usageHoverEl.style.visibility = "visible";
        });
      };

      window.__gwtHideUsageHover = () => {
        cancelUsageHoverHide();
        usageHoverHideTimer = setTimeout(() => {
          if (usageHoverEl) usageHoverEl.hidden = true;
          usageHoverHideTimer = null;
        }, 180);
      };

      // SPEC-2970 FR-009/FR-013 — Settings "Usage & Limits" panel: Claude
      // account usage is opt-in (Keychain + network); Codex is local + auto.
      function renderUsagePanel(panel) {
        while (panel.firstChild) panel.removeChild(panel.firstChild);
        const section = document.createElement("div");
        section.className = "settings-section";

        const heading = document.createElement("h3");
        heading.textContent = "Provider Usage & Limits";
        section.appendChild(heading);

        const codexNote = document.createElement("p");
        codexNote.className = "settings-hint";
        codexNote.textContent =
          "Codex usage is read from local session files automatically.";
        section.appendChild(codexNote);

        const label = document.createElement("label");
        label.className = "settings-toggle";
        const checkbox = document.createElement("input");
        checkbox.type = "checkbox";
        const claudeAccount = (latestProviderUsage.accounts || []).find(
          (a) => a.provider === "claude_code",
        );
        checkbox.checked = !!(
          claudeAccount &&
          claudeAccount.state &&
          claudeAccount.state.kind !== "disabled"
        );
        checkbox.addEventListener("change", () => {
          try {
            send({
              kind: "set_claude_account_usage_enabled",
              enabled: checkbox.checked,
            });
          } catch {
            /* no-op */
          }
        });
        const span = document.createElement("span");
        span.textContent = "Show Claude Code account usage (5-hour / weekly)";
        label.appendChild(checkbox);
        label.appendChild(span);
        section.appendChild(label);

        const consent = document.createElement("p");
        consent.className = "settings-hint";
        consent.textContent =
          "Off by default (opt-in). When enabled, Claude account usage reads your OAuth token from the Keychain / credentials file and requests usage from the Anthropic API (polled at most once every 3 minutes). While disabled, no Keychain read or network request happens. Per-session token usage is read locally and is not affected by this setting.";
        section.appendChild(consent);

        panel.appendChild(section);
      }

      return {
        applyProviderUsageUi,
        renderUsagePanel,
        usageForSession,
      };
}
