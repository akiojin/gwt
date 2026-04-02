> **Canonical Boundary**: `SPEC-1776` は TUI 再設計の親 SPEC である。`branch list` 中心の UX 方針、常設マルチモード、管理画面の情報設計、各関連 SPEC の統合優先順位だけを定義する。terminal emulation は `SPEC-1541`、interaction policy は `SPEC-1770`、workspace shell は `SPEC-1654`、SPEC workflow は `SPEC-1579`、workspace initialization は `SPEC-1787`、Quick Start は `SPEC-1782`、Issue detail は `SPEC-1354`、Issue search/discovery は `SPEC-1643`、Issue linkage/cache は `SPEC-1714`、local git/worktree は `SPEC-1644`、agent catalog/launch contract は `SPEC-1646`、session persistence は `SPEC-1648`、Codex hooks merge は `SPEC-1786`、Profiles persistence は `SPEC-1542` / `SPEC-1656`、Assistant interrupt semantics は `SPEC-1636`、custom agent UI は `SPEC-1779` を正本とする。

# 旧TUI UX を基準にした ratatui TUI 再設計

## Background

gwt の現行 `gwt-tui` は `native PTY`、hooks、SPEC/Issue integration、session persistence などの backend はすでに持っている。一方で UX は、`v6.30.3` の旧TUIと比べて、以下の点で使い勝手が下がっている。

- 運用の中心が `branch list` ではなく `tab/session surface` に寄りすぎている
- 起動中セッションの見え方が散り、旧TUIのような一目で把握できる感覚が薄い
- `tmux` multi-mode が担っていた複数セッション運用の価値が、`native PTY` 時代の UX に再設計されていない
- 管理画面の情報は tab 形式で整理したいが、初回完成条件と後続機能の境界が曖昧

この SPEC は `SPEC-1776` を「全部入り移行仕様」から親 SPEC へ戻し、全関連 SPEC の正本を尊重したうえで、新しい TUI の UX 方針と初回完成条件を定義する。

## Cross-Spec Scope

| Concern | Canonical SPEC | `SPEC-1776` の役割 |
|---|---|---|
| Parent UX / migration direction | `SPEC-1776` | 旧TUI基準の再設計方針と優先順位を決める |
| Terminal emulation | `SPEC-1541` | `native PTY + vt100` を前提に採用し、詳細契約は委譲する |
| Mouse / keyboard interaction | `SPEC-1770` | 新しい layout / Enter flow に合わせて interaction policy の親条件を渡す |
| Workspace shell / session lifecycle | `SPEC-1654` | `branch list` 中心 + 常設マルチモードへ再編する親方針を渡す |
| Local git / worktree domain | `SPEC-1644` | Branch-first UX でも ref/worktree 正本は変えない |
| Agent catalog / launch contract | `SPEC-1646` | Wizard と launch flow が参照する agent contract を維持する |
| Session persistence | `SPEC-1648` | `1ブランチ = Nセッション` でも persistence owner は維持する |
| SPECs tab | `SPEC-1777` | parent navigation と launch entry を合わせる |
| Quick Start | `SPEC-1782` | Branches 起点の Quick Start を維持し、新しい Enter flow に接続する |
| Hooks merge | `SPEC-1786` | Codex hooks confirm / embed flow を parent launch flow に組み込む |
| SPEC workflow / storage | `SPEC-1579` | local SPEC artifact を正本として扱う前提を維持する |
| Workspace initialization | `SPEC-1787` | 初期導線や SPEC-first workflow の責務を再確認する |
| Issue detail | `SPEC-1354` | Issues tab の detail contract は child 正本に従う |
| Issue discovery / search | `SPEC-1643` | Issues list/search の source of truth を維持する |
| Issue linkage / cache | `SPEC-1714` | Branches / Issues で同じ linkage source を使う |
| Profiles persistence | `SPEC-1542`, `SPEC-1656` | `Profiles = Env profiles` の保存契約を再利用する |
| Assistant behavior | `SPEC-1636` | Shell/Assistant の interrupt/queue semantics は上書きしない |
| Custom agent UI | `SPEC-1779` | 初回完成条件では後続だが canonical owner は維持する |

