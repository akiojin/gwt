import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { focusElement } from "../hosts/focus.js";

// Issue #7728: the one project stop/start (design items 56, 179 and 191).
// Stopping halts Issues assignment and the Project Manager patrol together;
// agents already working finish their current work (Stop pending) and the
// project then settles as Stopped. There is no cancel: Start resumes it once
// it is Stopped. The Hub project list and Settings mount this same surface,
// so both show the same confirmation and send the same operation.

const COPY = {
  unknown: "Unknown",
  running: "Running",
  partial: "Partly running",
  stopped: "Stopped",
  stopPending: agents => `Stop pending · ${agents} ${agents === 1 ? "agent" : "agents"} finishing`,
  stop: "Stop…",
  start: "Start",
  dialogTitle: "Stop this project?",
  dialogBody: "New assignments and the Project Manager patrol stop now. Agents that are already working finish their current work first (Stop pending), then the project is Stopped. A stop cannot be cancelled; start the project again once it is Stopped.",
  cancel: "Cancel",
  confirm: "Stop project",
};

// `autonomy` carries the runtime inputs: { monitor_enabled, pm_paused, active_agents }.
export function projectAutonomyState(autonomy) {
  if (!autonomy || typeof autonomy.monitor_enabled !== "boolean" || typeof autonomy.pm_paused !== "boolean") {
    return { state: "unknown", label: COPY.unknown };
  }
  if (autonomy.monitor_enabled && !autonomy.pm_paused) return { state: "running", label: COPY.running };
  // Only one half runs (an older saved state). Start brings both up, Stop both down.
  if (autonomy.monitor_enabled || !autonomy.pm_paused) return { state: "partial", label: COPY.partial };
  const agents = Number.isSafeInteger(autonomy.active_agents) ? autonomy.active_agents : 0;
  if (agents > 0) return { state: "stop_pending", label: COPY.stopPending(agents) };
  return { state: "stopped", label: COPY.stopped };
}

export function ProjectAutonomy({ autonomy, projectTitle = "", onSetRunning }) {
  const view = projectAutonomyState(autonomy);
  const rootRef = useRef(null);
  const actionRef = useRef(null);
  const dialogRef = useRef(null);
  const titleId = useId();
  const bodyId = useId();
  const [confirming, setConfirming] = useState(false);
  // The dialog is drawn on the page body, which is known once this is mounted.
  const [body, setBody] = useState(null);
  useEffect(() => setBody(rootRef.current.ownerDocument.body), []);
  const stoppable = view.state === "running" || view.state === "partial";
  const dialogOpen = confirming && stoppable && Boolean(body);

  useEffect(() => {
    if (!dialogOpen) return undefined;
    focusElement(dialogRef.current?.querySelector("[data-project-stop-cancel]"));
    const onKeyDown = event => {
      if (event.key === "Escape") setConfirming(false);
    };
    const document = dialogRef.current.ownerDocument;
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      focusElement(actionRef.current);
    };
  }, [dialogOpen]);

  // Stop pending is one-way (design item 179): no action until Stopped.
  const actions = [];
  if (view.state === "stopped" || view.state === "partial") actions.push({ kind: "start", label: COPY.start, run: () => onSetRunning?.(true) });
  if (stoppable) actions.push({ kind: "stop", label: COPY.stop, run: () => setConfirming(true) });

  return (
    <span className="project-autonomy" ref={rootRef} data-state={view.state}>
      <span className="project-autonomy__state" role="status" data-project-autonomy-state data-state={view.state}>{view.label}</span>
      {actions.map(action => (
        <button type="button" className="text-button" key={action.kind} ref={action.kind === "stop" ? actionRef : undefined}
          data-project-autonomy-action={action.kind} onClick={action.run}>
          {action.label}
        </button>
      ))}
      {dialogOpen ? createPortal(
        <div className="modal-backdrop open" data-project-stop-dialog onClick={event => { if (event.target === event.currentTarget) setConfirming(false); }}>
          <div className="modal-shell" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={bodyId} ref={dialogRef}>
            <div className="modal-header"><h2 id={titleId}>{COPY.dialogTitle}</h2></div>
            <div className="modal-body">
              {projectTitle ? <p className="project-stop-subject">{projectTitle}</p> : null}
              <p id={bodyId}>{COPY.dialogBody}</p>
            </div>
            <div className="modal-footer">
              <button type="button" className="text-button" data-project-stop-cancel onClick={() => setConfirming(false)}>{COPY.cancel}</button>
              <button type="button" className="wizard-button destructive" data-project-stop-confirm
                onClick={() => { setConfirming(false); onSetRunning?.(false); }}>
                {COPY.confirm}
              </button>
            </div>
          </div>
        </div>,
        body,
      ) : null}
    </span>
  );
}
