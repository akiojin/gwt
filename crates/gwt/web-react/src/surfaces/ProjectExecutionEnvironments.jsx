import { useState } from "react";
import { useStoreSelector } from "../store/useStoreSelector.js";

// Issue #7741 (SPEC #7007 item 82): a project's "Available execution
// environments" list. The order is the priority: a session starts on the
// first environment that can be used, skipping one that is held by a limit
// or signed out. The list is saved with the project's Issues settings.
const selectModel = state => state;

// What differs between the two lists drawn here: a project's list and the
// Project Manager's own list on this machine (Issue #7732, items 194 and 233).
const PROJECT = Object.freeze({
  id: "project",
  legend: "Available execution environments",
  help: "The order is the priority. A session starts on the first environment that can be used; one that is held by a "
    + "limit or signed out is skipped for the next.",
  listLabel: "Available execution environments, in priority order",
  unavailable: "Open a project to choose its execution environments.",
  unsaved: "This is the order the project launches with today. It is saved the first time you change it.",
  removeLastReason: "Keep at least one. A project with no execution environment cannot start a session.",
  // A project's launch candidates are unique per tool.
  onePerTool: true,
});
const PM = Object.freeze({
  id: "pm",
  legend: "Project Manager execution environments",
  help: "The execution environments kept for the Project Manager on this machine. The order is the priority. "
    + "This list is separate from each project's list.",
  // The list is kept and guarded here; starting the Project Manager from it is not built yet.
  note: "The Project Manager's sessions do not follow this list yet. They start with the agent assigned in the agent ledger.",
  listLabel: "Project Manager execution environments, in priority order",
  unavailable: "No execution environment is registered on this machine yet.",
  unsaved: "This list was made from the accounts registered on this machine. It is saved the first time you change it.",
  removeLastReason: "Keep at least one. The Project Manager cannot start a session without an execution environment.",
  onePerTool: false,
});

function entryName(entry) {
  return entry.name || `${entry.tool_name} (not registered as an execution environment)`;
}

// Environments the project can still add, each with the reason it cannot be
// added when its tool is already on the list.
export function addableEnvironments(view, copy = PROJECT) {
  const entries = view?.[copy.id]?.entries || [];
  const listed = new Set(entries.map(entry => entry.environment_id));
  return (view?.environments || [])
    .filter(environment => !listed.has(environment.id))
    .map(environment => {
      const taken = copy.onePerTool ? entries.find(entry => entry.tool === environment.via_tool) : null;
      return {
        id: environment.id,
        name: environment.name,
        blockedReason: taken ? `Already uses ${environment.via_tool_name} through "${entryName(taken)}"` : null,
      };
    });
}

function Row({ entry, index, ids, copy, onSave }) {
  const last = index === ids.length - 1;
  const only = ids.length === 1;
  const move = offset => () => {
    const next = [...ids];
    [next[index], next[index + offset]] = [next[index + offset], next[index]];
    onSave(next);
  };
  const name = entryName(entry);
  const reasonId = `${copy.id}-environment-reason-${index}`;
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
      {only ? <p className="settings-help project-environments__reason" id={reasonId}>{copy.removeLastReason}</p> : null}
    </li>
  );
}

function Add({ view, ids, copy, onSave }) {
  const addable = addableEnvironments(view, copy);
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

function EnvironmentList({ copy, store, onSave, onManage }) {
  const { view = null, error = null } = useStoreSelector(store, selectModel) || {};
  const list = view?.[copy.id] || null;
  const entries = list?.entries || [];
  const ids = entries.map(entry => entry.environment_id);
  return (
    <fieldset className="settings-section project-environments" data-environment-list={copy.id}>
      <legend>{copy.legend}</legend>
      <p className="settings-help">{copy.help}</p>
      {copy.note ? <p className="settings-help">{copy.note}</p> : null}
      {entries.length === 0 ? (
        <p className="settings-help">{view ? copy.unavailable : "Loading execution environments…"}</p>
      ) : (
        <>
          <ol className="project-environments__list" aria-label={copy.listLabel}>
            {entries.map((entry, index) => (
              <Row key={entry.environment_id} entry={entry} index={index} ids={ids} copy={copy} onSave={onSave} />
            ))}
          </ol>
          {list.saved ? null : <p className="settings-help">{copy.unsaved}</p>}
          <Add view={view} ids={ids} copy={copy} onSave={onSave} />
          <button type="button" className="wizard-button" data-action="environment-manage" onClick={onManage}>
            Manage execution environments
          </button>
          {error ? <p className="settings-status" data-kind="error" role="alert">{error}</p> : null}
        </>
      )}
    </fieldset>
  );
}

export function ProjectExecutionEnvironments({ store, onSave = () => {}, onManage = () => {} }) {
  return <EnvironmentList copy={PROJECT} store={store} onSave={onSave} onManage={onManage} />;
}

// Issue #7732 (SPEC #7007 items 194 and 233): the Project Manager's own
// list, kept with this machine's settings.
export function PmExecutionEnvironments({ store, onSave = () => {}, onManage = () => {} }) {
  return <EnvironmentList copy={PM} store={store} onSave={onSave} onManage={onManage} />;
}
