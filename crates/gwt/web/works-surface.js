const finite = value => typeof value === "number" && Number.isFinite(value);
const number = value => finite(value) ? new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value) : "—";
const seconds = value => finite(value) ? `${Math.floor(value / 3600)}h ${Math.floor(value % 3600 / 60)}m` : "—";
const stateFor = status => ({ running: "running", starting: "running", waiting: "waiting", idle: "idle", ready: "idle", error: "error", interrupted: "error" })[status] || "unknown";
const labelFor = state => state.charAt(0).toUpperCase() + state.slice(1);

// Live activity owns membership; a separately arriving catalog only enriches
// the row and authorizes access to its exact session's local terminal.
function currentWorks(catalog, activity, roster) {
  const app = catalog.appState, members = roster.members || [];
  const windows = (app.tabs || []).filter(tab => !activity.snapshot?.project_key
    || tab.project_key === activity.snapshot.project_key).flatMap(tab => tab.workspace?.windows || []);
  const order = work => {
    if (work.member) return members.indexOf(work.member);
    const index = windows.findIndex(data => data.id === work.data.id);
    return members.length + (index < 0 ? windows.length : index);
  };
  return (activity.agents || []).map(assignment => {
    const matched = windows.find(data => data.id === assignment.window_id
      && data.session_id === assignment.session_id && data.preset !== "shell");
    const data = matched || { id: assignment.window_id };
    const member = assignment.is_pm ? null : members.find(member => member.assignment?.session_id === assignment.session_id);
    return { data, assignment, member, ready: !!matched,
      state: matched ? stateFor(catalog.runtimeStates[data.id] ?? data.status) : "unknown" };
  }).sort((a, b) => Number(!!b.assignment.is_pm) - Number(!!a.assignment.is_pm) || order(a) - order(b));
}

function drawSpark(document, svg, input, maximum) {
  const values = Array.from({ length: 30 }, (_, index) => finite(input?.[index]) ? Math.max(0, input[index]) : null);
  const known = values.filter(finite);
  svg.replaceChildren();
  svg.setAttribute("viewBox", "0 0 300 56");
  svg.setAttribute("preserveAspectRatio", "none");
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", `Tokens per minute, last 30 minutes: ${known.length} measured minutes, ${30 - known.length} unavailable`);
  svg.dataset.knownCount = String(known.length);
  const max = Math.max(1, maximum || 0, ...known);
  let points = [];
  const append = () => {
    if (!points.length) return;
    const line = document.createElementNS("http://www.w3.org/2000/svg", "polyline");
    line.setAttribute("points", points.join(" "));
    line.setAttribute("class", "works-spark-line");
    svg.append(line); points = [];
  };
  values.forEach((value, index) => {
    if (value === null) { append(); return; }
    points.push(`${(index * 300 / 29).toFixed(1)},${(53 - value * 49 / max).toFixed(1)}`);
    const dot = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    dot.setAttribute("cx", (index * 300 / 29).toFixed(1));
    dot.setAttribute("cy", (53 - value * 49 / max).toFixed(1)); dot.setAttribute("r", "1.5");
    svg.append(dot);
  });
  append();
}

