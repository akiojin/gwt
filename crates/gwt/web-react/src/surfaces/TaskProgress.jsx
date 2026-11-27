import React from "react";
import { useStoreSelector } from "../store/useStoreSelector.js";
import { TaskCard } from "../components/TaskCard.jsx";
import { relativeTimeLabel } from "./time-labels.js";

// Task-centered Progress surface (SPEC #7007 T-012, US-4). The subject is
// the request-task overview from the PM window snapshot: headline counts
// come from the snapshot totals, never from the number of rows shown
// (AC-6). Agent windows the PM reports as live facts are NOT request
// tasks — they render in a separate frame and are never merged into a
// task (the frontend has no join key between a window and a task).
const selectProgress = state => state;

const EMPTY_MODEL = Object.freeze({ loaded: false, overview: null, error: null, activeWindows: null });

export function TaskProgress({
  store,
  onCreateTask = () => {},
  onReviewDecision = () => {},
  onViewTaskDetails = () => {},
  onViewAllTasks = () => {},
}) {
  const model = useStoreSelector(store, selectProgress, { frame: true }) || EMPTY_MODEL;
  const { loaded, overview, error } = model;
  const ready = overview?.state === "ready";
  const tasks = ready ? overview.tasks || [] : [];
  const parents = new Map(tasks.map(task => [task.key, task.title]));
  const active = tasks.filter(task => task.status === "in_progress" || task.status === "waiting");
  const queued = tasks.filter(task => task.status === "pending");
  const finished = tasks.filter(task => task.status === "completed" || task.status === "cancelled");
  const previewLimited = ready && overview.total > tasks.length;
  const activeWindows = model.activeWindows || [];
  const cardFor = task => (
    <TaskCard
      key={task.key}
      task={task}
      parentTitle={task.parent_key ? parents.get(task.parent_key) || null : null}
      onReviewDecision={onReviewDecision}
      onViewTaskDetails={onViewTaskDetails}
    />
  );
  return (
    <section className="task-progress" aria-label="Progress">
      <div className="task-progress__header">
        <div>
          <p className="task-progress__eyebrow">Counts include all tasks</p>
          <h1>Progress</h1>
        </div>
        <button type="button" className="task-progress__primary" onClick={onCreateTask}>+ Create task</button>
      </div>

      {!loaded || (!overview && !error) ? <p role="status">Loading progress…</p> : null}

      {ready ? (
        <>
          <p className="task-progress__summary" role="status">
            {`${overview.total} task${overview.total === 1 ? "" : "s"} · ${overview.pending_intake || 0} awaiting classification`}
            {previewLimited ? `. Showing ${tasks.length} of ${overview.total} tasks (unfinished first).` : ""}
          </p>
          {error ? (
            <p className="task-progress__error" role="alert">{`${error} Showing the last confirmed progress.`}</p>
          ) : null}
          {active.map(cardFor)}
          {queued.length ? <h2 className="task-progress__section">Queued</h2> : null}
          {queued.map(cardFor)}
          {finished.length ? (
            <p className="task-progress__meta">
              {`${finished.length} completed or cancelled task${finished.length === 1 ? " is" : "s are"} kept in the full task list.`}
            </p>
          ) : null}
          {previewLimited || finished.length ? (
            <div className="task-progress__actions">
              <button type="button" onClick={onViewAllTasks}>View all tasks</button>
            </div>
          ) : null}
        </>
      ) : null}

      {overview?.state === "absent" ? (
        <div className="task-progress__empty">
          <h2>No request tasks yet</h2>
          <p>No request tasks have been recorded yet. Create a task from Home or Chat and its progress will appear here.</p>
        </div>
      ) : null}

      {!overview && error ? (
        <div className="task-progress__empty">
          <h2>Unable to refresh progress</h2>
          <p>{error}</p>
          <p>Progress refreshes automatically while this window is open. If the problem continues, reopen the PM window.</p>
        </div>
      ) : null}

      {activeWindows.length ? (
        <section className="task-progress__other" aria-label="Other active work">
          <h2>Other active work</h2>
          <p className="task-progress__meta">
            Agent windows the PM reports as active right now. These live facts are separate from the request tasks above and are not merged into them.
          </p>
          <ul>
            {activeWindows.map((item, index) => (
              <li key={`${item.issueNumber ?? "unassigned"}-${index}`}>
                {`${item.issueNumber ? `#${item.issueNumber}` : "Unassigned issue"} ${item.title || "Unknown title"} — Last activity `}
                <time datetime={item.lastActivityAt || undefined}>{relativeTimeLabel(item.lastActivityAt) || "Unknown"}</time>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </section>
  );
}
