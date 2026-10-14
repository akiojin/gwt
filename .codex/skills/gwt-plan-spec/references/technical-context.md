## Phase 1: Technical Context

Establish the implementation landscape before designing.

1. **Establish source context.** Read the current SPEC and any applicable constitution material not already available in context.
   Refuse to continue only when `spec.md` is missing or a user decision still blocks planning.

2. **Run Board active-claim preflight.** Once the target SPEC number is known,
   read the current Board with JSON operation `board.show`.
   - Look for active `claim` entries from another session that mention the same
     owner (`#<N>` or `SPEC-<N>`) or the planning phase being refreshed.
   - If a matching claim exists, pause before changing artifacts and present the
     conflict: join that session with a Board handoff request, discuss a
     disjoint planning split, or continue only after the user explicitly accepts
     duplicate risk.
   - Intentional parallel planning is allowed only when artifact ownership is
     disjoint. Post a fresh Board `claim` with a `Boundary:` line naming the
     sections/files owned by this session before editing.
   - Acceptance scenario: Given another session has an active Board claim for
     `SPEC-1935 plan refresh`, when `gwt-plan-spec` starts for SPEC-1935, then
     the preflight reports the claim and requires user confirmation before
     `plan`, `tasks`, or related artifacts are edited.

3. **Identify affected scope.** List files, modules, services, and external constraints
   that the work touches. Record assumptions explicitly.

4. **Run the constitution check.** Evaluate the work against every rule in
   `.gwt/memory/constitution.md`. If a rule is violated, either redesign or record the
   justification in `Complexity Tracking`.

5. **Answer the Required Plan Gates** from the constitution:
   - What files/modules are affected?
   - What constitution constraints apply?
   - Which risks or complexity additions are accepted, and why?
   - How will the acceptance scenarios be verified?
