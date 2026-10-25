const finite = value => typeof value === "number" && Number.isFinite(value);
const number = value => finite(value) ? new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value) : "—";
const hours = value => finite(value) ? `${Math.round(value / 3600 * 10) / 10}h` : "—";
const activityPresentation = state => JSON.stringify([state.count, (state.agents || []).map(agent =>
  [agent.window_id, agent.session_id, agent.runtime_incarnation, agent.issue_number, agent.is_pm])
  .sort((left, right) => String(left[0]).localeCompare(String(right[0])))]);
const date = value => value && Number.isFinite(Date.parse(value)) ? new Date(value).toISOString().replace("T", " ").slice(0, 16) : "—";

export function createAgentsSurface({ document, rosterModel, activityModel, windowCatalogModel,
  scheduleModel, onReassign = () => {}, onRefreshSchedule = () => {},
  onAddMember = () => {}, onRestMember = () => {}, onOpenWork = () => {} }) {
  const element = document.createElement("section"); element.className = "agents-surface"; element.setAttribute("aria-label", "Agents");
  element.innerHTML = `<header class="roster-heading"><h2>Agents</h2><div class="roster-tabs" role="tablist" aria-label="Agent roster views"><button type="button" id="roster-tab-team" data-tab="team" role="tab" aria-controls="roster-team" aria-selected="true">Team</button><button type="button" id="roster-tab-past" data-tab="past" role="tab" aria-controls="roster-past" aria-selected="false" tabindex="-1">Past</button></div><span class="roster-active"></span></header>
    <div class="roster-pm-identity" role="group" aria-label="Project Manager" hidden><span class="roster-pm-name"></span><span class="window-role-badge">PM</span></div>
    <p class="roster-error" role="status" hidden></p><section class="roster-team" id="roster-team" role="tabpanel" aria-labelledby="roster-tab-team"><div class="roster-capacity"><div><b class="roster-count">—</b><span> enabled members<br>Concurrent capacity <b class="roster-capacity-value">—</b></span></div><div class="roster-capacity-scale"><div class="roster-capacity-bar" role="meter" aria-label="Enabled members"><span class="roster-capacity-fill"></span><span class="roster-capacity-limit"></span></div><small class="roster-capacity-note"></small></div><button type="button" data-action="rest">− Rest member</button><button type="button" data-action="add">＋ Add member</button></div>
    <p class="roster-description">Named members are reused across issues. Current work and terminals are in Works.</p><div class="roster-split"><div><div class="roster-list" role="list" aria-label="Team members"></div><button type="button" class="roster-add" data-action="add"><span aria-hidden="true">＋</span> Add a member · Increase concurrent capacity by one</button></div><aside class="roster-detail" aria-label="Selected member"></aside></div></section>
    <section class="roster-past" id="roster-past" role="tabpanel" aria-labelledby="roster-tab-past" hidden><div class="roster-past-kpis"></div><p class="roster-description">Previous contributors recorded in sessions and Board. Periods are shown in UTC.</p><table><thead><tr><th>Agent</th><th>Kind</th><th>Issues</th><th>Period (UTC)</th><th>Posts</th></tr></thead><tbody></tbody></table><p class="roster-past-empty">No previous contributors are recorded.</p></section>`;
  let selectedId = null, tab = "team", previewElement = null, disposed = false;
  let renderedRoster, renderedActivity, renderedCatalog, renderedSchedule;
  let renderedPlans = null;
  let submittedSnapshot = null, scheduleNotice = "";
  const scheduleState = () => scheduleModel?.read() || { snapshot: null, error: null, pending: false };
  function liveWork(member) {
    if (!member?.assignment?.session_id) return null;
    // The project-scoped child census proves membership independently of
    // catalog hydration and whichever project tab is currently selected.
    return (activityModel.read().agents || []).find(assignment => !assignment.is_pm
      && assignment.session_id === member.assignment.session_id) || null;
  }
  function workTitle(assignment) {
    const app = windowCatalogModel.read().catalog.appState, activity = activityModel.read();
    const data = (app.tabs || []).filter(tab => !activity.snapshot?.project_key
      || tab.project_key === activity.snapshot.project_key).flatMap(tab => tab.workspace?.windows || [])
      .find(data => data.id === assignment.window_id && data.session_id === assignment.session_id);
    return assignment.current_task || data?.dynamic_title || data?.title || (assignment.issue_number ? `Issue #${assignment.issue_number}` : "—");
  }
  function openCurrent(memberId, expected) {
    const member = rosterModel.read().members.find(member => member.member_id === memberId);
    const current = liveWork(member);
    if (current && current.window_id === expected.window_id && current.session_id === expected.session_id && current.runtime_incarnation === expected.runtime_incarnation) onOpenWork(current);
  }
  function stats(member, className = "") {
    const result = document.createElement("div"); result.className = `roster-stats ${className}`;
    for (const [label, value] of [["Assigned issues", number(member.issue_count)], ["Landed", number(member.landed_count)], ["Active time", hours(member.active_seconds)], ["Tokens", number(member.tokens)]]) {
      const stat = document.createElement("div"); stat.className = "roster-stat";
      const b = document.createElement("b"), span = document.createElement("span"); b.textContent = value; span.textContent = label; stat.append(b, span); result.append(stat);
    }
    return result;
  }
  function avatar(member) {
    const result = document.createElement("span"); result.className = "roster-avatar"; result.setAttribute("aria-hidden", "true"); result.textContent = Array.from(member.name || "?")[0]; return result;
  }
  function selectMember(memberId) {
    if (disposed || !rosterModel.read().members.some(member => member.member_id === memberId && (member.enabled || liveWork(member)))) return false;
    selectedId = memberId; tab = "team"; sync();
    [...element.querySelectorAll(".roster-row")].find(row => row.dataset.memberId === memberId)?.focus({ preventScroll: true });
    return true;
  }
  function restSelected() {
    const member = rosterModel.read().members.find(member => member.member_id === selectedId && member.enabled);
    if (member && rosterModel.read().members.filter(member => member.enabled).length > 1) onRestMember(member.member_id);
  }
  function renderPlans(member) {
    const state = scheduleState(), snapshot = state.snapshot;
    const pending = state.pending || submittedSnapshot === snapshot && snapshot !== null;
    const inputs = [member.member_id, member.enabled, rosterModel.read().revision, snapshot, state.error, pending, scheduleNotice];
    if (renderedPlans?.inputs.every((value, index) => value === inputs[index])) return renderedPlans.section;
    const section = document.createElement("section"); section.className = "roster-plans";
    renderedPlans = { inputs, section };
    const header = document.createElement("header"), heading = document.createElement("h4"), refresh = document.createElement("button");
    heading.textContent = "Next planned issues"; refresh.type = "button"; refresh.dataset.action = "refresh-plans"; refresh.textContent = "Refresh plans";
    refresh.addEventListener("click", () => { submittedSnapshot = null; scheduleNotice = ""; onRefreshSchedule(); sync(); });
    header.append(heading, refresh); section.append(header);
    const plan = snapshot?.members.find(plan => plan.agent_member_id === member.member_id);
    const stale = snapshot && snapshot.roster_revision !== rosterModel.read().revision;
    const status = document.createElement("p"); status.className = "roster-plan-status"; status.setAttribute("role", "status");
    status.textContent = state.error || scheduleNotice || (pending ? "Updating plans…" : stale ? "Plans changed. Refresh plans before reassigning." : "");
    status.hidden = !status.textContent; section.append(status);
    if (!plan?.issues.length) {
      const empty = document.createElement("p"); empty.className = "roster-plan-empty";
      empty.textContent = plan ? "No queued issue" : "Plans unavailable. Refresh plans to try again."; section.append(empty); return section;
    }
    const list = document.createElement("ol");
    for (const issue of plan.issues) {
      const item = document.createElement("li"); item.className = "roster-plan"; item.dataset.issueNumber = String(issue.issue_number);
      const title = document.createElement("strong"), reason = document.createElement("span");
      title.textContent = `#${issue.issue_number}`; reason.className = "roster-plan-reason";
      reason.textContent = { same_spec_continuation: "Same SPEC continuation", queue_order: "Queue order", assigned_by_you: "Assigned by you" }[issue.reason] || "Reason unavailable";
      const controls = document.createElement("div"); controls.className = "roster-plan-controls";
      const select = document.createElement("select"), reassign = document.createElement("button");
      select.setAttribute("aria-label", `Assign issue #${issue.issue_number} to member`);
      const placeholder = document.createElement("option"); placeholder.value = ""; placeholder.textContent = "Choose member"; select.append(placeholder);
      for (const target of rosterModel.read().members.filter(target => target.enabled && target.member_id !== member.member_id)) {
        const option = document.createElement("option"); option.value = target.member_id; option.textContent = target.name; select.append(option);
      }
      const blocked = pending || !!state.error || stale || !member.enabled;
      select.disabled = blocked || select.options.length <= 1;
      reassign.type = "button"; reassign.textContent = "Reassign"; reassign.setAttribute("aria-label", `Reassign issue #${issue.issue_number}`); reassign.disabled = true;
      select.addEventListener("change", () => { reassign.disabled = blocked || !select.value; });
      reassign.addEventListener("click", () => {
        const latest = scheduleState(), roster = rosterModel.read();
        if (latest.pending || submittedSnapshot || latest.error) return;
        const source = roster.members.find(target => target.member_id === member.member_id && target.enabled);
        const target = roster.members.find(target => target.member_id === select.value && target.enabled && target.member_id !== member.member_id);
        if (latest.snapshot !== snapshot || roster.revision !== snapshot.roster_revision || !source || !target) {
          scheduleNotice = "Plans changed. Refresh plans before reassigning."; sync(); return;
        }
        submittedSnapshot = snapshot;
        onReassign({ kind: "reassign", issue_number: issue.issue_number, source_member_id: member.member_id, target_member_id: target.member_id,
          expected_roster_revision: snapshot.roster_revision, expected_queue_revision: snapshot.queue_revision, expected_schedule_revision: snapshot.schedule_revision });
        sync();
      });
      controls.append(select, reassign); item.append(title, reason, controls); list.append(item);
    }
    section.append(list); return section;
  }
  function renderDetail(member) {
    const panel = element.querySelector(".roster-detail"), plans = member ? renderPlans(member) : null;
    // Keep unchanged native plan controls attached while telemetry and current
    // work refresh around them, preserving selection and browser keyboard focus.
    for (const child of [...panel.childNodes]) if (child !== plans) child.remove();
    if (!member) { panel.textContent = "No enabled members. Add a member to begin."; return; }
    if (plans.parentElement !== panel) panel.append(plans);
    const head = document.createElement("header"); head.className = "roster-detail-heading";
    const who = document.createElement("div"); const name = document.createElement("h3"), meta = document.createElement("p");
    name.textContent = member.name; meta.textContent = `${member.provider || "—"} · ${member.model || "—"} · Member since ${date(member.created_at)} UTC`; who.append(name, meta); head.append(avatar(member), who); panel.insertBefore(head, plans); panel.insertBefore(stats(member), plans);
    const history = document.createElement("section"); history.className = "roster-history"; history.innerHTML = "<h4>Assignment history</h4><ol></ol>";
    const ol = history.querySelector("ol"), current = liveWork(member);
    if (current) {
      const currentWork = document.createElement("section"); currentWork.className = "roster-history roster-current-work";
      currentWork.innerHTML = "<h4>Current work</h4><ol></ol>";
      const li = document.createElement("li"); li.className = "roster-now";
      const label = document.createElement("small"), button = document.createElement("button"), result = document.createElement("span");
      label.textContent = "Now"; button.type = "button"; button.textContent = `${current.issue_number ? `#${current.issue_number}` : "—"} ${workTitle(current)}`;
      button.addEventListener("click", () => openCurrent(member.member_id, current)); result.textContent = "Current work"; li.append(label, button, result);
      currentWork.querySelector("ol").append(li); panel.insertBefore(currentWork, plans);
    }
    const historyEntries = [...(member.history || [])].filter(item => !current || item.session_id !== current.session_id).sort((a, b) => (Date.parse(b.ended_at || b.started_at) || 0) - (Date.parse(a.ended_at || a.started_at) || 0));
    for (const item of historyEntries) {
      const li = document.createElement("li"), time = document.createElement("small"), title = document.createElement("span"), result = document.createElement("small");
      time.textContent = `${date(item.started_at)} – ${date(item.ended_at)} UTC`; title.textContent = `${item.issue_number ? `#${item.issue_number}` : "—"} ${item.title || "—"}`; result.textContent = item.result || "—";
      li.append(time, title, result); ol.append(li);
    }
    if (!ol.childNodes.length) { const empty = document.createElement("li"); empty.textContent = "No assignment history yet."; ol.append(empty); }
    if (member.assignment?.session_id && activityModel.read().count === null) {
      const unavailable = document.createElement("p"); unavailable.className = "roster-description"; unavailable.textContent = "Activity unavailable"; panel.append(unavailable);
    }
    panel.append(history);
    const actions = document.createElement("div"); actions.className = "roster-actions";
    if (current) { const open = document.createElement("button"); open.type = "button"; open.className = "roster-open"; open.textContent = "Open terminal in Works"; open.addEventListener("click", () => openCurrent(member.member_id, current)); actions.append(open); }
    const rest = document.createElement("button"); rest.type = "button"; rest.dataset.action = "rest"; rest.textContent = "Rest member"; rest.disabled = !member.enabled || rosterModel.read().members.filter(member => member.enabled).length <= 1; rest.addEventListener("click", restSelected); actions.append(rest); panel.append(actions);
  }
  function renderPast(contributors, complete) {
    const kpis = element.querySelector(".roster-past-kpis"); kpis.replaceChildren();
    for (const [label, value] of [["Contributors", contributors.length], ["Implementation", contributors.filter(item => item.kind === "implementation").length], ["Review", contributors.filter(item => item.kind === "review").length], ["Posts", contributors.every(item => finite(item.post_count)) ? contributors.reduce((total, item) => total + item.post_count, 0) : null]]) {
      const stat = document.createElement("div"); stat.className = "roster-stat"; const b = document.createElement("b"), span = document.createElement("span"); b.textContent = number(complete === false ? null : value); span.textContent = label; stat.append(b, span); kpis.append(stat);
    }
    const body = element.querySelector(".roster-past tbody"); body.replaceChildren();
    for (const contributor of contributors) {
      const row = document.createElement("tr");
      const kind = { implementation: "Implementation", review: "Review" }[contributor.kind] || contributor.kind || "—";
      for (const [index, value] of [contributor.name || "—", kind, contributor.issue_numbers?.length ? contributor.issue_numbers.map(n => `#${n}`).join(", ") : "—", `${date(contributor.first_posted_at)} – ${date(contributor.last_posted_at)}`, number(contributor.post_count)].entries()) {
        const td = document.createElement("td"); td.dataset.label = ["Agent", "Kind", "Issues", "Period (UTC)", "Posts"][index]; td.textContent = value; row.append(td);
      }
      body.append(row);
    }
    element.querySelector(".roster-past-empty").hidden = contributors.length > 0;
    element.querySelector(".roster-past-empty").textContent = complete === false ? "Previous contributor data is unavailable." : "No previous contributors are recorded.";
  }
  function syncPreview() {
    if (!previewElement) return;
    const clone = element.cloneNode(true); clone.classList.add("agents-surface--preview"); clone.setAttribute("aria-label", "Agents read-only preview");
    for (const node of clone.querySelectorAll("[id]")) node.removeAttribute("id");
    for (const control of clone.querySelectorAll("button, select")) { control.disabled = true; control.tabIndex = -1; }
    for (const row of clone.querySelectorAll(".roster-row")) row.tabIndex = -1;
    previewElement.replaceChildren(clone);
  }
  function setPreviewHost(host) {
    if (previewElement?.parentElement === host) return;
    previewElement?.remove(); previewElement = null;
    if (!host || disposed) return;
    previewElement = document.createElement("div"); previewElement.className = "roster-preview-host"; host.append(previewElement); syncPreview();
  }
  function sync() {
    if (disposed) return;
    const roster = rosterModel.read(), activity = activityModel.read(), catalog = windowCatalogModel.read().catalog;
    const windows = (catalog.appState.tabs || []).filter(tab => !activity.snapshot?.project_key
      || tab.project_key === activity.snapshot.project_key).flatMap(tab => tab.workspace?.windows || []);
    const pm = windows.find(window => (activity.agents || []).some(assignment => assignment.is_pm
      && assignment.window_id === window.id && assignment.session_id === window.session_id));
    const identity = element.querySelector(".roster-pm-identity");
    identity.hidden = !pm;
    identity.querySelector(".roster-pm-name").textContent = pm?.agent_name || (pm ? "Project Manager" : "");
    const schedule = scheduleState();
    if (submittedSnapshot && (schedule.error || schedule.snapshot !== submittedSnapshot)) submittedSnapshot = null;
    const enabledMembers = (roster.members || []).filter(member => member.enabled && !member.is_pm);
    const members = (roster.members || []).filter(member => !member.is_pm && (member.enabled || liveWork(member)));
    const enabledCount = enabledMembers.length;
    if (!members.some(member => member.member_id === selectedId)) selectedId = members[0]?.member_id || null;
    const selected = members.find(member => member.member_id === selectedId);
    element.querySelector(".roster-count").textContent = String(enabledCount);
    element.querySelector(".roster-capacity-value").textContent = number(roster.capacity);
    element.querySelector(".roster-active").textContent = `Active ${number(activity.count)}`;
    const scale = Math.max(enabledCount, roster.recommended_capacity || 0, roster.capacity || 0, 1) * 1.25;
    const bar = element.querySelector(".roster-capacity-bar"); bar.setAttribute("aria-valuemin", "0"); bar.setAttribute("aria-valuemax", String(scale)); bar.setAttribute("aria-valuenow", String(enabledCount));
    element.querySelector(".roster-capacity-fill").style.width = `${enabledCount / scale * 100}%`;
    const limit = element.querySelector(".roster-capacity-limit"); limit.hidden = !finite(roster.recommended_capacity); limit.style.left = `${(roster.recommended_capacity || 0) / scale * 100}%`;
    element.querySelector(".roster-capacity-note").textContent = `Busy ${number(activity.count === null ? null : members.filter(member => liveWork(member)).length)} / Team ${enabledCount} · Recommended capacity ${number(roster.recommended_capacity)}${finite(roster.recommended_capacity) && enabledCount > roster.recommended_capacity ? " · Above recommended capacity" : ""}`;
    element.querySelector('.roster-capacity [data-action="rest"]').disabled = !selected?.enabled || enabledCount <= 1;
    const error = element.querySelector(".roster-error"); error.textContent = roster.error || ""; error.hidden = !roster.error;
    const list = element.querySelector(".roster-list"); list.replaceChildren();
    for (const member of members) {
      const row = document.createElement("article"); row.className = "roster-row"; row.dataset.memberId = member.member_id; row.setAttribute("role", "listitem"); row.tabIndex = 0; row.setAttribute("aria-selected", String(member.member_id === selectedId));
      const current = liveWork(member), unavailable = member.assignment?.session_id && activity.count === null, who = document.createElement("div"); who.className = "roster-who";
      const name = document.createElement("h3"), purpose = document.createElement(current ? "button" : "span"); name.textContent = member.name; purpose.className = "roster-purpose"; purpose.textContent = current ? workTitle(current) : unavailable ? "Activity unavailable" : "Waiting for the next issue"; purpose.title = purpose.textContent;
      if (current) { purpose.type = "button"; purpose.addEventListener("click", event => { event.stopPropagation(); openCurrent(member.member_id, current); }); }
      who.append(name, purpose, stats(member, "roster-row-stats"));
      const state = document.createElement(current ? "button" : "span"); state.className = `roster-state${current ? " roster-current" : ""}`; state.textContent = current ? `${member.enabled ? "In Works" : "Resting · Finishing in Works"} ${current.issue_number ? `#${current.issue_number}` : ""}` : unavailable ? "Activity unavailable" : "Waiting";
      if (current) { state.type = "button"; state.title = "Open current work in Works"; state.addEventListener("click", event => { event.stopPropagation(); openCurrent(member.member_id, current); }); }
      row.append(avatar(member), who, state);
      row.addEventListener("click", event => { if (!event.target.closest("button")) selectMember(member.member_id); });
      row.addEventListener("keydown", event => { if (event.target === row && ["Enter", " "].includes(event.key)) { event.preventDefault(); selectMember(member.member_id); } });
      list.append(row);
    }
    renderDetail(selected); renderPast(roster.past_contributors || [], roster.past_complete);
    for (const button of element.querySelectorAll("[data-tab]")) { const active = button.dataset.tab === tab; button.setAttribute("aria-selected", String(active)); button.tabIndex = active ? 0 : -1; }
    element.querySelector(".roster-team").hidden = tab !== "team"; element.querySelector(".roster-past").hidden = tab !== "past";
    element.dataset.catalogVersion = String(catalog.version);
    renderedRoster = roster; renderedActivity = activityPresentation(activity); renderedCatalog = catalog; renderedSchedule = schedule; syncPreview();
  }
  for (const button of element.querySelectorAll('[data-action="add"]')) button.addEventListener("click", () => onAddMember());
  element.querySelector('.roster-capacity [data-action="rest"]').addEventListener("click", restSelected);
  for (const button of element.querySelectorAll("[data-tab]")) {
    button.addEventListener("click", () => { tab = button.dataset.tab; sync(); });
    button.addEventListener("keydown", event => { if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return; event.preventDefault(); tab = event.key === "Home" ? "team" : event.key === "End" ? "past" : tab === "team" ? "past" : "team"; sync(); element.querySelector(`[data-tab="${tab}"]`).focus(); });
  }
  sync();
  const unsubscribers = [rosterModel.subscribeFrame(state => state, value => { if (value !== renderedRoster) sync(); }), activityModel.subscribeFrame(activityPresentation, value => { if (value !== renderedActivity) sync(); }), windowCatalogModel.subscribeFrame(state => state.catalog, value => { if (value !== renderedCatalog) sync(); })];
  if (scheduleModel) unsubscribers.push(scheduleModel.subscribeFrame(state => state, value => { if (value !== renderedSchedule) { scheduleNotice = ""; sync(); } }));
  return { element, sync, selectMember, setPreviewHost, dispose() { if (disposed) return; for (const unsubscribe of unsubscribers) unsubscribe(); setPreviewHost(null); disposed = true; } };
}