export function createWorksSurface({ document, windowCatalogModel, activityModel, rosterModel, windowDisplayTitle, systemSettingsModel,
  mountTerminal, mountPreview, onLayout = () => {}, onFocus = () => {}, onSelectMember = () => {}, getTelemetry = () => null }) {
  const element = document.createElement("section");
  element.className = "works-surface"; element.setAttribute("aria-label", "Works");
  element.innerHTML = `<div class="works-overview"><header class="works-heading"><h2>Works</h2><div class="works-counts">
    ${["running", "waiting", "idle", "error"].map(state => `<span>${state.toUpperCase()} <b data-count="${state}">—</b></span>`).join("")}</div><span class="works-active"></span></header>
    <div class="works-today">Today · Tokens <b data-metric="today-total">—</b> · Input <b data-metric="today-input">—</b> · Cache <b data-metric="today-cache">—</b> · Output <b data-metric="today-output">—</b> · Cost <b data-metric="today-cost">—</b> · Landed PRs <b data-metric="today-landed">—</b>
      <small class="works-availability" data-availability="today"></small><small class="works-availability works-source-note"></small></div>
    <div class="works-fleet"><div>Fleet tokens/min <b data-metric="fleet-now">—</b><small>Last 30 minutes</small></div><svg></svg><small class="works-availability" data-availability="fleet"></small></div>
    <div class="works-list" role="list" aria-label="Current agent works"></div><p class="works-empty">No local agents are active.</p></div>
    <section class="works-full" aria-label="Work terminal" hidden><header class="works-full-header"><button type="button" data-action="back">← Back to Works</button>
      <span class="works-full-name"></span><span class="works-full-meta"></span><span class="works-full-usage"></span>
      <button type="button" data-action="previous" aria-label="Previous work">↑</button><button type="button" data-action="next" aria-label="Next work">↓</button></header><div class="works-terminal-host"></div></section>`;
  const overview = element.querySelector(".works-overview"), list = element.querySelector(".works-list"), full = element.querySelector(".works-full");
  const entries = new Map();
  let selected = null, previewElement = null, previewCleanups = [], disposed = false;
  let renderedCatalog, renderedActivity, renderedRoster;
  const previewLines = () => systemSettingsModel?.read().worksPreviewLines ?? 3;
  const isPresented = () => element.isConnected && !element.hidden;
  const nameFor = work => work.member?.name || work.data.agent_name || (work.assignment.is_pm ? "Project Manager" : "Agent");
  const purposeFor = work => work.assignment.current_task || work.data.dynamic_title || work.data.purpose_title || (work.assignment.is_pm ? "Project Manager" : work.data.title) || "—";
  const providerFor = work => work.member?.provider || work.data.agent_id || work.data.preset || "—";
  const telemetryFor = work => getTelemetry()?.by_window?.[work.data.id] || {};
  function setName(host, work) {
    const memberId = work.member?.member_id || "";
    let name = host.firstElementChild;
    if (!name || name.dataset.memberId !== memberId) {
      host.replaceChildren();
      name = document.createElement(memberId ? "button" : "span");
      name.dataset.memberId = memberId;
      name.className = memberId ? "work-member work-name" : "work-name";
      if (memberId) {
        name.type = "button";
        name.addEventListener("click", event => { event.stopPropagation(); onSelectMember(memberId); });
      }
      host.append(name);
    }
    name.textContent = nameFor(work);
    if (memberId) name.setAttribute("aria-label", `View ${nameFor(work)} in Agents`);
    let badge = host.querySelector(".window-role-badge");
    if (work.assignment.is_pm && !badge) {
      badge = document.createElement("span"); badge.className = "window-role-badge"; badge.textContent = "PM";
      host.append(badge);
    } else if (!work.assignment.is_pm) badge?.remove();
  }
  function park(entry, rehost = false) {
    const moved = entry.terminal.parentElement !== entry.cache;
    if (moved) entry.cache.append(entry.terminal);
    if (exactWork(entry.work.assignment) && isPresented() && (moved || rehost)) mountTerminal(entry.work.data.id, entry.terminal, { readOnly: true });
  }
  function back(restore = true) {
    const previous = selected; selected = null; full.hidden = true; overview.hidden = false;
    for (const entry of entries.values()) park(entry);
    if (restore) entries.get(previous?.window_id)?.row.focus({ preventScroll: true });
    syncPreview(); onLayout();
  }
  function exactWork(target) {
    const requested = typeof target === "string" ? { window_id: target } : target;
    if (!requested?.window_id) return null;
    return currentWorks(windowCatalogModel.read().catalog, activityModel.read(), rosterModel.read()).find(work =>
      work.ready && work.assignment.window_id === requested.window_id &&
      (!requested.session_id || requested.session_id === work.assignment.session_id) &&
      (requested.runtime_incarnation === undefined || requested.runtime_incarnation === work.assignment.runtime_incarnation));
  }
  function showFull(work, rehost = true) {
    selected = work.assignment; overview.hidden = true; full.hidden = false;
    const entry = entries.get(work.data.id);
    for (const other of entries.values()) if (other !== entry) park(other);
    const host = full.querySelector(".works-terminal-host");
    if (entry.terminal.parentElement !== host) host.append(entry.terminal);
    setName(full.querySelector(".works-full-name"), work);
    const t = telemetryFor(work);
    const available = work.data.session_id && ["running", "waiting", "idle"].includes(work.state);
    full.querySelector(".works-full-meta").textContent = `${work.assignment.is_pm ? "PM" : work.assignment.issue_number ? `#${work.assignment.issue_number}` : "—"} · ${purposeFor(work)} · ${labelFor(work.state)} · ${seconds(t.elapsed_seconds)} · ${providerFor(work)} / ${work.member?.model || t.model || "—"} · ${available ? "Interactive" : "Input unavailable"}`;
    full.querySelector(".works-full-usage").textContent = `${number(t.input_tokens)} input / ${number(t.cache_tokens)} cache / ${number(t.output_tokens)} output · Context ${finite(t.context_remaining_percent) ? `${t.context_remaining_percent}%` : "—"}`;
    if (rehost && isPresented()) mountTerminal(work.data.id, entry.terminal, { readOnly: false });
  }
  function openWork(target) {
    if (disposed) return false;
    render(false);
    const work = exactWork(target);
    if (!work) { back(false); return false; }
    showFull(work); syncPreview(); onLayout(); onFocus(work.data.id); return true;
  }
  function createEntry(work) {
    const row = document.createElement("article"); row.className = "work-row"; row.setAttribute("role", "listitem"); row.tabIndex = 0;
    row.dataset.windowId = work.data.id;
    row.innerHTML = `<div class="work-stripe" aria-hidden="true"></div><div class="work-body"><div class="work-who"><div class="work-name-host"></div><span class="work-purpose"></span><span class="work-identity"></span><button type="button" class="work-open">Open terminal</button></div>
      <div class="work-tail"></div><div class="work-side"><svg class="work-spark"></svg><small>Tokens/min · Last 30 minutes</small><span class="work-state"></span><span class="work-elapsed"></span><span class="work-model"></span><span class="work-warnings"></span></div>
      <div class="work-stats"><span>Input <b data-stat="input"></b> · Cache <b data-stat="cache"></b> · Output <b data-stat="output"></b></span><span>Context remaining <b data-stat="context"></b></span><span>Branch <b data-stat="branch"></b> · Ahead of develop <b data-stat="ahead"></b></span><span>Verification <b data-stat="verification"></b></span><span>Last action <b data-stat="last"></b></span></div></div>`;
    const cache = document.createElement("div"); cache.className = "work-runtime-cache"; cache.setAttribute("inert", ""); cache.setAttribute("aria-hidden", "true");
    const terminal = document.createElement("div"); terminal.className = "works-terminal terminal-root";
    cache.dataset.windowId = work.data.id;
    cache.append(terminal); element.append(cache);
    terminal.dataset.windowId = work.data.id;
    terminal.addEventListener("focusin", () => onFocus(work.data.id));
    terminal.addEventListener("pointerdown", () => onFocus(work.data.id));
    row.addEventListener("click", event => { if (!event.target.closest("button, a, .work-tail")) openWork(entries.get(work.data.id).work.assignment); });
    row.addEventListener("keydown", event => {
      if (event.target !== row || !["Enter", " "].includes(event.key)) return;
      event.preventDefault(); openWork(entries.get(work.data.id).work.assignment);
    });
    row.querySelector(".work-open").addEventListener("click", () => openWork(entries.get(work.data.id).work.assignment));
    list.append(row);
    return { row, terminal, cache, work, cleanup: null, previewMounted: false };
  }
  function updateEntry(entry, work, max) {
    entry.work = work;
    const row = entry.row, t = telemetryFor(work);
    row.setAttribute("aria-disabled", String(!work.ready));
    row.querySelector(".work-open").disabled = !work.ready;
    row.querySelector(".work-open").textContent = work.ready ? "Open terminal" : "Syncing terminal…";
    row.dataset.state = work.state;
    row.setAttribute("aria-label", `${nameFor(work)} · ${purposeFor(work)} · ${labelFor(work.state)}. Open terminal`);
    setName(row.querySelector(".work-name-host"), work);
    row.querySelector(".work-purpose").textContent = purposeFor(work);
    row.querySelector(".work-purpose").title = windowDisplayTitle(work.data);
    row.querySelector(".work-identity").textContent = `${work.assignment.is_pm ? "PM" : work.assignment.issue_number ? `#${work.assignment.issue_number}` : "—"} · ${providerFor(work)}`;
    row.querySelector(".work-state").textContent = labelFor(work.state);
    row.querySelector(".work-elapsed").textContent = seconds(t.elapsed_seconds);
    row.querySelector(".work-model").textContent = work.member?.model || t.model || "—";
    row.querySelector(".work-warnings").textContent = `Warnings ${number(t.warning_count)}`;
    for (const [key, value] of Object.entries({ input: number(t.input_tokens), cache: number(t.cache_tokens), output: number(t.output_tokens), context: finite(t.context_remaining_percent) ? `${t.context_remaining_percent}%` : "—", branch: t.branch || "—", ahead: number(t.ahead_of_develop), verification: t.verification || "—", last: t.last_action || "—" })) row.querySelector(`[data-stat="${key}"]`).textContent = value;
    drawSpark(document, row.querySelector("svg"), t.tokens_per_minute, max);
  }
  function syncPreview() {
    if (!previewElement) return;
    previewElement.style.setProperty("--works-preview-lines", String(previewLines()));
    for (const cleanup of previewCleanups) cleanup?.(); previewCleanups = [];
    previewElement.replaceChildren();
    const note = document.createElement("p"); note.className = "surface-readonly-note"; note.textContent = "Read-only preview · Select this pane to interact"; previewElement.append(note);
    if (selected) {
      if (!exactWork(selected)) return;
      const heading = full.querySelector(".works-full-header").cloneNode(true);
      for (const button of heading.querySelectorAll("button")) { button.disabled = true; button.tabIndex = -1; }
      const output = document.createElement("div"); output.className = "works-preview-output";
      previewElement.append(heading, output);
      previewCleanups.push(mountPreview(selected.window_id, output));
    } else {
      const clone = overview.cloneNode(true);
      for (const cache of clone.querySelectorAll(".work-runtime-cache")) cache.remove();
      for (const button of clone.querySelectorAll("button")) { button.disabled = true; button.tabIndex = -1; }
      for (const row of clone.querySelectorAll(".work-row")) {
        row.tabIndex = -1; const tail = row.querySelector(".work-tail"); tail.replaceChildren();
        if (!exactWork(entries.get(row.dataset.windowId)?.work.assignment)) continue;
        previewCleanups.push(mountPreview(row.dataset.windowId, tail, { tailLines: previewLines() }));
      }
      previewElement.append(clone);
    }
    previewElement.dataset.catalogVersion = element.dataset.catalogVersion;
  }
  function setPreviewHost(host) {
    if (previewElement?.parentElement === host) return;
    for (const cleanup of previewCleanups) cleanup?.(); previewCleanups = [];
    previewElement?.remove(); previewElement = null;
    if (!host || disposed) return;
    previewElement = document.createElement("section"); previewElement.className = "works-surface works-surface--preview";
    previewElement.setAttribute("aria-label", "Works read-only preview"); host.append(previewElement); syncPreview();
  }
  function render(rehost) {
    if (disposed) return;
    const lines = previewLines();
    const linesChanged = element.style.getPropertyValue("--works-preview-lines") !== String(lines);
    element.style.setProperty("--works-preview-lines", String(lines));
    const catalog = windowCatalogModel.read().catalog, activity = activityModel.read(), roster = rosterModel.read();
    const works = currentWorks(catalog, activity, roster), ids = new Set(works.map(work => work.data.id));
    let changed = false;
    for (const [id, entry] of entries) if (!ids.has(id) || !works.some(work => work.data.id === id && work.assignment.session_id === entry.work.assignment.session_id && work.assignment.runtime_incarnation === entry.work.assignment.runtime_incarnation)) {
      entry.cleanup?.(); entry.row.remove(); entry.cache.remove(); entry.terminal.remove(); entries.delete(id); changed = true;
    }
    const telemetry = getTelemetry() || {}, today = telemetry.today || {};
    const maximum = Math.max(1, ...works.flatMap(work => telemetryFor(work).tokens_per_minute || []).filter(finite));
    works.forEach((work, index) => {
      if (!entries.has(work.data.id)) { entries.set(work.data.id, createEntry(work)); changed = true; }
      const entry = entries.get(work.data.id);
      updateEntry(entry, work, maximum);
      if (list.children[index] !== entry.row) list.insertBefore(entry.row, list.children[index] || null);
    });
    for (const state of ["running", "waiting", "idle", "error"]) element.querySelector(`[data-count="${state}"]`).textContent = activity.count === null ? "—" : String(works.filter(work => work.state === state).length);
    element.querySelector(".works-active").textContent = `Active ${number(activity.count)}`;
    for (const [key, value] of Object.entries({ total: finite(today.input_tokens) && finite(today.output_tokens) ? number(today.input_tokens + today.output_tokens) : "—", input: number(today.input_tokens), cache: number(today.cache_tokens), output: number(today.output_tokens), cost: finite(today.cost_usd) ? `$${today.cost_usd.toFixed(2)}` : "—", landed: number(today.landed_pr_count) })) element.querySelector(`[data-metric="today-${key}"]`).textContent = value;
    for (const key of ["today", "fleet"]) {
      const note = element.querySelector(`[data-availability="${key}"]`);
      const status = telemetry.availability?.[key];
      note.dataset.state = status?.state || "no_data";
      note.textContent = status?.detail || "No data: session measurements are not available yet.";
    }
    const sourceNote = element.querySelector(".works-source-note");
    sourceNote.textContent = [!finite(today.cost_usd) && "Cost unavailable: per-session USD is not supported.",
      !finite(today.landed_pr_count) && "Landed PRs unavailable: daily merge data is not connected."].filter(Boolean).join(" ");
    sourceNote.hidden = !sourceNote.textContent;
    element.querySelector('[data-metric="fleet-now"]').textContent = number(telemetry.fleet_tokens_per_minute?.at(-1));
    drawSpark(document, element.querySelector(".works-fleet svg"), telemetry.fleet_tokens_per_minute);
    element.querySelector(".works-empty").textContent = activity.count === null ? "Local agent activity is unavailable." : "No local agents are active.";
    element.querySelector(".works-empty").hidden = works.length > 0;
    element.dataset.catalogVersion = String(catalog.version);
    if (selected) { const work = exactWork(selected); if (work) showFull(work, rehost); else back(false); }
    for (const entry of entries.values()) if (!entry.work.ready || linesChanged) {
      entry.cleanup?.(); entry.cleanup = null; entry.previewMounted = false;
    }
    if (isPresented()) for (const entry of entries.values()) {
      if (!entry.work.ready) continue;
      if (!selected || entry.work.data.id !== selected.window_id) park(entry, rehost || !entry.previewMounted);
      if (!entry.previewMounted) {
        entry.cleanup = mountPreview(entry.work.data.id, entry.row.querySelector(".work-tail"), { tailLines: previewLines() });
        entry.previewMounted = true;
      }
    }
    renderedCatalog = catalog; renderedActivity = activity; renderedRoster = roster; syncPreview();
    if (changed || rehost) onLayout();
  }
  element.querySelector('[data-action="back"]').addEventListener("click", () => back());
  for (const [action, delta] of [["previous", -1], ["next", 1]]) element.querySelector(`[data-action="${action}"]`).addEventListener("click", () => {
    const works = currentWorks(windowCatalogModel.read().catalog, activityModel.read(), rosterModel.read()).filter(work => work.ready);
    const index = works.findIndex(work => work.assignment.window_id === selected?.window_id && work.assignment.session_id === selected?.session_id && work.assignment.runtime_incarnation === selected?.runtime_incarnation);
    if (index < 0 || !works.length) { back(false); return; }
    openWork(works[(index + delta + works.length) % works.length].assignment);
  });
  render(false);
  const unsubscribers = [
    windowCatalogModel.subscribeFrame(state => state.catalog, value => { if (value !== renderedCatalog) render(false); }),
    activityModel.subscribeFrame(state => state, value => { if (value !== renderedActivity) render(false); }),
    rosterModel.subscribeFrame(state => state, value => { if (value !== renderedRoster) render(false); }),
    systemSettingsModel?.subscribeFrame(state => state.worksPreviewLines, () => render(false)),
  ];
  return { element, sync: () => render(true), openWork, reveal: openWork, showList: () => back(false), setPreviewHost,
    contains: id => !!exactWork(id), isVisible: id => !element.hidden && !!exactWork(id),
    dispose() { if (disposed) return; for (const unsubscribe of unsubscribers) unsubscribe?.(); setPreviewHost(null); for (const entry of entries.values()) entry.cleanup?.(); entries.clear(); disposed = true; },
  };
}
