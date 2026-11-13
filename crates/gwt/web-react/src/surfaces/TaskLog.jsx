import React from "react";
import { useStoreSelector } from "../store/useStoreSelector.js";
import { localTimeLabel, relativeTimeLabel } from "./time-labels.js";

// Task-centered Log surface (SPEC #7007 T-013, US-5). Lists the #5016
// notification history groups exactly as the history model grouped them:
// one row per group with its occurrence count, so a repeated event never
// floods the list and is never re-grouped here by message text (AC-3).
// The formatted row keeps the original record reachable — the raw text
// sits in the details disclosure and the group hands back to the
// notification drawer (AC-4/AC-9).
const selectLog = state => state;

const CATEGORY_LABELS = {
  action: "Action required",
  completed: "Completed",
  information: "Information",
};

export function TaskLog({ store, onViewNotification = () => {} }) {
  const model = useStoreSelector(store, selectLog, { frame: true }) || {};
  const groups = model.groups || [];
  return (
    <section className="task-log" aria-label="Log">
      <div className="task-log__header">
        <div>
          <p className="task-log__eyebrow">Updates grouped by task and event</p>
          <h1>Log</h1>
        </div>
      </div>
      {groups.length === 0 ? (
        <div className="task-log__empty">
          <h2>No updates yet</h2>
          <p>Notifications from agents and surfaces are grouped here as they arrive.</p>
        </div>
      ) : (
        groups.map(entry => (
          <article className="task-log__card" key={entry.id} data-group-id={entry.id}>
            <div className="task-log__card-head">
              <h2>{entry.title}</h2>
              <span className={`task-log__chip task-log__chip--${entry.category}`}>
                {CATEGORY_LABELS[entry.category] || "Information"}
              </span>
            </div>
            {entry.message ? <p className="task-log__message">{entry.message}</p> : null}
            <p className="task-log__meta">
              {entry.count > 1 ? `Repeated ${entry.count} times · ` : ""}
              {"Last updated "}
              <time datetime={entry.latestAt || undefined} title={localTimeLabel(entry.latestAt) || undefined}>
                {relativeTimeLabel(entry.latestAt) || "Unknown"}
              </time>
              {localTimeLabel(entry.latestAt) ? ` (${localTimeLabel(entry.latestAt)})` : ""}
            </p>
            {entry.details ? (
              <details className="task-log__details">
                <summary>Original record</summary>
                <p className="task-log__detail">{entry.details}</p>
              </details>
            ) : null}
            <div className="task-log__actions">
              <button type="button" onClick={() => onViewNotification(entry)}>View in Notifications</button>
            </div>
          </article>
        ))
      )}
    </section>
  );
}
