// Issue #7771 (SPEC #7007 mock v3) — a new launch that starts work on an
// Issue is planned in three stages after the start method is chosen: Issue &
// assignee, Launch plan, Confirm. The first stage offers the project's
// registered agents with their traits; the second holds the branch, the
// place to run and the permissions next to the assignee.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";

const surface = readFileSync(new URL("../launch-wizard-surface.js", import.meta.url), "utf8");
const css = readFileSync(new URL("../styles/app.css", import.meta.url), "utf8");

function surfaceFunction(name) {
  const start = surface.indexOf(`      function ${name}(`);
  assert.notEqual(start, -1, `${name} must exist`);
  return surface.slice(start, surface.indexOf("\n      }\n", start) + 9);
}

const HELPERS = [
  "createLaunchSection",
  "createLaunchField",
  "appendIssuePlanTraits",
  "appendIssuePlanBranchRow",
  "appendIssuePlanPermissionsRow",
];

function render(name, ...args) {
  const { document } = parseHTML("<section id='parent'></section>");
  const createNode = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const parent = document.getElementById("parent");
  const source = [...new Set([...HELPERS, name])].map(surfaceFunction).join("\n");
  new Function("createNode", "parent", "args", `${source}; ${name}(parent, ...args);`)(
    createNode,
    parent,
    args,
  );
  return parent;
}

const PLAN = {
  stage: "assignee",
  issue_number: 5561,
  assignees: [
    {
      agent_member_id: "busy",
      name: "Himeno Daiki",
      selected: false,
      available: false,
      status: "Working on #7007 now. It can be chosen once that work ends.",
      reasoning: "Standard",
      speed: "Fast",
      personality: "Screens and experience first.",
    },
    {
      agent_member_id: "oda",
      name: "Oda Yohei",
      selected: true,
      available: true,
      status: "Assigned to this launch",
      reasoning: "Max",
      speed: "Fastest",
      personality: "Implementation first.",
    },
    {
      agent_member_id: "ando",
      name: "Ando Kenji",
      selected: false,
      available: true,
      status: "Can take this launch",
      reasoning: "Standard",
      speed: "Tool default",
      personality: null,
    },
  ],
  branch: "work/issue-5561",
  branch_detail: "Created from develop",
  permissions: "Skips permission prompts",
};

test("stage one shows the Issue and every registered agent with its traits", () => {
  const assigned = [];
  const parent = render("appendIssuePlanAssignees", PLAN, (id) => assigned.push(id));
  assert.equal(parent.querySelector(".issue-plan-issue").textContent, "#5561");

  const options = [...parent.querySelectorAll(".assignee-option")];
  assert.deepEqual(
    options.map((option) => option.dataset.agentMemberId),
    ["busy", "oda", "ando"],
    "roster order is kept",
  );
  const traits = (option) =>
    [...option.querySelectorAll(".assignee-trait")].map((chip) => chip.textContent);
  assert.deepEqual(traits(options[1]), [
    "Reasoning Max",
    "Speed Fastest",
    "Personality Implementation first.",
  ]);
  // An unset trait says so, and a missing personality leaves no chip.
  assert.deepEqual(traits(options[2]), ["Reasoning Standard", "Speed Tool default"]);

  // The chosen agent is marked; an agent that cannot take the launch says why.
  assert.equal(options[1].getAttribute("aria-pressed"), "true");
  assert.equal(options[1].querySelector(".start-method-badge").textContent, "Assignee");
  assert.equal(options[2].getAttribute("aria-pressed"), "false");
  assert.equal(options[0].disabled, true);
  assert.match(options[0].textContent, /Working on #7007 now/);

  options[2].click();
  options[1].click();
  options[0].click();
  assert.deepEqual(assigned, ["ando"], "only a different, available agent is assigned");
});

test("stage two names the assignee and offers to change it", () => {
  let changes = 0;
  const parent = render("appendIssuePlanAssignee", PLAN, () => {
    changes += 1;
  });
  const row = parent.querySelector(".issue-plan-assignee");
  assert.match(row.textContent, /Oda Yohei/);
  assert.deepEqual(
    [...row.querySelectorAll(".assignee-trait")].map((chip) => chip.textContent),
    ["Reasoning Max", "Speed Fastest", "Personality Implementation first."],
  );
  parent.querySelector(".issue-plan-change-assignee").click();
  assert.equal(changes, 1);
});

test("stage two holds the branch and the permissions as rows of the plan", () => {
  const fixed = render("appendIssuePlanRows", PLAN, true);
  assert.match(fixed.querySelector(".issue-plan-branch").textContent, /work\/issue-5561/);
  assert.match(fixed.querySelector(".issue-plan-branch").textContent, /Created from develop/);
  assert.match(
    fixed.querySelector(".issue-plan-permissions").textContent,
    /Skips permission prompts/,
  );
  // A launch opened from a branch edits the branch with the wizard's own
  // controls, so the read-only row is not repeated.
  const editable = render("appendIssuePlanRows", PLAN, false);
  assert.equal(editable.querySelector(".issue-plan-branch"), null);
  assert.ok(editable.querySelector(".issue-plan-permissions"));
});

test("the surface draws the three stages from the backend view", () => {
  // Each stage of the rail jumps to the phase it stands for.
  assert.match(surface, /assignee:\s*"settings"/);
  assert.match(surface, /plan:\s*"runtime"/);
  assert.match(surface, /confirm:\s*"confirm"/);
  // The settings form is not drawn next to a stage.
  assert.match(surface, /const planStage = launchWizard\.issue_plan\?\.stage \|\| null;/);
  assert.match(surface, /const showSetupForms = showManualSetup && !isRuntimeConfirmation && !planStage;/);
  assert.match(
    surface,
    /planStage === "assignee"[\s\S]{0,400}kind:\s*"set_agent_member"/,
    "choosing an agent tells the backend",
  );
  assert.match(
    surface,
    /planStage === "plan"[\s\S]{0,400}gotoWizardStep\("settings"\)/,
    "changing the assignee goes back to stage one",
  );
  assert.match(
    surface,
    /planStage === "plan"[\s\S]{0,700}appendExecutionEnvironmentNote\(/,
    "the plan names the execution environment",
  );
});

test("the trait chips use Operator tokens only", () => {
  const start = css.indexOf(".assignee-traits");
  assert.notEqual(start, -1, "the chips are styled");
  const block = css.slice(start, css.indexOf("/* end issue-plan */", start));
  assert.ok(block.length > 0, "the block is closed by its marker");
  assert.doesNotMatch(block, /#[0-9a-fA-F]{3,8}\b|rgba?\(/, "no raw colours");
  assert.match(block, /var\(--color-/);
});
