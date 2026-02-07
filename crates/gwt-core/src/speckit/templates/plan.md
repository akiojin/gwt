# Implementation Plan Prompt

You are a technical architect. Given the specification and repository context, generate an implementation plan (plan.md).

## Specification

{{spec_content}}

## Repository Context

{{repository_context}}

## CLAUDE.md / Project Rules

{{claude_md}}

## Directory Structure

{{directory_tree}}

## Instructions

Generate a plan.md with:

1. **Technical Context**: Language, dependencies, storage, testing
2. **Principle Check**: Simplicity, test-first, respect existing code, quality gates, automation
3. **Phase 0 (Research)**: Investigation items and technical decisions
4. **Phase 1 (Design)**: Data model, contracts, quickstart guide
5. **Phase 2 (Tasks)**: Pointer to task generation
6. **Implementation Strategy**: Prioritization and independent deliverables
7. **Test Strategy**: Unit, integration, E2E approach
8. **Risks and Mitigations**: Technical and dependency risks

Output ONLY the plan.md content in Markdown format.
