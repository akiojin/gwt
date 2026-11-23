import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useStoreSelector } from "../store/useStoreSelector.js";
import { focusElement } from "../hosts/focus.js";

// Themes: the screen of a project that groups Issues (Issue #7643, SPEC #7007
// T-025). Theme list → theme detail → the issues under it. The store carries
// the view the host derived; every row is one click target that opens the
// next level (design item 126).
const selectView = state => state;

const plural = (count, word) => `${count} ${word}${count === 1 ? "" : "s"}`;

// Every string this surface shows.
const COPY = {
  title: "Themes",
  lead: "A theme groups several issues. The PM records a theme from what you ask for or decide in the conversation on Home. Each theme is a GitHub Issue, and its progress comes from GitHub.",
  stats: { in_progress: "Themes in progress", almost_done: "Themes almost done", completedToday: "Issues completed today" },
  status: { in_progress: "In progress", almost_done: "Almost done" },
  loading: "Loading themes…",
  refresh: "Refresh",
  retry: "Try again",
  truncated: "This repository has more open issues than one read covers. Only themes among the most recently updated issues are listed.",
  deleted: number => `Theme #${number} was deleted.`,
  emptyTitle: "No themes yet",
  emptyBody: "The PM records a theme when you ask for something or decide something in the conversation on Home. A GitHub Issue that has sub-issues also appears here.",
  progress: (total, completed) => `${plural(total, "issue")} under it · ${completed} completed`,
  back: "Back to themes",
  kind: total => `Theme · ${plural(total, "issue")} under it`,
  github: number => `This theme is a GitHub Issue: #${number}`,
  openGithub: "Open on GitHub",
  about: "About this theme",
  origin: "Where it came from",
  recorded: at => `Recorded automatically from the conversation on Home${at ? ` on ${formatDate(at)}` : ""}. What was asked or decided later is added to the GitHub Issue.`,
  openConversation: "Show in conversation",
  noSummary: "This theme has no description.",
  progressTitle: "Progress",
  issues: "Issues under this theme",
  issuesPartial: (shown, total) => `Showing the first ${shown} of ${total} issues. Open the theme on GitHub to see the rest.`,
  noIssues: "No issues sit under this theme yet. The PM adds them as the work is planned.",
  issueOpen: "Open",
  issueCompleted: "Completed",
  otherRepository: "In another repository — opens on GitHub",
  actions: "Theme actions",
  deleteTitle: "Delete this theme",
  deleteLead: "Deleting is the only way to correct a theme that was recorded by mistake. It closes the theme's GitHub Issue and keeps the issues under it. Themes cannot be merged, and issues cannot be moved between themes.",
  deleteButton: "Delete theme",
  dialogTitle: "Delete this theme?",
  dialogBody: (title, number) => `"${title}" leaves this list: GitHub Issue #${number} is closed on GitHub as not planned.`,
  dialogImpact: total => `${total === 1 ? "The 1 issue" : `The ${total} issues`} under it ${total === 1 ? "stays as it is" : "stay as they are"}.`,
  dialogUndo: "Nothing is erased. Reopen the Issue on GitHub to bring the theme back.",
  deleting: "Deleting…",
  cancel: "Cancel",
};

