import { useEffect, useState } from "react";
import { DeletionImpact } from "./AccountPool.jsx";

// Issue #7741 (SPEC #7007 items 77-82): registering, editing and deleting an
// execution environment. Every row comes from the backend's definitions; an
// action that is not available stays visible, disabled, with its reason.

const FAST_MODE_TOOLS = new Set(["claude", "codex"]);

function Back({ onNavigate, to = "environments", label = "Execution environments" }) {
  return (
    <button type="button" className="exec-env__back" onClick={() => onNavigate(to)}>
      {`← ${label}`}
    </button>
  );
}

function RequestError({ error }) {
  return error ? <p className="exec-env__error" role="alert">{error}</p> : null;
}

function UsedBy({ environment }) {
  const projects = environment.projects || [];
  return (
    <p className="exec-env__meta" data-used-by>
      {projects.length === 0
        ? "Not on any project's list."
        : `Used by ${projects.map(project => `${project.title} (priority ${project.priority})`).join(", ")}`}
    </p>
  );
}

export function AddEnvironment({ model, headingRef, onNavigate, onAdd }) {
  const accounts = model.registeredAccounts || [];
  const [accountId, setAccountId] = useState(accounts[0]?.id || "");
  const account = accounts.find(item => item.id === accountId) || accounts[0] || null;
  const options = (model.viaOptions || []).filter(option => option.account_id === account?.id);
  const open = options.filter(option => !option.blocked_reason);
  const [toolId, setToolId] = useState("");
  const tool = open.find(option => option.via_tool === toolId) || open[0] || null;
  const suggested = account && tool ? `${account.name} (via ${tool.via_tool_name})` : "";
  const [name, setName] = useState(null);
  const blocked = options.filter(option => option.blocked_reason);
  return (
    <>
      <div className="exec-env__header">
        <Back onNavigate={onNavigate} />
        <h1 ref={headingRef} tabIndex={-1}>Add an execution environment</h1>
        <p className="exec-env__lead">
          Register an account reached through a tool that reads its own settings. An account&apos;s own tool is added
          with the account, so it is not registered here.
        </p>
      </div>
      {accounts.length === 0 ? (
        <div className="exec-env__empty">
          <h2>No account to register yet</h2>
          <p>An account appears once its tool is installed on this device.</p>
        </div>
      ) : (
        <form
          className="exec-env__form"
          aria-label="Add an execution environment"
          onSubmit={event => {
            event.preventDefault();
            if (account && tool) onAdd({ account_id: account.id, via_tool: tool.via_tool, name: (name ?? suggested).trim() || null });
          }}
        >
          <label className="exec-env__field">
            <span>Account</span>
            <select value={account?.id || ""} onChange={event => { setAccountId(event.target.value); setName(null); }}>
              {accounts.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
            </select>
          </label>
          <label className="exec-env__field">
            <span>Tool to go through</span>
            <select value={tool?.via_tool || ""} disabled={open.length === 0} onChange={event => { setToolId(event.target.value); setName(null); }}>
              {options.map(option => (
                <option key={option.via_tool} value={option.via_tool} disabled={Boolean(option.blocked_reason)}>
                  {option.blocked_reason ? `${option.via_tool_name} (not available)` : option.via_tool_name}
                </option>
              ))}
            </select>
          </label>
          {blocked.length > 0 ? (
            <ul className="exec-env__reasons" aria-label="Tools that cannot be chosen">
              {blocked.map(option => (
                <li key={option.via_tool} data-blocked-tool={option.via_tool}>
                  <strong>{option.via_tool_name}</strong>
                  {` — ${option.blocked_reason}`}
                </li>
              ))}
            </ul>
          ) : null}
          <label className="exec-env__field">
            <span>Name</span>
            <input type="text" value={name ?? suggested} disabled={!tool} onChange={event => setName(event.target.value)} />
          </label>
          <p className="exec-env__hint">
            Runtime options are not set here. The tool uses the provider and model in its own settings file.
          </p>
          <RequestError error={model.error} />
          <div className="exec-env__actions">
            <button type="submit" className="exec-env__primary" disabled={!tool || model.saving}>Register</button>
            <button type="button" onClick={() => onNavigate("environments")}>Cancel</button>
          </div>
        </form>
      )}
    </>
  );
}

function Pair({ environment }) {
  return (
    <section className="exec-env__detail" aria-label="Account and tool">
      <h2>Account and tool</h2>
      <dl className="exec-env__facts">
        <div><dt>Account</dt><dd>{environment.accountName || "Unknown account"}</dd></div>
        <div>
          <dt>Tool</dt>
          <dd>
            {environment.via}
            {environment.kind === "set"
              ? " — fixed for this account"
              : " — to go through another tool, delete this environment and register it again"}
          </dd>
        </div>
      </dl>
      <UsedBy environment={environment} />
    </section>
  );
}

function DeleteAction({ environment, onNavigate }) {
  const reason = environment.deleteBlockedReason;
  return (
    <div className="exec-env__actions">
      <button
        type="button"
        className="exec-env__danger"
        disabled={Boolean(reason)}
        aria-describedby={reason ? "exec-env-delete-reason" : undefined}
        onClick={() => onNavigate("delete", environment.id)}
      >
        Delete
      </button>
      {reason ? <p className="exec-env__reason" id="exec-env-delete-reason">{reason}</p> : null}
    </div>
  );
}

function ToolSettings({ environment }) {
  const settings = environment.toolSettings || { values: [] };
  return (
    <section className="exec-env__detail" aria-label="Current settings">
      <h2>{`Current settings (read from ${environment.via})`}</h2>
      {settings.values.length === 0 ? (
        <p className="exec-env__hint">No provider or model is set in its settings file yet.</p>
      ) : (
        <dl className="exec-env__facts">
          {settings.values.map(setting => (
            <div key={setting.label}><dt>{setting.label}</dt><dd>{setting.value}</dd></div>
          ))}
        </dl>
      )}
      <p className="exec-env__hint">
        {environment.readOnlyReason}
        {settings.path ? <>{" File: "}<code>{settings.path}</code></> : null}
      </p>
    </section>
  );
}

export function EditEnvironment({ model, environment, headingRef, onNavigate, onUpdate }) {
  const [name, setName] = useState(environment?.name || "");
  const [modelName, setModelName] = useState(environment?.runtime?.model || "");
  // Another window may have saved; follow the saved values when they change.
  useEffect(() => { setName(environment?.name || ""); }, [environment?.id, environment?.name]);
  useEffect(() => { setModelName(environment?.runtime?.model || ""); }, [environment?.id, environment?.runtime?.model]);
  if (!environment) {
    return (
      <div className="exec-env__header">
        <Back onNavigate={onNavigate} />
        <h1 ref={headingRef} tabIndex={-1}>Execution environment</h1>
        <p className="exec-env__lead">{model.loaded ? "This execution environment no longer exists." : "Loading execution environment…"}</p>
      </div>
    );
  }
  const readOnly = Boolean(environment.readOnlyReason);
  const runtime = environment.runtime || {};
  const saveRuntime = change => onUpdate(environment.id, { runtime: { model: runtime.model || null, skip_permissions: Boolean(runtime.skip_permissions), fast_mode: Boolean(runtime.fast_mode), ...change } });
  const saveName = () => {
    const next = name.trim();
    if (next && next !== environment.name) onUpdate(environment.id, { name: next });
    else setName(environment.name);
  };
  const saveModel = () => {
    const next = modelName.trim();
    if (next !== (runtime.model || "")) saveRuntime({ model: next || null });
  };
  const onEnter = save => event => { if (event.key === "Enter") save(); };
  const fastMode = FAST_MODE_TOOLS.has(environment.viaTool);
  return (
    <>
      <div className="exec-env__header">
        <Back onNavigate={onNavigate} />
        <h1 ref={headingRef} tabIndex={-1}>{readOnly ? environment.name : `Edit ${environment.name}`}</h1>
        <p className="exec-env__lead">
          {readOnly
            ? "Its runtime options are the current values in the tool's own settings file."
            : "Changes are saved as you make them and take effect from the next session. Running sessions are not affected."}
        </p>
      </div>
      <section className="exec-env__detail" aria-label="Name">
        <label className="exec-env__field">
          <span>Name</span>
          <input type="text" value={name} onChange={event => setName(event.target.value)} onBlur={saveName} onKeyDown={onEnter(saveName)} />
        </label>
      </section>
      <Pair environment={environment} />
      {readOnly ? <ToolSettings environment={environment} /> : (
        <section className="exec-env__detail" aria-label="Runtime options">
          <h2>Runtime options</h2>
          <label className="exec-env__field">
            <span>Model</span>
            <input
              type="text"
              value={modelName}
              placeholder="Tool default"
              onChange={event => setModelName(event.target.value)}
              onBlur={saveModel}
              onKeyDown={onEnter(saveModel)}
            />
          </label>
          <p className="exec-env__hint">Leave empty to use the tool&apos;s default model. Automatic tiers choose their own model.</p>
          <label className="settings-toggle">
            <input type="checkbox" checked={Boolean(runtime.skip_permissions)} onChange={() => saveRuntime({ skip_permissions: !runtime.skip_permissions })} />
            <span>Skip permissions — do not ask before each risky operation</span>
          </label>
          <label className="settings-toggle">
            <input
              type="checkbox"
              checked={fastMode && Boolean(runtime.fast_mode)}
              disabled={!fastMode}
              aria-describedby={fastMode ? undefined : "exec-env-fast-reason"}
              onChange={() => saveRuntime({ fast_mode: !runtime.fast_mode })}
            />
            <span>Fast mode</span>
          </label>
          {fastMode ? null : <p className="exec-env__reason" id="exec-env-fast-reason">{`${environment.via} has no Fast mode.`}</p>}
          <p className="exec-env__hint">
            An option turned on here applies to every project that uses this environment. Turning it off does not turn
            it off for a launch candidate that sets it itself.
          </p>
        </section>
      )}
      <p className="exec-env__status" role="status">{model.saving ? "Saving…" : model.saved ? "Saved" : ""}</p>
      <RequestError error={model.error} />
      <DeleteAction environment={environment} onNavigate={onNavigate} />
    </>
  );
}

export function DeleteEnvironment({ model, environment, headingRef, onNavigate, onDelete }) {
  if (!environment) {
    return (
      <div className="exec-env__header">
        <Back onNavigate={onNavigate} />
        <h1 ref={headingRef} tabIndex={-1}>Execution environment</h1>
        <p className="exec-env__lead">{model.loaded ? "This execution environment no longer exists." : "Loading execution environment…"}</p>
      </div>
    );
  }
  const projects = environment.projects || [];
  return (
    <>
      <div className="exec-env__header">
        <Back onNavigate={() => onNavigate("edit", environment.id)} label={environment.name} />
        <h1 ref={headingRef} tabIndex={-1}>Delete this execution environment?</h1>
        <p className="exec-env__lead">{environment.name}</p>
      </div>
      <section className="exec-env__detail" aria-label="Projects that use it">
        <h2>Projects that use it</h2>
        {projects.length === 0 ? (
          <p className="exec-env__hint">No project has it on its list.</p>
        ) : (
          <>
            <p className="exec-env__hint">It is taken off the list of each of these projects:</p>
            <ul className="exec-env__reasons" aria-label="Projects that use it">
              {projects.map(project => (
                <li key={project.project_root} data-project={project.title}>
                  <strong>{project.title}</strong>
                  {` — priority ${project.priority}`}
                </li>
              ))}
            </ul>
          </>
        )}
        <p className="exec-env__hint">
          {`The ${environment.accountName || "account"} account and its other environments stay. Running sessions are not affected.`}
        </p>
      </section>
      <DeletionImpact impact={environment.deleteImpact} />
      <RequestError error={model.error} />
      <div className="exec-env__actions">
        <button type="button" className="exec-env__danger" disabled={model.saving} onClick={() => onDelete(environment.id)}>
          Delete
        </button>
        <button type="button" onClick={() => onNavigate("edit", environment.id)}>Cancel</button>
      </div>
    </>
  );
}
