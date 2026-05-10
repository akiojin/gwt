function createButton(document, className, label, ariaLabel) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = className;
  button.textContent = label;
  if (ariaLabel) button.setAttribute("aria-label", ariaLabel);
  return button;
}

function appendEnvRow(document, rows, key = "", value = "") {
  const row = document.createElement("div");
  row.className = "custom-agent-env-row";
  row.dataset.role = "custom-agent-env-row";

  const keyInput = document.createElement("input");
  keyInput.className = "custom-agent-env-input";
  keyInput.dataset.role = "custom-agent-env-key";
  keyInput.setAttribute("aria-label", "Environment key");
  keyInput.placeholder = "KEY";
  keyInput.value = key;

  const valueInput = document.createElement("input");
  valueInput.className = "custom-agent-env-input";
  valueInput.dataset.role = "custom-agent-env-value";
  valueInput.setAttribute("aria-label", "Environment value");
  valueInput.placeholder = "value";
  valueInput.value = value;

  const remove = createButton(
    document,
    "icon-button custom-agent-env-remove",
    "×",
    "Remove environment row",
  );
  remove.dataset.role = "custom-agent-env-remove";
  remove.addEventListener("click", () => row.remove());

  row.appendChild(keyInput);
  row.appendChild(valueInput);
  row.appendChild(remove);
  rows.appendChild(row);
  return row;
}

function collectEnv(editor) {
  const env = {};
  for (const row of editor.querySelectorAll("[data-role='custom-agent-env-row']")) {
    const key = row.querySelector("[data-role='custom-agent-env-key']")?.value.trim();
    if (!key) continue;
    env[key] = row.querySelector("[data-role='custom-agent-env-value']")?.value || "";
  }
  return env;
}

function cloneModeArgs(modeArgs) {
  if (!modeArgs) return modeArgs;
  return {
    normal: [...(modeArgs.normal || [])],
    continue: [...(modeArgs.continue || [])],
    resume: [...(modeArgs.resume || [])],
  };
}

export function renderCustomAgentEnvEditor({ document, agent, onSave, onCancel }) {
  const editor = document.createElement("section");
  editor.className = "custom-agent-env-editor";
  editor.dataset.role = "custom-agent-env-editor";

  const title = document.createElement("h3");
  title.className = "custom-agent-env-title";
  title.textContent = "Environment";
  editor.appendChild(title);

  const rows = document.createElement("div");
  rows.className = "custom-agent-env-rows";
  rows.dataset.role = "custom-agent-env-rows";
  const entries = Object.entries(agent.env || {});
  for (const [key, value] of entries) {
    appendEnvRow(document, rows, key, value);
  }
  if (entries.length === 0) {
    appendEnvRow(document, rows);
  }
  editor.appendChild(rows);

  const actions = document.createElement("div");
  actions.className = "custom-agent-env-actions";

  const add = createButton(
    document,
    "wizard-button custom-agent-env-add",
    "+ Env",
    "Add environment row",
  );
  add.dataset.role = "custom-agent-env-add";
  add.addEventListener("click", () => appendEnvRow(document, rows));
  actions.appendChild(add);

  const save = createButton(
    document,
    "wizard-button custom-agent-env-save",
    "Save",
    "Save environment changes",
  );
  save.dataset.role = "custom-agent-env-save";
  save.addEventListener("click", () => {
    onSave({
      ...agent,
      default_args: [...(agent.default_args || [])],
      mode_args: cloneModeArgs(agent.mode_args),
      skip_permissions_args: [...(agent.skip_permissions_args || [])],
      env: collectEnv(editor),
    });
  });
  actions.appendChild(save);

  const cancel = createButton(
    document,
    "wizard-button custom-agent-env-cancel",
    "Cancel",
    "Cancel environment editing",
  );
  cancel.dataset.role = "custom-agent-env-cancel";
  cancel.addEventListener("click", () => onCancel());
  actions.appendChild(cancel);

  editor.appendChild(actions);
  return editor;
}
