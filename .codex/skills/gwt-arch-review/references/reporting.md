## Phase 5: Report Generation

**Goal**: Produce a single, actionable improvement report.

Reference: [references/report-format.md](report-format.md)

### Report structure

```markdown
# Architecture Review Report

**Repository**: <name>
**Scope**: <full repository | changed files since <base>>
**Date**: <date>
**Reviewed by**: gwt-arch-review

## Executive Summary

<2-3 sentences: overall health, top concern, recommended first action>

## Metrics

| Metric | Value | Status |
|--------|-------|--------|
| Total modules | N | — |
| Files > 300 lines | N | <ok/warn/critical> |
| Files > 500 lines | N | <ok/warn/critical> |
| Circular dependencies | N | <ok/warn/critical> |
| God objects | N | <ok/warn/critical> |
| Untested public APIs | N | <ok/warn/critical> |

## Findings

### Critical

- **[C1]** <title>
  - Files: <paths>
  - Issue: <description>
  - Impact: <why it matters>
  - Suggested action: <what to do>

### High

- **[H1]** ...

### Medium

- **[M1]** ...

### Low

- **[L1]** ...

## Improvement Proposals

Priority-ordered list of concrete improvements:

1. **<proposal title>** — Addresses [C1], [H2]
   - Estimated effort: <small/medium/large>
   - Suggested approach: <1-2 sentences>
   - Create SPEC via: `gwt-discussion`

2. ...

## Feedback Loop

To act on these findings:
1. Pick the top 1-3 proposals
2. Use `gwt-discussion` to create improvement SPECs
3. Follow the normal gwt-plan-spec → gwt-build-spec pipeline
4. Run `gwt-arch-review` again after implementation to verify improvement
```

## Execution rules

1. **Read-only**: Never modify source code, tests, or configuration
2. **Evidence-based**: Every finding must cite specific file paths and line ranges
3. **Actionable**: Every finding must include a suggested action
4. **Proportional**: Don't flag trivia in a codebase with critical issues — focus attention on what matters most
5. **Repository-agnostic**: This skill works on any codebase, not just gwt
6. **No SPEC dependency**: This skill operates independently of the SPEC pipeline — it reads source code directly
7. **Severity budget**: Report the findings the evidence supports; prioritize the few that matter most.

## Limitations

- Static analysis only — no runtime profiling or performance measurement
- Heuristic-based — findings are informed opinions, not mathematical proofs
- Language-aware for Rust primarily; other languages get structural analysis without deep semantic checks
- Does not replace security audits, performance profiling, or compliance reviews
