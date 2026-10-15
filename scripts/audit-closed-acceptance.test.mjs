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

test("closed audit reads only routed SPEC comments and assembles split acceptance", () => {
  const issue = {
    number: 7, state: "closed", state_reason: "completed", html_url: "https://example.test/issues/7",
    labels: [{ name: "gwt-spec" }],
    body: "<!-- gwt-spec id=7 version=1 -->\n<!-- sections:\nspec=comment:12,comment:11\n-->",
  };
  const comments = {
    11: "<!-- artifact:spec BEGIN part=1/2 -->\n## Acceptance Criteria\n- [x] AC-1: done\n<!-- artifact:spec END part=1/2 -->",
    12: "<!-- artifact:spec BEGIN part=2/2 -->\n- [ ] AC-2: remaining\n<!-- artifact:spec END part=2/2 -->",
  };
  const calls = [];
  assert.deepEqual(auditClosedAcceptance("owner/repo", (_command, args) => {
    calls.push(args[3]);
    const commentId = args[3].match(/\/issues\/comments\/(\d+)$/)?.[1];
    return { status: 0, stdout: JSON.stringify(commentId ? { body: comments[commentId] } : [[issue]]) };
  }), [{ number: 7, url: issue.html_url, unchecked: ["AC-2"] }]);
  assert.deepEqual(calls, [
    "repos/owner/repo/issues?state=closed&per_page=100",
    "repos/owner/repo/issues/comments/12", "repos/owner/repo/issues/comments/11",
  ]);
  issue.body = issue.body.replace("comment:12,comment:11", "comment:11,comment:12");
  for (const id of [11, 12]) comments[id] = comments[id].replaceAll(/ part=\d\/2/g, "");
  assert.deepEqual(auditClosedAcceptance("owner/repo", (_command, args) => {
    const commentId = args[3].match(/\/issues\/comments\/(\d+)$/)?.[1];
    return { status: 0, stdout: JSON.stringify(commentId ? { body: comments[commentId] } : [[issue]]) };
  }), [{ number: 7, url: issue.html_url, unchecked: ["AC-2"] }]);
  delete comments[11];
  assert.throws(() => auditClosedAcceptance("owner/repo", (_command, args) => {
    const commentId = args[3].match(/\/issues\/comments\/(\d+)$/)?.[1];
    return { status: 0, stdout: JSON.stringify(commentId ? { body: comments[commentId] } : [[issue]]) };
  }), /missing SPEC section/);
  // Plain bug reports can quote a SPEC routing index as a code example.
  issue.labels = [];
  issue.body = `\`\`\`markdown\n${issue.body}\n\`\`\`\n## Acceptance Criteria\n- [ ] AC-plain: remaining\n`;
  assert.deepEqual(auditClosedAcceptance("owner/repo", (_command, args) => {
    assert.equal(args[3], "repos/owner/repo/issues?state=closed&per_page=100");
    return { status: 0, stdout: JSON.stringify([[issue]]) };
  }), [{ number: 7, url: issue.html_url, unchecked: ["AC-plain"] }]);
});
