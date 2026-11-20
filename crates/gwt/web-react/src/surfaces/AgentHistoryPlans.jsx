import { useStoreSelector } from "../store/useStoreSelector.js";

// Issue #7649 (SPEC #7007 T-031, design items 25 and 169): the project's
// agent history and planned assignments. History lists every member's
// sessions in this project, the Project Manager's included. Planned
// assignments are the assignment plan itself, so the Project Manager, who
// holds no issue, never appears there. The agent ledger shows neither.
const selectModel = state => state;

const RESULT_LABELS = { WaitingInput: "Waiting for input" };
const REASON_LABELS = {
  same_spec_continuation: "Same SPEC continuation",
  queue_order: "Queue order",
  assigned_by_you: "Assigned by you",
};
const TOOL_DEFAULT = "Tool default";

const utcMinute = value => value && Number.isFinite(Date.parse(value))
  ? new Date(value).toISOString().replace("T", " ").slice(0, 16) : "Unknown";

function issueLabel(item) {
  const title = item.title ? ` ${item.title}` : "";
  if (item.issue_number) return `#${item.issue_number}${title}`;
  if (item.role === "pm") return `Project Manager${item.title ? ` — ${item.title}` : ""}`;
  return item.title || "No linked issue";
}

// One row per recorded session, newest first. A fact the session did not
// record is named instead of left blank: the model falls back to the
// execution environment's own, then to the tool's default.
export function historyRows(members = []) {
  return members.flatMap(member => (member.history || []).map(item => ({
    key: item.session_id,
    agent: member.name || item.name || "Unknown agent",
    pm: item.role === "pm",
    issue: issueLabel(item),
    environment: item.environment
      || (item.provider ? `${item.provider} (not registered as an execution environment)` : "Unknown"),
    model: item.model || item.environment_model || TOOL_DEFAULT,
    reasoning: item.reasoning || TOOL_DEFAULT,
    result: RESULT_LABELS[item.result] || item.result || "Unknown",
    startedAt: item.started_at,
  }))).sort((left, right) => (Date.parse(right.startedAt) || 0) - (Date.parse(left.startedAt) || 0));
}

// The assignment plan as it was read: members in ledger order, each one's
// issues in plan order. `status` says why no plan is shown.
export function plannedAssignments({ members = [], pmMemberIds = [], rosterRevision = null, schedule = {} } = {}) {
  const snapshot = schedule?.snapshot;
  if (schedule?.error) return { status: schedule.error, rows: [] };
  if (!snapshot) return { status: "Plans unavailable. Refresh plans to try again.", rows: [] };
  if (snapshot.roster_revision !== rosterRevision) {
    return { status: "Members changed. Refresh plans to see the current plan.", rows: [] };
  }
  const rows = members.filter(member => !pmMemberIds.includes(member.member_id)).flatMap(member =>
    (snapshot.members.find(plan => plan.agent_member_id === member.member_id)?.issues || []).map(issue => ({
      key: `${member.member_id}:${issue.issue_number}`,
      memberId: member.member_id,
      agent: member.name,
      issueNumber: issue.issue_number,
      reason: REASON_LABELS[issue.reason] || "Reason unavailable",
    })));
  return { status: "", rows };
}

export function AgentHistoryPlans({ store }) {
  const model = useStoreSelector(store, selectModel) || {};
  const history = historyRows(model.members);
  const plan = plannedAssignments(model);
  return (
    <section className="agent-history-plans" aria-label="Agent history and plans">
      <section className="agent-history">
        <h3>Agent history</h3>
        <p className="roster-description">
          Every session in this project, newest first, with the execution environment, model and reasoning it started with.
        </p>
        {history.length === 0 ? (
          <p className="agent-history__empty">No sessions are recorded for this project yet.</p>
        ) : (
          <div className="agent-history__scroll">
            <table aria-label="Agent history">
              <thead>
                <tr>
                  <th scope="col">Agent</th>
                  <th scope="col">Issue</th>
                  <th scope="col">Execution environment</th>
                  <th scope="col">Model</th>
                  <th scope="col">Reasoning</th>
                  <th scope="col">Result</th>
                  <th scope="col">Started (UTC)</th>
                </tr>
              </thead>
              <tbody>
                {history.map(row => (
                  <tr key={row.key} data-session-id={row.key} data-role={row.pm ? "pm" : undefined}>
                    <td>
                      {row.agent}
                      {row.pm ? <span className="window-role-badge">PM</span> : null}
                    </td>
                    <td>{row.issue}</td>
                    <td>{row.environment}</td>
                    <td>{row.model}</td>
                    <td>{row.reasoning}</td>
                    <td>{row.result}</td>
                    <td>{utcMinute(row.startedAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
      <section className="agent-plans">
        <h3>Planned assignments</h3>
        <p className="roster-description">The issue each agent starts next, in the order of the assignment plan.</p>
        <p className="roster-plan-status" role="status" hidden={!plan.status}>{plan.status}</p>
        {plan.status ? null : plan.rows.length === 0 ? (
          <p className="agent-plans__empty">No planned assignments.</p>
        ) : (
          <ol aria-label="Planned assignments">
            {plan.rows.map(row => (
              <li key={row.key} data-member-id={row.memberId} data-issue-number={row.issueNumber}>
                <strong>{`${row.agent} → #${row.issueNumber}`}</strong>
                <span className="roster-plan-reason">{row.reason}</span>
                <span className="agent-plans__badge">Planned</span>
              </li>
            ))}
          </ol>
        )}
      </section>
    </section>
  );
}
