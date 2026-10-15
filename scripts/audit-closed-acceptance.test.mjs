import assert from "node:assert/strict";
import test from "node:test";

import { auditClosedAcceptance, uncheckedAcceptance } from "./audit-closed-acceptance.mjs";

test("acceptance audit keeps every unchecked item, including Japanese and unnumbered criteria", () => {
  assert.deepEqual(uncheckedAcceptance(`## 受け入れ基準
- [x] AC-1: done
- [ ] AC-2: remaining
* [ ] additional check
## Notes
- [ ] not acceptance
`), ["AC-2", "additional check"]);
  assert.deepEqual(uncheckedAcceptance("## ACCEPTANCE CRITERIA\n- [X] AC-1: done\n"), []);
  assert.deepEqual(uncheckedAcceptance("## 受け入れ条件\n- [ ] AC-A: remaining\n"), ["AC-A"]);
});

test("acceptance audit ignores code examples and SPEC section boundaries", () => {
  assert.deepEqual(uncheckedAcceptance(`\`\`\`markdown
## Acceptance Criteria
- [ ] AC-example: example
\`\`\`
## Acceptance Criteria
- [ ] AC-1: remaining
    - [ ] indented code
<!-- artifact:spec END -->
- [ ] task outside the spec
`), ["AC-1"]);
});

test("closed audit requests all pages and emits only COMPLETED Issues with unchecked acceptance", () => {
  const issue = (number, body, extra = {}) => ({
    number, state: "closed", state_reason: "completed", html_url: `https://example.test/issues/${number}`,
    body, ...extra,
  });
  const open = "## Acceptance Criteria\n- [ ] AC-1: remaining\n";
  const done = "## Acceptance Criteria\n- [x] AC-1: done\n";
  const report = auditClosedAcceptance("owner/repo", (command, args, options) => {
    assert.equal(command, "gh");
    assert.deepEqual(args, ["api", "--method", "GET", "repos/owner/repo/issues?state=closed&per_page=100", "--paginate", "--slurp"]);
    assert.equal(options.encoding, "utf8");
    return { status: 0, stdout: JSON.stringify([
      [issue(1, open), issue(2, done), issue(3, open, { state_reason: "not_planned" })],
      [issue(4, open, { pull_request: {} }), issue(5, open, { state: "open" }), issue(6, open)],
    ]) };
  });
  assert.deepEqual(report, [
    { number: 1, url: "https://example.test/issues/1", unchecked: ["AC-1"] },
    { number: 6, url: "https://example.test/issues/6", unchecked: ["AC-1"] },
  ]);
});

test("closed audit fails on GitHub errors or an invalid inventory instead of returning an empty success", () => {
  assert.throws(() => auditClosedAcceptance("owner/repo", () => ({ status: 1, stderr: "permission denied" })), /permission denied/);
  assert.throws(() => auditClosedAcceptance("owner/repo", () => ({ status: 0, stdout: "{}" })), /page arrays/);
  assert.throws(() => auditClosedAcceptance("owner/repo", () => ({ status: 0, stdout: "not JSON" })), SyntaxError);
});
