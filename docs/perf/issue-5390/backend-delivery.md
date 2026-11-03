# Issue #5390: backend delivery scope

This slice ships three independently testable backend corrections while keeping
the released frontend unchanged:

- Empty execution observations return before runtime inventory or Git reads.
- Private HTTP connections do not outlive the short-lived caller runtime that
  drove them; requests are not replayed to recover from a stale pooled socket.
- Candidate loading and autonomous progress share one open-PR inventory Result
  within a scan. Failed reads retry in the next scan, and unknown/stale phases
  and launch safeguards remain intact.

The typed browser-check preflight changes are verification tooling. Their
contract tests do not claim a completed browser run.

The delivered Issue #1935 dependency is integrated from develop, including its
source-day provenance contract and consumer tests. Directory-entry I/O errors
continue to propagate instead of silently dropping entries. Earlier donor
`0869655a3` results remain historical; this merge is verified independently.

The UI telemetry rehosting and sparkline changes are preserved at
`0df66c0e18be4622519b8d194b23771eb4520f84`. They are excluded from this backend
delivery and remain a follow-up under Issue #5390. The parent Mac input/scroll,
fleet duration, CPU/RSS, installed behavior, and headed dark/light acceptance
remain unproven. This PR does not close Issue #5390.

Local acceptance uses actual Linux Cargo tests through WSL with an isolated
target. Each command declares that same physical target to native artifact
coordination, and the Linux operational binary is restored in that target after
tests. Windows compilation, strict Clippy, and PowerShell helper contracts stay
native. Required CI supplies the full Rust and Windows test matrices and scoped
coverage gates. No Windows operational candidate is replaced by Linux outputs.

The original Windows candidate pair, recovery packet, and old verification
record retain their four recorded SHA-256 values. No shared GUI/daemon restart,
owner reassignment, Job registration, measurement guard, or Ready gate change is
part of this delivery.

Rollback can revert this backend slice without applying the preserved UI
changes. Follow-up UI delivery starts from the saved forward patch, reconciles
the current frontend, and requires its own headed verification.
