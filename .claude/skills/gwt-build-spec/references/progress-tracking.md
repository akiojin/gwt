# Progress Tracking Reference

This reference applies to **SPEC mode only**. Standalone mode does not use SPEC artifacts.

## tasks.md update format

When a task is completed, update `tasks.md` to reflect completion:

```markdown
- [x] Task description (completed)
- [ ] Task description (remaining)
```

Rules:

- Update tasks.md after each task slice is verified, not before.
- Do not mark tasks complete until the implementation passes verification.
- Do not batch-mark multiple tasks; update incrementally.
- アーティファクト操作には `gwt issue spec <N> --edit tasks -f <file>` を使用する。

## 進捗追跡

tasks セクションのチェックボックスを `gwt issue spec <N> --edit tasks -f <file>` で更新する。

### ルール

- 事実に基づき、インクリメンタルに更新する。
- コードで裏付けられていない完了を主張しない。
- 検証コマンドの出力（pass/fail カウント）を含める。
- `tasks.md` の具体的なタスク ID を参照する。

## Completion markers

### Task-level completion

A task is complete when:

1. The failing test was written (Red)
2. The implementation makes it pass (Green)
3. Refactoring is done with all tests green
4. Verification commands pass (`cargo test`, `cargo clippy`, `cargo fmt`)

### Phase-level completion

A phase is complete when all tasks in that phase are marked `[x]` and verified.

### SPEC-level completion

A SPEC is complete only after the Completion Gate (Phase 5) passes.
Do not mark the SPEC complete during progress tracking; that is the gate's responsibility.

## False completion detection

Watch for these signals that indicate premature completion claims:

- Tasks marked `[x]` but related tests are not in the codebase
- tasks セクションが "all tests pass" と主張しているが `cargo test` の出力が失敗を示している
- tasks セクションの受け入れチェックが "accepted" だが動作が未実装
- Task marked complete but the file listed in the task was not modified

If any of these are detected, revert the completion marker and return to implementation.
