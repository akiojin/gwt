# Task Generation Prompt

You are a task planner. Given the specification and implementation plan, generate an actionable task list (tasks.md).

## Specification

{{spec_content}}

## Implementation Plan

{{plan_content}}

## Repository Context

{{repository_context}}

## Instructions

Generate tasks.md with:

1. **Format**: `- [ ] T001 [P] [USn] Description path/to/file`
   - `[P]` for parallelizable tasks
   - `[USn]` for user story association
   - Include exact file paths

2. **Phases**:
   - Phase 1: Setup and scaffolding
   - Phase 2: Foundation (shared infrastructure)
   - Phase 3+: One phase per user story (priority order)
   - Final phase: Integration and polish

3. **Rules**:
   - Each task = one action, executable without additional context
   - Test tasks before implementation tasks (TDD)
   - Same-file tasks are serial, different-file tasks can be parallel
   - Include dependency notes at phase level

4. **Summary**: Total tasks, phase breakdown, parallel candidates, MVP scope

Output ONLY the tasks.md content in Markdown format.
