# Intake Reference

Detailed logic for Phase 1 of gwt-spec-design.

## Preflight search protocol

### Required searches

Run these before any creation or routing decision:

1. **gwt-issue-search** with at least 2 semantic queries:
   - One using the primary feature keyword
   - One using an alternative phrasing or related concept
2. **gwt-spec-search** with at least 2 semantic queries:
   - Same query strategy as above
3. **Local SPEC listing** if ownership is still unclear:

```bash
python3 ".claude/scripts/spec_artifact.py" --repo "." --list-all
```

### Search evaluation

- If a SPEC or Issue clearly owns the scope, route to it (EXISTING-SPEC or EXISTING-ISSUE).
- If multiple candidates exist, present them to the user for disambiguation.
- If no match, proceed with the interview.
- Never skip search because the idea "sounds new".

## Interview protocol

### Principles

- One question at a time. Wait for the answer before asking the next.
- Use the platform-native question tool when available.
- Prefer multiple-choice framing when it reduces ambiguity quickly.
- Ask only what materially changes routing, scope, behavior, or testability.
- Do not ask questions already answered by the repo, existing specs, or search.
- Do not answer product or scope decisions on the user's behalf.

### Question priority order

1. **User goal / why now** - What problem does this solve? Why is it urgent?
2. **Target user or actor** - Who benefits? Which subsystem is affected?
3. **Current pain or missing capability** - What workaround exists today?
4. **Scope boundary and explicit non-goals** - What is explicitly excluded?
5. **Success condition for the first slice** - What is the minimum viable outcome?
6. **Integration target or owning subsystem** - Where does this fit in the architecture?

### When to stop interviewing

Stop when all of these are true:

- The user's goal is clear enough to route correctly.
- The first slice has a visible success condition.
- High-impact scope ambiguity has been resolved by the user.
- The SPEC vs Issue decision is justified.

### Broad request handling

If the request is too broad for one SPEC:

1. Identify 2-4 natural slices.
2. Present them to the user with scope boundaries.
3. Ask the user to pick the first slice.
4. Continue with the selected slice.

## Intake memo structure

Maintain in conversation context (not as a file):

```markdown
## Intake Memo
- Request: <one-sentence summary of what the user wants>
- Why now: <urgency or trigger>
- Users / actors: <who benefits or interacts>
- In scope: <what this covers>
- Out of scope: <what is explicitly excluded>
- Constraints: <technical, time, or design constraints>
- Success signal: <how to know it works>
- Open questions: <unresolved items>
- Candidate owner: <existing SPEC/Issue if found, or "none">
```

Update it as answers arrive. Keep it short.

## Registration decision format

After enough signal:

```text
## Registration Decision

**Duplicate Check:** CLEAN | EXISTING-ISSUE | EXISTING-SPEC | AMBIGUOUS
**Chosen Path:** EXISTING-ISSUE | EXISTING-SPEC | NEW-SPEC | ISSUE | TOO-BROAD-SPLIT-FIRST
**Action:** <specific next step>

### Intake Memo
- <final memo bullets>

### Candidates
- #<number> <title> [issue] - <why it matches>
- SPEC-<id> <title> [spec] - <why it matches>
```

## Routing rules

| Duplicate Check | Chosen Path | Action |
|---|---|---|
| EXISTING-SPEC | EXISTING-SPEC | Skip to Phase 4 (clarify owning SPEC) |
| EXISTING-ISSUE | EXISTING-ISSUE | Hand off to `gwt-fix-issue` |
| CLEAN | NEW-SPEC | Continue to Phase 2 (domain discovery) |
| CLEAN | ISSUE | Hand off to `gwt-register-issue` |
| AMBIGUOUS | -- | Present candidates to user, ask to disambiguate |
| -- | TOO-BROAD-SPLIT-FIRST | Decompose, ask user for first slice |

Do not ask "should I proceed?" after the decision. Move into the next phase
unless a real product decision still blocks progress.