// The day a theme was recorded, in the viewer's locale; an unreadable time
// shows as it was given.
function formatDate(value) {
  const at = new Date(value);
  return Number.isNaN(at.getTime()) ? value : at.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

function Progress({ theme }) {
  return (
    <div
      className="project-themes__bar"
      role="progressbar"
      aria-label={COPY.progress(theme.total, theme.completed)}
      aria-valuemin={0}
      aria-valuemax={theme.total}
      aria-valuenow={theme.completed}
    >
      <i style={{ width: `${theme.percent}%` }} />
    </div>
  );
}

function Badge({ status }) {
  return <span className="project-themes__badge" data-status={status}>{COPY.status[status]}</span>;
}

function ThemeList({ view, actions, headingRef }) {
  const count = status => view.themes.filter(theme => theme.status === status).length;
  return (
    <>
      <header className="project-themes__header">
        <h1 tabIndex={-1} ref={headingRef}>{COPY.title}</h1>
        <p className="project-themes__lead">{COPY.lead}</p>
      </header>
      {view.loaded ? (
        <dl className="project-themes__stats">
          <div><dd data-themes-stat="in_progress">{count("in_progress")}</dd><dt>{COPY.stats.in_progress}</dt></div>
          <div><dd data-themes-stat="almost_done">{count("almost_done")}</dd><dt>{COPY.stats.almost_done}</dt></div>
          <div><dd data-themes-stat="completed_today">{view.completedToday}</dd><dt>{COPY.stats.completedToday}</dt></div>
        </dl>
      ) : null}
      <p className="project-themes__status" aria-live="polite">
        {view.loading ? COPY.loading : view.notice ? COPY.deleted(view.notice.deleted) : ""}
      </p>
      {view.error ? <p className="project-themes__error" role="alert">{view.error}</p> : null}
      {view.loaded && view.themes.length ? (
        <ul className="project-themes__list" data-themes-list>
          {view.themes.map(theme => (
            <li key={theme.number}>
              <button type="button" className="project-themes__row" data-theme-number={theme.number} onClick={() => actions.openTheme(theme.number)}>
                <span className="project-themes__row-text">
                  <span className="project-themes__row-title">{`${theme.title} (#${theme.number})`}</span>
                  <span className="project-themes__meta">{COPY.progress(theme.total, theme.completed)}</span>
                  <Progress theme={theme} />
                </span>
                <Badge status={theme.status} />
                <span className="project-themes__chevron" aria-hidden="true">›</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      {view.loaded && !view.themes.length ? (
        <div className="project-themes__empty" data-themes-empty>
          <h2>{COPY.emptyTitle}</h2>
          <p>{COPY.emptyBody}</p>
        </div>
      ) : null}
      {view.truncated ? <p className="project-themes__hint">{COPY.truncated}</p> : null}
      <div className="project-themes__actions">
        <button type="button" data-themes-refresh disabled={view.loading} onClick={actions.reload}>
          {view.error ? COPY.retry : COPY.refresh}
        </button>
      </div>
    </>
  );
}

function ThemeDetail({ view, theme, actions, headingRef, deleteRef }) {
  return (
    <>
      <header className="project-themes__header">
        <button type="button" className="project-themes__back" data-themes-back onClick={actions.closeTheme}>{`← ${COPY.back}`}</button>
        <h1 tabIndex={-1} ref={headingRef}>{`${theme.title} (#${theme.number})`}</h1>
        <p className="project-themes__lead">{COPY.kind(theme.total)}</p>
        <p className="project-themes__meta">
          {COPY.github(theme.number)}{" "}
          <a href={theme.url} target="_blank" rel="noopener noreferrer">{COPY.openGithub}</a>
        </p>
      </header>
      <section className="project-themes__group">
        <h2>{COPY.about}</h2>
        <p>{theme.summary || COPY.noSummary}</p>
        <div className="project-themes__progress">
          <div className="project-themes__row-text">
            <span className="project-themes__row-title">{COPY.progressTitle}</span>
            <span className="project-themes__meta">{COPY.progress(theme.total, theme.completed)}</span>
            <Progress theme={theme} />
          </div>
          <Badge status={theme.status} />
        </div>
      </section>
      {theme.recordedFrom ? (
        <section className="project-themes__group" data-theme-origin>
          <h2>{COPY.origin}</h2>
          <div className="project-themes__progress">
            <div className="project-themes__row-text">
              <span className="project-themes__meta">{COPY.recorded(theme.recordedFrom.recordedAt)}</span>
            </div>
            <button type="button" data-theme-conversation onClick={() => actions.openConversation(theme.recordedFrom)}>
              {COPY.openConversation}
            </button>
          </div>
        </section>
      ) : null}
      <section className="project-themes__group">
        <h2>{COPY.issues}</h2>
        <ul className="project-themes__list" data-theme-issues>
          {theme.issues.map(issue => {
            const text = (
              <>
                <span className="project-themes__row-text">
                  <span className="project-themes__row-title">{`${issue.title} (#${issue.number})`}</span>
                  {issue.inRepository ? null : <span className="project-themes__meta">{COPY.otherRepository}</span>}
                </span>
                <span className="project-themes__badge" data-status={issue.completed ? "completed" : "open"}>
                  {issue.completed ? COPY.issueCompleted : COPY.issueOpen}
                </span>
                <span className="project-themes__chevron" aria-hidden="true">›</span>
              </>
            );
            return (
              <li key={issue.url || issue.number}>
                {issue.inRepository ? (
                  <button type="button" className="project-themes__row" data-theme-issue={issue.number} onClick={() => actions.openIssue(issue.number)}>{text}</button>
                ) : (
                  <a className="project-themes__row" data-theme-issue={issue.number} href={issue.url} target="_blank" rel="noopener noreferrer">{text}</a>
                )}
              </li>
            );
          })}
        </ul>
        {theme.issues.length ? null : <p className="project-themes__hint" data-theme-no-issues>{COPY.noIssues}</p>}
        {theme.total > theme.issues.length ? <p className="project-themes__hint">{COPY.issuesPartial(theme.issues.length, theme.total)}</p> : null}
      </section>
      <section className="project-themes__group">
        <h2>{COPY.actions}</h2>
        <div className="project-themes__progress">
          <div className="project-themes__row-text">
            <span className="project-themes__row-title">{COPY.deleteTitle}</span>
            <span className="project-themes__meta">{COPY.deleteLead}</span>
          </div>
          <button type="button" className="project-themes__danger" data-theme-delete ref={deleteRef} onClick={() => actions.askDelete(theme.number)}>
            {COPY.deleteButton}
          </button>
        </div>
      </section>
      {view.truncated ? <p className="project-themes__hint">{COPY.truncated}</p> : null}
    </>
  );
}

export function ProjectThemes({ store, actions }) {
  const view = useStoreSelector(store, selectView);
  const rootRef = useRef(null);
  const headingRef = useRef(null);
  const deleteRef = useRef(null);
  const dialogRef = useRef(null);
  const theme = view.themes.find(candidate => candidate.number === view.selected) ?? null;
  const dialogTheme = view.dialog ? view.themes.find(candidate => candidate.number === view.dialog.number) ?? null : null;
  // The dialog is drawn on the page body, which is known once this is mounted.
  const [body, setBody] = useState(null);
  useEffect(() => setBody(rootRef.current.ownerDocument.body), []);
  const dialogOpen = Boolean(dialogTheme && body);

  // Moving between the list and a theme puts the reader at the new heading.
  const moved = useRef(false);
  useEffect(() => {
    if (moved.current) focusElement(headingRef.current);
    moved.current = true;
  }, [view.selected]);

  useEffect(() => {
    if (!dialogOpen) return undefined;
    focusElement(dialogRef.current?.querySelector("[data-theme-delete-cancel]"));
    const onKeyDown = event => {
      if (event.key === "Escape") actions.cancelDelete();
    };
    const document = dialogRef.current.ownerDocument;
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      focusElement(deleteRef.current);
    };
  }, [dialogOpen]);

  return (
    <section className="project-themes" aria-label={COPY.title} ref={rootRef} data-themes-view={theme ? "theme" : "themes"}>
      {theme
        ? <ThemeDetail view={view} theme={theme} actions={actions} headingRef={headingRef} deleteRef={deleteRef} />
        : <ThemeList view={view} actions={actions} headingRef={headingRef} />}
      {dialogOpen ? createPortal(
        <div className="modal-backdrop open" data-theme-delete-dialog>
          <div className="modal-shell" role="dialog" aria-modal="true" aria-labelledby="project-themes-delete-title" ref={dialogRef}>
            <div className="modal-header"><h2 id="project-themes-delete-title">{COPY.dialogTitle}</h2></div>
            <div className="modal-body">
              <p>{COPY.dialogBody(dialogTheme.title, dialogTheme.number)}</p>
              <p data-theme-delete-impact>{COPY.dialogImpact(dialogTheme.total)}</p>
              <p data-theme-delete-undo>{COPY.dialogUndo}</p>
              {view.deleteError ? <p className="project-themes__error" role="alert">{view.deleteError}</p> : null}
            </div>
            <div className="modal-footer">
              <button type="button" className="wizard-button" data-theme-delete-cancel disabled={view.deleting} onClick={actions.cancelDelete}>{COPY.cancel}</button>
              <button type="button" className="wizard-button destructive" data-theme-delete-confirm disabled={view.deleting} onClick={actions.confirmDelete}>
                {view.deleting ? COPY.deleting : COPY.deleteButton}
              </button>
            </div>
          </div>
        </div>,
        body,
      ) : null}
    </section>
  );
}
