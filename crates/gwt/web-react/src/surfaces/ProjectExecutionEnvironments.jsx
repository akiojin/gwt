import { useState } from "react";
import { useStoreSelector } from "../store/useStoreSelector.js";

// Issue #7741 (SPEC #7007 item 82): a project's "Available execution
// environments" list. The order is the priority: a session starts on the
// first environment that can be used, skipping one that is held by a limit
// or signed out. The list is saved with the project's Issues settings.
const selectModel = state => state;

const REMOVE_LAST_REASON = "Keep at least one. A project with no execution environment cannot start a session.";

function entryName(entry) {
  return entry.name || `${entry.tool_name} (not registered as an execution environment)`;
}

// Environments the project can still add, each with the reason it cannot be
// added when its tool is already on the list.
export function addableEnvironments(view) {
  const entries = view?.project?.entries || [];
  const listed = new Set(entries.map(entry => entry.environment_id));
  return (view?.environments || [])
    .filter(environment => !listed.has(environment.id))
    .map(environment => {
      const taken = entries.find(entry => entry.tool === environment.via_tool);
      return {
        id: environment.id,
        name: environment.name,
        blockedReason: taken ? `Already uses ${environment.via_tool_name} through "${entryName(taken)}"` : null,
      };
    });
}

function Row({ entry, index, ids, onSave }) {
  const last = index === ids.length - 1;
  const only = ids.length === 1;
  const move = offset => () => {
    const next = [...ids];
    [next[index], next[index + offset]] = [next[index + offset], next[index]];
    onSave(next);
  };
  const name = entryName(entry);
  const reasonId = `project-environment-reason-${index}`;
  return (
    <li className="project-environments__item" data-environment-id={entry.environment_id}>
      <div className="project-environments__text">
        <span className="project-environments__name">{`${index + 1}. ${name}`}</span>
        <span className="settings-help">{`Via ${entry.tool_name}`}</span>
      </div>
      <div className="project-environments__actions">
        <button type="button" className="wizard-button" aria-label={`Move up ${name}`} disabled={index === 0}
          title={index === 0 ? "Already first" : undefined} onClick={move(-1)}>Move up</button>
        <button type="button" className="wizard-button" aria-label={`Move down ${name}`} disabled={last}
          title={last ? "Already last" : undefined} onClick={move(1)}>Move down</button>
        <button type="button" className="wizard-button" aria-label={`Remove ${name}`} disabled={only}
          aria-describedby={only ? reasonId : undefined}
          onClick={() => onSave(ids.filter(id => id !== entry.environment_id))}>Remove</button>
      </div>
      {only ? <p className="settings-help project-environments__reason" id={reasonId}>{REMOVE_LAST_REASON}</p> : null}
    </li>
  );
}

function Add({ view, ids, onSave }) {
  const addable = addableEnvironments(view);
  const first = addable.find(candidate => !candidate.blockedReason) || null;
  const [picked, setPicked] = useState(null);
  // A pick that left the list, or can no longer be added, falls back to the first choice.
  const chosen = addable.some(candidate => candidate.id === picked && !candidate.blockedReason) ? picked : first?.id;
  const reason = addable.length === 0 ? "Every execution environment is already on the list."
    : first ? null : "The remaining environments use a tool that is already on the list.";
  return (
    <>
      <form
        className="project-environments__add"
        onSubmit={event => {
          event.preventDefault();
          if (chosen) onSave([...ids, chosen]);
        }}
      >
        <label className="settings-field">
          <span>Add an execution environment</span>
          <select className="settings-select" value={chosen || ""} disabled={!first} onChange={event => setPicked(event.target.value)}>
            {addable.map(candidate => (
              <option key={candidate.id} value={candidate.id} disabled={Boolean(candidate.blockedReason)}>
                {candidate.blockedReason ? `${candidate.name} (${candidate.blockedReason})` : candidate.name}
              </option>
            ))}
          </select>
        </label>
        <button type="submit" className="wizard-button" disabled={!first}>Add to the end</button>
      </form>
      {reason ? <p className="settings-help project-environments__reason">{reason}</p> : null}
    </>
  );
}

export function ProjectExecutionEnvironments({ store, onSave = () => {}, onManage = () => {} }) {
  const { view = null, error = null } = useStoreSelector(store, selectModel) || {};
  const project = view?.project || null;
  const ids = (project?.entries || []).map(entry => entry.environment_id);
  return (
    <fieldset className="settings-section project-environments">
      <legend>Available execution environments</legend>
      <p className="settings-help">
        The order is the priority. A session starts on the first environment that can be used; one that is held by a
        limit or signed out is skipped for the next.
      </p>
      {!project ? (
        <p className="settings-help">
          {view ? "Open a project to choose its execution environments." : "Loading execution environments…"}
        </p>
      ) : (
        <>
          <ol className="project-environments__list" aria-label="Available execution environments, in priority order">
            {project.entries.map((entry, index) => (
              <Row key={entry.environment_id} entry={entry} index={index} ids={ids} onSave={onSave} />
            ))}
          </ol>
          {project.saved ? null : (
            <p className="settings-help">
              This is the order the project launches with today. It is saved the first time you change it.
            </p>
          )}
          <Add view={view} ids={ids} onSave={onSave} />
          <button type="button" className="wizard-button" data-action="environment-manage" onClick={onManage}>
            Manage execution environments
          </button>
          {error ? <p className="settings-status" data-kind="error" role="alert">{error}</p> : null}
        </>
      )}
    </fieldset>
  );
}
