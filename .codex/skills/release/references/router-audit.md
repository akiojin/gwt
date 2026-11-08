# Router constraint audit

- Relaxed: Load release mechanics only when the requested release reaches that phase. A completion goal is optional and requires an explicit user request; monitoring is still required.
- Retained: Explicit version/changelog approval, workflow-owned frozen snapshots, major-version opt-in, scoped bypass cleanup, reference-only release bodies, bounded retries, and publication proof.
- Distribution: This repository-local Codex skill remains outside the bundled dual-provider assets; no Claude mirror is introduced.
