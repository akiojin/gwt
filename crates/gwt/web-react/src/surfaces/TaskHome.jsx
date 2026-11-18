import { useStoreSelector } from "../store/useStoreSelector.js";

// Task-centered entry surface (SPEC #7007, US-1/US-2). Reads only through the
// #5016 UI state store: the host feeds the PM window state (request-task
// overview, pending decision reports, completed-today count) into the store,
// push payloads never reach this component directly.
const selectHome = state => state;

const STATUS_LABELS = {
  pending: "Queued",
  in_progress: "In progress",
  waiting: "Waiting",
  completed: "Completed",
  cancelled: "Cancelled",
};

function summaryFor(home) {
  const { overview, decisionsWaiting, stale } = home;
  if (decisionsWaiting == null || overview == null) {
    return {
      title: "Unable to refresh your project status",
      description: "We could not load the latest status. Try again to see what needs your attention.",
      action: null,
    };
  }
  const prefix = stale ? "Showing the last confirmed summary. " : "";
  if (decisionsWaiting > 0) {
    return {
      title: `${prefix}${decisionsWaiting} decision${decisionsWaiting === 1 ? "" : "s"} need${decisionsWaiting === 1 ? "s" : ""} your attention`,
      description: "Review what is waiting so those tasks can continue.",
      action: "decide",
    };
  }
  return {
    title: `${prefix}No decisions waiting`,
    description: "Nothing needs your input right now. You can start a new task or check progress.",
    action: null,
  };
}

function countsFor(home) {
  const { overview, completedToday } = home;
  if (!overview || overview.state !== "ready") return null;
  const tasks = overview.tasks || [];
  const inProgress = tasks.filter(task => task.status === "in_progress" || task.status === "waiting").length;
  const queued = tasks.filter(task => task.status === "pending").length + (overview.pending_intake || 0);
  return { inProgress, queued, completedToday };
}

export function TaskHome({ store, onCreateTask = () => {}, onReviewDecisions = () => {}, onViewProgress = () => {} }) {
  const home = useStoreSelector(store, selectHome, { frame: true }) || {};
  const { overview, stale } = home;
  const summary = summaryFor(home);
  const counts = countsFor(home);
  const tasks = overview?.state === "ready" ? overview.tasks || [] : [];
  const recent = tasks.filter(task => task.status !== "completed" && task.status !== "cancelled").slice(0, 5);

  return (
    <section className="task-home" aria-label="Home">
      <div className="task-home__header">
        <div>
          <p className="task-home__eyebrow">Your project at a glance</p>
          <h1>Home</h1>
        </div>
        <button type="button" className="task-home__primary" onClick={onCreateTask}>+ Create task</button>
      </div>

      <div className="task-home__hero">
        <p className="task-home__eyebrow">Your next step</p>
        <h2>{summary.title}</h2>
        <p>{summary.description}</p>
        {summary.action === "decide" ? (
          <button type="button" className="task-home__primary" onClick={onReviewDecisions}>
            {`Review ${home.decisionsWaiting} decision${home.decisionsWaiting === 1 ? "" : "s"} →`}
          </button>
        ) : null}
      </div>

      {counts ? (
        <div className="task-home__counts">
          <div><strong>{counts.inProgress}</strong> Tasks in progress</div>
          <div><strong>{counts.queued}</strong> Tasks queued</div>
          <div><strong>{counts.completedToday ?? "Unknown"}</strong> Completed today</div>
        </div>
      ) : null}

      {overview?.state === "absent" ? <p>No request tasks have been recorded yet.</p> : null}
      {stale && overview ? <p className="task-home__stale">Showing the last confirmed summary — the latest refresh failed.</p> : null}

      {recent.length ? <h2 className="task-home__section">Recent progress</h2> : null}
      {recent.map(task => {
        const needsDecision = Boolean(task.review_required) || task.status === "waiting";
        const stage = task.status === "waiting"
          ? (task.wait_reason || "Waiting")
          : (task.next_action || STATUS_LABELS[task.status] || "In progress");
        return (
          <article key={task.key} className="task-home__card">
            <div className="task-home__card-top">
              <h3>{task.title}</h3>
              <span className={needsDecision ? "task-home__badge task-home__badge--attention" : "task-home__badge"}>
                {needsDecision ? "Needs your decision" : (STATUS_LABELS[task.status] || task.status)}
              </span>
            </div>
            <p>{`${task.assignee || "Unassigned"} · ${stage}`}</p>
            {needsDecision ? (
              <div className="task-home__actions">
                <button type="button" onClick={onReviewDecisions}>Review decision →</button>
              </div>
            ) : (
              <div className="task-home__actions">
                <button type="button" onClick={onViewProgress}>View progress →</button>
              </div>
            )}
          </article>
        );
      })}
    </section>
  );
}
