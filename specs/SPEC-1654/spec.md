> **Canonical Boundary**: `SPEC-1654` は rebuilt TUI における `Branches` 起点の session workspace と management workspace の正本である。parent UX と優先順位は `SPEC-1776`、terminal behavior は `SPEC-1541`、interaction policy は `SPEC-1770`、session persistence は `SPEC-1648`、Assistant semantics は `SPEC-1636` が担当する。

# ワークスペースシェル

## Background

- 旧TUIの価値は `branch list` を中心に置いた運用感にあった
- 現行 `gwt-tui` は `native PTY` を持つが、session surface が `tab-first` に寄りすぎている
- 新しい shell model では、`Branches` を primary entry に戻しつつ、`tmux` に依存しない `permanent multi-mode` を session workspace に採用する
- 1 ブランチに複数 session を持てる前提とし、通常時は `equal grid`、集中時は `maximize + tab switch` で扱う
- 管理画面は tabbed workspace を採用し、初回完成条件のタブは `Branches / SPECs / Issues / Profiles` とする

## User Stories

### US-1: Branches から作業を始めたい (P0)

開発者として、gwt を開いたらまず `Branches` から現在の作業状況を把握し、そこから session に入ったり launch したりしたい。

### US-2: tmux なしで複数 session を同時に運用したい (P0)

開発者として、Agent/Shell session を tmux なしで複数走らせ、同時に監視し、必要なものを拡大して扱いたい。

### US-3: 1 ブランチ上の複数 session を自然に扱いたい (P0)

開発者として、同じブランチで複数の session を持てるようにしつつ、`Enter` から既存 session と追加起動のどちらにも自然に入れるようにしたい。

### US-4: 管理画面は独立タブで見たい (P0)

開発者として、`Branches / SPECs / Issues / Profiles` を独立タブとして扱い、session workspace と役割分担したい。

### US-5: session 状態を best-effort で復元したい (P1)

開発者として、再起動後も session metadata と last layout を可能な範囲で復元したい。

## Acceptance Scenarios

1. gwt 起動時、最初に見える primary entry は `Branches` である
2. ブランチ行にはそのブランチ上の起動中 session 件数が表示される
3. session がないブランチで `Enter` すると launch flow に入る
4. session が 1 件だけあるブランチで `Enter` すると、その session が開く
5. session が複数あるブランチで `Enter` すると、既存 session 選択または追加起動を選べる
6. session workspace は 4 件以上を前提に通常時 `equal grid` で表示できる
7. focus 中 session を maximize でき、再度トグルで grid へ戻れる
8. maximize 時は tab switch で他 session へ移動できる
9. 管理画面を開閉しても、session workspace は直前レイアウトへ戻る
10. 管理画面は `Branches / SPECs / Issues / Profiles` の独立タブを持つ
11. session metadata は再起動後の restore 候補として再利用できる

## Functional Requirements

- FR-001: `Branches` は workspace shell の primary entry でなければならない
- FR-002: ブランチ行は `session count` を表示しなければならない
- FR-003: `1ブランチ = Nセッション` を許可しなければならない
- FR-004: ブランチ `Enter` は `no session / one session / many sessions` の 3 分岐を持たなければならない
- FR-005: `many sessions` の場合は selector を表示し、既存 session に入るか追加起動へ進めなければならない
- FR-006: session workspace は `equal grid` を通常形としなければならない
- FR-007: focus session の maximize toggle を持たなければならない
- FR-008: maximize 時は tab switch で session を切り替えられなければならない
- FR-009: `hidden pane` 概念は持たず、表示制御は `grid / maximize` だけで扱わなければならない
- FR-010: 管理画面は `Branches / SPECs / Issues / Profiles` の tabbed workspace を持たなければならない
- FR-011: 管理画面を閉じたとき、session workspace は直前レイアウトへ復帰しなければならない
- FR-012: session persistence の canonical owner は `SPEC-1648` とし、本 SPEC は shell-side restore behavior だけを定義する
- FR-013: terminal interaction detail は `SPEC-1541` / `SPEC-1770` に委譲し、本 SPEC では shell topology と navigation のみを定義する

## Non-Functional Requirements

- NFR-001: session grid への切替や maximize toggle は 100ms 以内に完了する
- NFR-002: session restore は best-effort かつ non-blocking である
- NFR-003: shell model は local git/worktree truth を重複実装せず、`SPEC-1644` の projection を消費する

## Success Criteria

- SC-001: rebuilt TUI の shell model が `branch-first` へ戻る
- SC-002: tmux なしで複数 session を安定運用できる
- SC-003: 1 ブランチ上の複数 session を自然に扱える
- SC-004: 管理画面タブと session workspace の責務が明確に分離される