## Embedded Workflow Coverage

- `gwt-spec-ops`、`gwt-spec-implement`、`gwt-spec-plan`、`gwt-spec-tasks`、`gwt-spec-analyze` などの埋め込み workflow skill 群は `SPEC-1579` が正本である
- `SPEC-1776` はそれらを再定義しない。制約するのは `SPECs` / `Issues` / `Branches` からの launch entry、viewer navigation、branch-first UX だけである
- `SPEC-first workflow` の起点や workspace initialization は `SPEC-1787` が正本であり、`SPEC-1776` はその entry surface が新しい管理画面タブと矛盾しないことだけを担保する

## Product Direction

- 旧TUIの価値は `コード` ではなく `UX` にある。したがって `gwt-cli` を単純に戻すのではなく、現行 `gwt-core` 上で旧TUIの使いやすさを再構成する
- `tmux` 依存は廃止する。ただし、旧TUIの multi-mode が持っていた「複数セッションを走らせて、すぐ切り替えられる」価値は維持する
- 管理画面は旧TUIへ戻さず、現行TUIの `tabbed management workspace` を採用する
- `Profiles` は一般的な個人設定ではなく `Env profiles` とする。主責務は環境変数セットの作成・編集・削除・切替である
- 旧TUIにあった `AI summary` は今回は後続とし、初回完成条件から外す

## User Stories

### US1 - Branch list を中心に運用したい (P0)

As a developer, I want the main entry point of gwt to feel like the old TUI branch list again, so that I can understand branch state and active work without first navigating tabs.

### US2 - tmux なしで複数セッションを常時運用したい (P0)

As a developer, I want to keep multiple agent or shell sessions alive without tmux, so that I can monitor and switch across them in a single permanent multi-session workspace.

### US3 - 1ブランチに複数セッションを持ちたい (P0)

As a developer, I want a single branch to host multiple sessions, so that I can run several agents on the same work item while still entering from the branch list.

### US4 - 管理画面はタブ形式で整理したい (P0)

As a developer, I want Branches / SPECs / Issues / Profiles to remain separate tabs in the management workspace, so that navigation stays structured while the session surface stays execution-focused.

### US5 - 現行 integration を落としたくない (P0)

As a developer, I want the rebuilt TUI to keep native PTY, hooks, SPEC integration, Issue integration, and Quick Start, so that the redesign does not regress current gwt capabilities.

## Acceptance Scenarios

1. gwt 起動時、最初に見える主要画面は `Branches` である
2. ブランチ一覧の各行には、そのブランチ上の起動中セッション件数が表示される
3. セッションが存在しないブランチで `Enter` すると `Wizard` が開く
4. セッションが 1 件だけ存在するブランチで `Enter` すると、そのセッションへ直接入る
5. セッションが複数存在するブランチで `Enter` すると、`既存へ入る / 追加起動 / フルWizard` を選べる
6. セッション領域は `4件以上` を前提に均等グリッドで表示できる
7. フォーカス中セッションを最大化でき、再度トグルで均等グリッドへ戻れる
8. 最大化時はタブ切替で他セッションへ移動できる
9. 管理画面を開閉しても、セッション領域は直前レイアウトへ戻る
10. `SPECs` と `Issues` はどちらも一覧・詳細・起動導線まで初回から使える
11. `Profiles` では env profile の作成・編集・削除・切替と、OS 環境変数参照・置換ができる
12. `Quick Start`、hooks confirm、skill registration、native PTY は引き続き動作する

## Edge Cases

- 1 ブランチに多数のセッションがある場合でも、ブランチ一覧は件数表示だけに留めて横幅を圧迫しない
- セッション数が 4 を超える場合でも、均等グリッドから最大化へ切り替えて文脈を失わずに操作できる
- 管理画面を開いたままでも、起動中セッションが落ちず、閉じたときに直前レイアウトへ戻る
- Quick Start が有効なブランチで追加起動した場合でも、既存セッション選択と新規起動が競合しない
- `Profiles` で OS 環境変数を参照する値が存在しない場合、欠落が分かる形で編集できる
- child SPEC が detail contract を持つ機能は、親 SPEC の UI 方針変更で勝手に上書きしない

