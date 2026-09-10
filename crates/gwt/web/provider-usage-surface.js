// SPEC-3064 Phase 3 (E1) — provider usage & rate limits surface (SPEC-2970)
// extracted from app.js. Owns the latest provider usage snapshot, the
// usage formatter/render helpers, the consolidated status-strip hover
// popover (window.__gwtShowUsageHover / window.__gwtHideUsageHover), and
// the Settings "Usage & Limits" panel. The WS protocol remains owned by
// app.js; presentation follow-ups stay within this extracted surface.
//
// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - renderWorkspaceWindows(): re-render the Workspace Overview (Kanban)
//   Work surface after a usage snapshot lands. Late-bound: app.js
//   constructs that surface after this factory runs, so the dep closes
//   over the binding instead of receiving the surface object.
// - sessionLabel(sessionId) (optional): resolve a gwt session id to the
//   window title the user knows it by (Issue #3862). Returning a falsy
//   value falls back to a shortened session id.
export function createProviderUsageSurface({
  send,
  renderWorkspaceWindows,
  sessionLabel = () => null,
}) {
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

      // Issue #3860 — a window's reported length in minutes, when it is a
      // finite positive number; otherwise null.
      function usageWindowMinutes(w) {
        const minutes = Number(w && w.window_minutes);
        return Number.isFinite(minutes) && minutes > 0 ? Math.round(minutes) : null;
      }

      // "1-day" / "5-hour" / "90-minute" style length for labels.
      function usageWindowLengthShort(minutes) {
        if (minutes % 1440 === 0) return `${minutes / 1440}-day`;
        if (minutes % 60 === 0) return `${minutes / 60}-hour`;
        return `${minutes}-minute`;
      }

      // "7 days" / "5 hours" / "90 minutes" style length for tooltips.
      function usageWindowLengthLong(minutes) {
        const unit = (n, name) => `${n} ${name}${n === 1 ? "" : "s"}`;
        if (minutes % 1440 === 0) return unit(minutes / 1440, "day");
        if (minutes % 60 === 0) return unit(minutes / 60, "hour");
        return unit(minutes, "minute");
      }

      // Row label for one window. Known kinds keep their fixed label; an
      // `unknown` kind (length missing or unrecognized upstream) is shown as
      // such, with the reported length when there is one, instead of being
      // forced into a known window or dropped.
      function usageWindowLabel(w) {
        const known = USAGE_WINDOW_LABEL[w.kind];
        if (known) return known;
        if (w.kind !== "unknown") return w.kind;
        const minutes = usageWindowMinutes(w);
        return minutes == null ? "Unknown" : `Unknown (${usageWindowLengthShort(minutes)})`;
      }

      // Apply the label plus the real window length (data attribute + tooltip)
      // to a label element so the UI can surface the length for every window.
      function applyUsageWindowLabel(el, w) {
        el.textContent = usageWindowLabel(w);
        const minutes = usageWindowMinutes(w);
        if (minutes != null) {
          el.dataset.windowMinutes = String(minutes);
          el.title = `Window length: ${usageWindowLengthLong(minutes)}`;
        }
      }

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

      function buildUsageBar(percent, limitReached = false) {
        const wrap = document.createElement("div");
        wrap.className = "op-usage-bar";
        const fill = document.createElement("div");
        fill.className = "op-usage-bar__fill";
        const boundedPercent = Math.max(0, Math.min(100, percent));
        fill.style.width = `${Math.round(boundedPercent)}%`;
        if (limitReached || boundedPercent >= 95) fill.dataset.severity = "danger";
        else if (boundedPercent >= 80) fill.dataset.severity = "warning";
        else fill.dataset.severity = "normal";
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
          applyUsageWindowLabel(label, w);
          const pct = document.createElement("span");
          pct.className = "op-usage-window__pct";
          pct.textContent = `${Math.round(w.used_percent)}%`;
          line.appendChild(label);
          line.appendChild(buildUsageBar(w.used_percent, account.limit_reached));
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
      function buildUsageWindowRow(w, limitReached = false) {
        const row = document.createElement("div");
        row.className = "op-usage-win";
        const lbl = document.createElement("span");
        lbl.className = "op-usage-win__lbl";
        applyUsageWindowLabel(lbl, w);
        const bar = buildUsageBar(w.used_percent, limitReached);
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

      // ---- Per-session tokens / context (Issue #3862) ----
      // The popover is the one usage surface, so per-session data must be
      // readable here too. The list is bounded: the earlier unbounded table
      // grew to hundreds of rows and was removed, so each provider shows at
      // most USAGE_SESSION_ROWS_MAX rows (context-hungriest first) plus a
      // "+N more" line.
      const USAGE_SESSION_ROWS_MAX = 5;

      function usageSessionsFor(provider) {
        return (latestProviderUsage.sessions || []).filter(
          (s) => s && s.provider === provider,
        );
      }

      function usageSessionContextLeft(s) {
        const pct = Number(s.context_left_pct);
        return s.context_left_pct != null && Number.isFinite(pct) ? pct : null;
      }

      // Rank: eligible sessions with a known context figure (lowest remaining
      // first — the ones closest to compaction), then eligible sessions
      // without one, then ineligible (API-key backend / other agent) sessions.
      function usageSessionRank(s) {
        if (s.eligible === false) return 2;
        return usageSessionContextLeft(s) == null ? 1 : 0;
      }

      function compareUsageSessions(a, b) {
        const ra = usageSessionRank(a);
        const rb = usageSessionRank(b);
        if (ra !== rb) return ra - rb;
        if (ra === 0) return usageSessionContextLeft(a) - usageSessionContextLeft(b);
        return 0;
      }

      function usageSessionName(s) {
        let label = null;
        try {
          label = sessionLabel(s.session_id);
        } catch {
          label = null;
        }
        const text = String(label || "").trim();
        if (text) return text;
        const id = String(s.session_id || "");
        return id.length > 8 ? `${id.slice(0, 8)}…` : id || "Session";
      }

      function usageSessionSeverity(left) {
        if (left <= 10) return "danger";
        if (left <= 25) return "warning";
        return "normal";
      }

      function buildUsageSessionRow(s) {
        const row = document.createElement("div");
        row.className = "op-usage-sess__row";
        row.dataset.sessionId = String(s.session_id || "");
        row.dataset.eligible = s.eligible === false ? "false" : "true";

        const name = document.createElement("span");
        name.className = "op-usage-sess__name";
        name.textContent = usageSessionName(s);
        name.title = String(s.session_id || "");

        const model = document.createElement("span");
        model.className = "op-usage-sess__model";
        model.textContent = s.model ? String(s.model) : "";

        const stateReason =
          s.state && s.state.kind !== "ok" ? usageStateReason(s.state) : "";

        const tokens = document.createElement("span");
        tokens.className = "op-usage-sess__tokens";
        tokens.textContent = stateReason ? "—" : usageFmtTokens(s.total_tokens);
        tokens.title = stateReason
          ? stateReason
          : `Total tokens: in ${usageFmtTokens(s.input_tokens || 0)} · out ${usageFmtTokens(
              s.output_tokens || 0,
            )}`;

        const ctx = document.createElement("span");
        ctx.className = "op-usage-sess__ctx";
        const left = usageSessionContextLeft(s);
        if (s.eligible === false) {
          ctx.textContent = "n/a";
          ctx.title = "Not on the subscription quota (API-key backend or unsupported agent)";
        } else if (stateReason) {
          ctx.textContent = stateReason;
        } else if (left != null) {
          const bounded = Math.max(0, Math.min(100, left));
          ctx.textContent = `${Math.round(bounded)}% left`;
          ctx.dataset.severity = usageSessionSeverity(bounded);
          const used = s.context_used_tokens;
          const limit = s.context_limit_tokens;
          ctx.title =
            used != null && limit != null
              ? `Context remaining (approx.): ${usageFmtTokens(used)} / ${usageFmtTokens(limit)} used`
              : "Context remaining (approx.)";
        } else {
          ctx.textContent = "—";
          ctx.title = "Context limit unknown for this model";
        }
        if (s.limit_reached) row.dataset.limit = "true";

        row.appendChild(name);
        row.appendChild(model);
        row.appendChild(tokens);
        row.appendChild(ctx);
        return row;
      }

      // Bounded per-provider session list, or null when the snapshot carries
      // no sessions for the provider (nothing is rendered rather than a fake
      // empty table).
      function buildUsageSessionsSection(provider) {
        const sessions = usageSessionsFor(provider);
        if (!sessions.length) return null;
        const ordered = [...sessions].sort(compareUsageSessions);
        const section = document.createElement("div");
        section.className = "op-usage-sess";
        const head = document.createElement("div");
        head.className = "op-usage-sess__head";
        head.textContent = `Sessions (${ordered.length})`;
        section.appendChild(head);
        for (const s of ordered.slice(0, USAGE_SESSION_ROWS_MAX)) {
          section.appendChild(buildUsageSessionRow(s));
        }
        const hidden = ordered.length - USAGE_SESSION_ROWS_MAX;
        if (hidden > 0) {
          const more = document.createElement("div");
          more.className = "op-usage-sess__more";
          more.textContent = `+${hidden} more session${hidden === 1 ? "" : "s"}`;
          section.appendChild(more);
        }
        return section;
      }

      // A provider card: header (icon · name · plan) + rate-limit windows (or a
      // degraded reason) + consumption grid + 7-day sparkline + bounded session
      // list. Grouping all of one provider's data together is the key
      // readability win.
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
        if (account.limit_reached) {
          const limit = document.createElement("span");
          limit.className = "op-usage-card__limit";
          limit.textContent = "Limit reached";
          head.appendChild(limit);
        }
        card.appendChild(head);

        if (account.account_label) {
          const accountLine = document.createElement("div");
          accountLine.className = "op-usage-card__account";
          accountLine.textContent = `Account: ${account.account_label}`;
          card.appendChild(accountLine);
        }

        const windows = account.windows || [];
        if (windows.length) {
          const wins = document.createElement("div");
          wins.className = "op-usage-wins";
          for (const w of windows) {
            wins.appendChild(buildUsageWindowRow(w, account.limit_reached));
          }
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

        const sessions = buildUsageSessionsSection(account.provider);
        if (sessions) card.appendChild(sessions);
        return card;
      }

      // SPEC-2970 — the full usage detail as provider cards, appended to a
      // container. The hover popover is the single surface for all usage info
      // (the click-open modal was removed per UX feedback). Per-session
      // tokens / context live inside each provider card as a bounded list
      // (Issue #3862) — the earlier unbounded table was removed because it
      // grew to hundreds of rows.
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
        const restoreKeyboardFocus = usageHoverEl.contains(document.activeElement);
        while (usageHoverEl.firstChild) usageHoverEl.removeChild(usageHoverEl.firstChild);
        usageHoverEl.appendChild(buildUsageHoverBody());
        positionUsageHover();
        if (restoreKeyboardFocus) {
          requestAnimationFrame(() => usageHoverEl?.focus());
        }
      }

      window.__gwtShowUsageHover = (anchor) => {
        cancelUsageHoverHide();
        if (usageHoverAnchor && usageHoverAnchor !== anchor) {
          usageHoverAnchor.setAttribute("aria-expanded", "false");
        }
        usageHoverAnchor = anchor || usageHoverAnchor;
        if (!usageHoverEl) {
          usageHoverEl = document.createElement("div");
          usageHoverEl.className = "op-usage-hover";
          usageHoverEl.id = "provider-usage-popover";
          usageHoverEl.setAttribute("role", "region");
          usageHoverEl.setAttribute("aria-label", "Usage & Limits");
          usageHoverEl.setAttribute("tabindex", "0");
          usageHoverEl.addEventListener("mouseenter", cancelUsageHoverHide);
          usageHoverEl.addEventListener("mouseleave", () => window.__gwtHideUsageHover());
          usageHoverEl.addEventListener("focusin", cancelUsageHoverHide);
          usageHoverEl.addEventListener("focusout", (event) => {
            if (
              usageHoverEl?.contains(event.relatedTarget) ||
              event.relatedTarget === usageHoverAnchor
            ) {
              return;
            }
            window.__gwtHideUsageHover();
          });
          usageHoverEl.addEventListener("keydown", (event) => {
            if (event.key !== "Escape") return;
            event.preventDefault();
            const anchor = usageHoverAnchor;
            anchor?.focus();
            window.__gwtHideUsageHover({ immediate: true });
          });
          document.body.appendChild(usageHoverEl);
        }
        while (usageHoverEl.firstChild) usageHoverEl.removeChild(usageHoverEl.firstChild);
        usageHoverEl.appendChild(buildUsageHoverBody());
        usageHoverEl.hidden = false;
        usageHoverAnchor?.setAttribute("aria-expanded", "true");
        usageHoverEl.style.visibility = "hidden";
        requestAnimationFrame(() => {
          positionUsageHover();
          if (usageHoverEl) usageHoverEl.style.visibility = "visible";
        });
      };

      window.__gwtUsageHoverContains = (node) => Boolean(node && usageHoverEl?.contains(node));
      window.__gwtFocusUsageHover = () => usageHoverEl?.focus();
      window.__gwtHideUsageHover = (options = {}) => {
        cancelUsageHoverHide();
        if (options.immediate) {
          if (usageHoverEl) usageHoverEl.hidden = true;
          usageHoverAnchor?.setAttribute("aria-expanded", "false");
          usageHoverAnchor = null;
          return;
        }
        usageHoverHideTimer = setTimeout(() => {
          if (usageHoverEl) usageHoverEl.hidden = true;
          usageHoverAnchor?.setAttribute("aria-expanded", "false");
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
