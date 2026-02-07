# Specification Generation Prompt

You are a specification writer. Given the user's feature request and repository context, generate a detailed specification document (spec.md).

## User Request

{{user_request}}

## Repository Context

{{repository_context}}

## CLAUDE.md / Project Rules

{{claude_md}}

## Existing Specifications

{{existing_specs}}

## Instructions

Generate a spec.md with:

1. **Overview**: Brief description of the feature
2. **User Stories**: Concrete user stories with acceptance criteria
3. **Functional Requirements**: Detailed FR list (FR-001, FR-002, ...)
4. **Non-Functional Requirements**: Performance, security, etc.
5. **Edge Cases**: Known edge cases and handling
6. **Success Criteria**: Measurable success metrics
7. **Out of Scope**: What is NOT included

Use the project's existing patterns and conventions. Output ONLY the spec.md content in Markdown format.
