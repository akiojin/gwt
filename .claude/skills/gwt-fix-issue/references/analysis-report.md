# Issue Analysis Report Template

## Full Report Structure

Output must use this structure for non-SPEC issues before execution:

````text
## Issue Analysis Report: #<number>

**Issue Type:** BUG | FEATURE | ENHANCEMENT | DOCUMENTATION | QUESTION | UNCLASSIFIED
**Title:** <issue title>
**State:** OPEN | CLOSED
**Labels:** <label1>, <label2>, ...
**Assignees:** <assignee1>, <assignee2>, ...
**Execution Path:** DIRECT-FIX | EXISTING-SPEC | NEW-SPEC
Actionable items: <N>

---

### EXTRACTED CONTEXT

#### Error Messages
- `<error message 1>`

#### Stack Traces
~~~
<stack trace>
~~~

#### File References
- `path/to/file.ext:42` [EXISTS]

#### Repro Steps
<extracted Steps to Reproduce section>

#### Expected vs Actual
- **Expected:** <extracted expected behavior>
- **Actual:** <extracted actual behavior>

---

### CODEBASE MATCHES

#### M1. <file or symbol>
- **Path:** `path/to/file.ext:line`
- **Relevance:** Why this file matters

---

### ACTIONABLE

#### A1. [CATEGORY] <1-line title>
- **What:** Factual statement
- **Where:** file_path:line_number
- **Evidence:** Verbatim quote from issue or codebase
- **Action:** Specific fix or handoff action
- **Confidence:** High | Medium | Low

---

### INFORMATIONAL

#### I1. [CATEGORY] <1-line title>
- **What / Note**

---

### LINKED CONTEXT

#### Linked PRs
- PR #<number>: <title> [<state>]

#### Cross-references
- #<number>

#### Comments Summary
- <N> comments from <M> authors
- Key points: ...

---

**Summary:** <N> actionable items, <M> informational items, <K> codebase matches.
````

## Anti-Patterns

### Prohibited Language

| Prohibited | Required Alternative |
|---|---|
| "We should look into..." | "Edit `path/file.ts:42` to..." |
| "There seem to be some issues" | "3 actionable items detected" |
| "This might be causing..." | "Root cause: `<error from issue>`" |
| "Consider fixing..." / "It looks like..." | "Action: Fix `<what>` in `<where>`" |
| "Various errors are reported" | "2 error messages extracted: `<msg1>`, `<msg2>`" |
| "Some files are involved" | "3 file references: `src/a.ts:42`, `src/b.rs:10`, `src/c.py`" |
| "I'll try to fix this" | "Action: <specific fix>" |

### Structural Prohibitions

- Prose paragraphs for reporting. Use A1/I1 item format.
- Omitting the Evidence field in any ACTIONABLE item.
- Combining multiple independent problems into a single item.
- Omitting file paths or line numbers when the script output contains them.
