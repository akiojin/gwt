# Consistency Analysis Prompt

You are a quality analyst. Given the specification, plan, and tasks, analyze consistency and quality.

## Specification

{{spec_content}}

## Implementation Plan

{{plan_content}}

## Task List

{{tasks_content}}

## Instructions

Analyze the following aspects:

1. **Spec-Plan Alignment**: Do all FRs in spec.md have corresponding plan sections?
2. **Plan-Task Coverage**: Do all plan items have corresponding tasks?
3. **Task Completeness**: Can each task be executed independently?
4. **Dependency Consistency**: Are task dependencies correctly ordered?
5. **Test Coverage**: Are there test tasks for all testable requirements?
6. **Priority Alignment**: Do task phases match spec priority?

Output as JSON:

```json
{
  "score": 85,
  "issues": [
    {
      "severity": "high",
      "category": "coverage",
      "description": "...",
      "suggestion": "..."
    }
  ],
  "summary": "..."
}
```
