# Discussions

This file is the canonical gwt discussion log. Entries are updated in place while active and indexed by the `discussions` semantic scope.

## 2026-05-23 — Workspace terminology and durable discussions

Status: active
Topics: workspace, work, discussion, semantic-search
Related SPECs: #2359
Related Works:
Promoted To:

Summary:
Workspace の意味が分かりにくい。Branch は作業空間、SPEC は仕様、Work は永続する作業単位として整理する。議論フェーズは Work ではなく Discussion として扱い、memory と同じくファイルへ保存し、セマンティック検索できるようにする。

Decisions:
- Discussion is not Work.
- Work remains durable until completion and can be persisted with completed status.
- A Work can cover multiple SPECs, and concrete tasks may be undecided at creation time.
- Past Work and Discussion records should be semantically searchable and can surface similar candidates during conversation.

Open Questions:
- Workspace という語を UI 上で Project State / Work / Discussion / Branch とどう分けると直感的か。

Next:
実データとして discussion log を保存し、discussions semantic index で検索できることを確認する。