## Functional Requirements

### Parent Governance

- FR-001: `SPEC-1776` は parent UX spec として振る舞い、detail contract は child SPEC に委譲する
- FR-002: 実装前に `旧TUI / 現行 gwt-tui / 現行 gwt-core / 新目標` の比較マトリクスを持つ
- FR-003: child SPEC の正本境界を壊す変更は、該当 child SPEC の同期タスクとして扱う
- FR-004: `gwt-spec-ops` などの embedded workflow skill contract は `SPEC-1579` / `SPEC-1787` を正本とし、`SPEC-1776` では launch/view/navigation への影響だけを定義する

### Branches and Session Workspace

- FR-010: `Branches` は常に第一入口である
- FR-011: ブランチ一覧の各行は `セッション件数のみ` を表示する
- FR-012: `1ブランチ = Nセッション` を許可する
- FR-013: セッションが無いブランチの `Enter` は `Wizard` を開く
- FR-014: セッションが 1 件だけあるブランチの `Enter` は、そのセッションを開く
- FR-015: セッションが複数あるブランチの `Enter` は `既存へ入る / 追加起動 / フルWizard` を提示する
- FR-016: セッション領域は `常設マルチモード` とし、`4件以上` を前提に均等グリッドを扱う
- FR-017: フォーカス中セッションはキーボード中心で最大化トグルできる
- FR-018: 最大化時はタブ切替でセッション間を移動できる
- FR-019: `hidden pane` 概念は廃止し、表示制御は `均等グリッド / 最大化` で扱う
- FR-019a: 管理画面を閉じたとき、セッション領域は直前レイアウトへ復帰する

### Management Workspace

- FR-020: 管理画面は `tabbed management workspace` とする
- FR-021: 初回完成条件の管理画面タブは `Branches / SPECs / Issues / Profiles` とする
- FR-022: `Settings / Logs / Versions` は後続フェーズへ回す

### Integrations to Preserve

- FR-030: terminal emulation は `SPEC-1541` の契約を維持する
- FR-031: keyboard / mouse interaction は `SPEC-1770` の契約を維持しつつ、新 layout に同期する
- FR-032: Branches 起点の Quick Start は `SPEC-1782` に従って維持する
- FR-033: Issue linkage と exact cache は `SPEC-1714` の source of truth を使う
- FR-034: local SPEC artifact の正本は `SPEC-1579` / `SPEC-1787` に従う
- FR-035: Codex hooks confirm / merge flow は `SPEC-1786` に従う
- FR-036: 現行 `native PTY`、hooks、skill registration は削除しない

### Profiles

- FR-040: `Profiles` は `Env profiles` を意味する
- FR-041: `Profiles` タブは env profile の作成・編集・削除・切替を提供する
- FR-042: `Profiles` は旧TUI相当の `OS環境変数参照・置換` を提供する
- FR-043: persistence contract は `SPEC-1542` / `SPEC-1656` に従う

### Deferred Scope

- FR-050: `AI summary` は今回は実装対象外とする
- FR-051: `Settings / Logs / Versions` は parent comparison matrix に載せるが、初回完成条件には含めない
- FR-052: custom agent UI の再設計は `Settings` 復帰フェーズで扱う

## Success Criteria

- SC-001: 旧TUIに近い `branch list` 中心の運用感が戻る
- SC-002: `tmux` なしで複数セッションを同時運用できる
- SC-003: `SPECs / Issues / Profiles` の初回必須タブが独立して成立する
- SC-004: `native PTY`、Quick Start、hooks、SPEC/Issue integration が維持される
- SC-005: parent / child SPEC の責務境界が明確で、`SPEC-1776` が他仕様を上書きしない
