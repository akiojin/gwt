## User Verification Handoff (post-Executed)

This phase runs only when the owner explicitly requests a human check in an
`interactive` launch. User Verification Result is optional otherwise; do not
infer a request from a missing, pending, or legacy deferred field.
In `autonomous` mode, skip
straight to recording `User Verification Result: n/a (autonomous)` plus the
`Agent Visual Check:` line, then finalize `Overall`. Do
not execute any step below, and in particular do not call the question tool in
step 5.

When `Overall` would otherwise be `PASS` (every `Executed` entry passed) and
the owner explicitly requests the check in an `interactive` launch, and
`--mode full` or `--mode pre-pr` is active:

1. Compute the `User Verification` block:
   - `Status: required` if any changed surface is marked Required above.
   - `Status: recommended` if only Recommended surfaces changed.
   - `Status: skipped(<reason>)` for `--mode quick`, `Changed surfaces:
     (none)`, docs-only changes, `--skip-user-check`, or any explicit
     skip rule above. The reason must be human-readable and specific.
2. Resolve and print the **Verification Target Card** before any instructions:
   - `Owner Issue/SPEC:` identifies the work item whose behavior is being
     checked.
   - `Work purpose:` distinguishes the concrete objective from other active
     agents that may share the same Issue, branch, or commit.
   - `Success Goal:` states what behavior was achieved and what observation
     proves success without relying on prior conversation.
   - `Requesting agent/session:` names the provider/agent and a stable session
     identifier such as `GWT_SESSION_ID`.
   - `Branch:`, `Absolute worktree:`, and `Commit:` identify the exact code.
   - `Prepared instance ID:` names the exact runtime using its PID, window or
     pane ID, port, or invocation label and ties it to the identified checkout.
   - `URL or launch target:` names the reachable URL or exact non-URL target.
   The full card, not any single field, is the identity. A bare URL, process
   name, PID, or generic "this change" is insufficient when multiple agents or
   instances may be running.
3. Generate the 4-step 導線 ("How to access the changed behavior") using the
   project's build / launch conventions. The structure is fixed across all
   project types — **always exactly four labelled steps in this order**:
   1. **build** — the project's build command for the smallest target that
      exercises the change. When the target is already prepared, state the
      exact command the agent already completed and its result.
   2. **launch** — how to start the running artifact (binary / server /
      editor / interpreter / web page). For a prepared runtime, tell the user
      how to focus that exact Prepared instance ID; never instruct them to
      start another process or discover a different dynamic URL.
   3. **navigate** — how to reach the changed feature from the entry point.
   4. **observe** — what the user should look at, click, or interact with to
      confirm the change.
   See [references/user-verification-guide.md](user-verification-guide.md) for project-type-specific
   launch patterns (Rust CLI / WebView / Unity Editor / .NET WPF / Python
   service / generic TUI / long-running service).
4. Apply the **Manual Feasibility Gate** before producing **Check Items**:
   - Every manual checkbox must be reachable in the prepared instance and use
     the exact `Action → Expected` shape.
   - Include all three categories: Expected, Edge, and Regression.
   - Do not ask the user to manufacture remote failures, mutate unrelated
     external state, switch to an unidentified instance, or infer which agent
     owns the request.
   - Move every unreachable boundary to **Automated-only Evidence** and name
     the exact command and named test that covers it. The item must match a
     `PASS` entry under `Executed`.
   - When a real failure path is not reachable, use the nearest safe reachable
     boundary for the Edge checkbox and report the destructive or external
     failure path only under Automated-only Evidence.
   - Before handoff, map every scoped acceptance boundary to either one
     reachable checkbox or one valid Automated-only Evidence item. Stop with
     `Overall: FAIL` if any boundary has neither form of evidence.

   Use this exact minimum shape:

   ```markdown
   - [ ] Expected — Action: <concrete action> → Expected: <decidable result>
   - [ ] Edge — Action: <reachable action> → Expected: <decidable result>
   - [ ] Regression — Action: <adjacent action> → Expected: <unchanged result>
   ```

5. Ask the user via the platform's selection question tool
   (`AskUserQuestionTool` for Claude Code, `request_user_input` for Codex,
   the closest equivalent for other runtimes) to choose one of:
   - `Confirmed` — observed behavior matches expectations; record
     `User Verification Result: confirmed`.
   - `Rejected(<reason>)` — observed behavior does not match; downgrade
     `Overall: FAIL` and preserve the user's reason.
   - `Skip with reason(<reason>)` — user defers verification with an
     explicit justification; record `User Verification Result:
     skipped(<reason>)`.

If no selection UI exists in the current runtime, fall back to plain-text
prompting but keep the same three-option discipline. An autonomous session is
not a runtime without a selection UI — it is a runtime where asking parks the
Issue, so it never reaches this step at all.
