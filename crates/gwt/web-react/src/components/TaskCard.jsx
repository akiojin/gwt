import React from "react";

// Purpose-first task card (SPEC #7007 T-012, US-4). The primary text is
// the task purpose, its Agent and its current stage — for a waiting task
// the stage is the blocker. Internal references (the task key, criteria
// previews, parentage) live in the details disclosure, never in the
// primary text (AC-7). Metrics the snapshot does not carry are not
// rendered at all (AC-8).
export const TASK_STATUS_LABELS = {
  pending: "Queued",
  in_progress: "In progress",
  waiting: "Waiting",
  completed: "Completed",
  cancelled: "Cancelled",
};

export function TaskCard({
  task,
  parentTitle = null,
  onReviewDecision = () => {},
  onViewTaskDetails = () => {},
}) {
  const needsDecision = Boolean(task.review_required) || task.status === "waiting";
  const assignee = task.assignee || "Unassigned";
  const stage = task.wait_reason
    ? `Waiting — ${task.wait_reason}`
    : task.next_action || TASK_STATUS_LABELS[task.status] || "In progress";
  const criteria = task.criteria_preview;
  const child = task.child_progress;
  return (
    <article className="task-card" data-task-key={task.key}>
      <div className="task-card__head">
        <h3>{task.title}</h3>
        <span className={needsDecision ? "task-card__chip task-card__chip--attention" : "task-card__chip"}>
          {needsDecision ? "Needs your decision" : TASK_STATUS_LABELS[task.status] || task.status}
        </span>
      </div>
      <p className="task-card__line">{`${assignee} · ${stage}`}</p>
      {task.review_required && task.status !== "waiting" ? (
        <p className="task-card__review">PM reassessment required</p>
      ) : null}
      {child ? (
        <p className="task-card__subtasks">
          {`Subtasks: ${child.completed} of ${child.total} completed${child.cancelled ? ` · ${child.cancelled} cancelled` : ""}`}
        </p>
      ) : null}
      <div className="task-card__actions">
        {needsDecision ? (
          <button type="button" onClick={() => onReviewDecision(task)}>Review decision</button>
        ) : null}
        <button type="button" onClick={() => onViewTaskDetails(task.key)}>View task details</button>
      </div>
      <details className="task-card__details">
        <summary>Task details</summary>
        {criteria ? (
          <>
            <p>{`Purpose: ${criteria.purpose}`}</p>
            {criteria.completion_conditions?.length ? (
              <ol>
                {criteria.completion_conditions.map((condition, index) => (
                  <li key={index}>{condition}</li>
                ))}
              </ol>
            ) : null}
            {criteria.completion_condition_total > (criteria.completion_conditions?.length || 0) ? (
              <p className="task-card__meta">
                {`Showing ${criteria.completion_conditions.length} of ${criteria.completion_condition_total} completion criteria.`}
              </p>
            ) : null}
          </>
        ) : null}
        {task.parent_key ? (
          <p className="task-card__meta">{`Part of: ${parentTitle || "a task outside this preview"}`}</p>
        ) : null}
        <p className="task-card__meta">{`Reference: ${task.key}`}</p>
      </details>
    </article>
  );
}
