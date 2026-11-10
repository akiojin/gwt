# Native conversation record fixtures

These excerpts were derived on 2026-11-10 from a managed Claude Code 2.1.322
PM session and a managed Codex 0.167.0 session. Records retain their source
order and the fields consumed by `pm_conversation`; unrelated metadata is
omitted. They are anonymized field-shape regressions, not complete raw-session
replays or evidence of a rendered chat UI.

User/assistant text, tool payloads, paths and identities are replaced with fixed
values. The two Codex user text blocks preserve the observed block boundaries.
The Claude scheduled tick retains its public generated text and human-origin
metadata; its delivery UUID is replaced and its body digest recomputed. A tick
submitted through the PTY can have human-origin metadata, so that field alone
must not make it visible. The tool-output replacements contain ANSI and terminal
decoration to make accidental inclusion apparent.

The expected projection contains only the user question and assistant reply.
Claude attachment records, API request blobs, thinking/tool blocks, tool results,
meta prompts, Stop feedback, hook summaries, scheduled ticks and task notifications
are excluded. Codex developer messages, tool calls/results, reasoning and duplicate
event records are excluded. This text-only contract does not establish attachment
support, native choice-answer capture or the parent chat UI acceptance criteria.
