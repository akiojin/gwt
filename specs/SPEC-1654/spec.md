> **🔄 TUI MIGRATION (SPEC-1776)**: This SPEC has been updated for the gwt-tui migration. Agent Canvas is removed. The workspace shell now uses Shell tabs and Agent tabs within the gwt-tui management screen.
> **Canonical Boundary**: `SPEC-1654` は Shell / Agent タブと管理画面の正本である。Assistant 送信制御は `SPEC-1636`、session persistence は `SPEC-1648`、TUI 全体の移行親は `SPEC-1776` が担当する。

# ワークスペースシェル

## Background

- `#1654` is the canonical workspace shell spec.
- gwt-tui はフルスクリーン TUI アプリケーションであり、メイン画面（Shell/Agent ペイン）と管理画面（Ctrl+G で切り替え）の 2 層構成を持つ。
- メイン画面: Shell タブ（Assistant PTY）と Agent タブ（各エージェントの PTY 出力）を横並びまたはタブ切り替えで表示。
- 管理画面: Branches / Issues / PRs / Settings / Logs / SPECs 等の情報タブをフラットなタブバーで切り替え。
- Ref と worktree のドメイン正本は `#1644` に委譲。本 SPEC はシェル構成、セッション管理、タブ切り替えを定義する。
- `#1648` が永続化境界、`#1636` が Assistant の動作境界を担当する。

## User Scenarios

### User Story 1 - Shell/Agent タブがメインの作業面

**Priority**: P0

開発者として、gwt-tui を起動した瞬間に Shell タブ（Assistant）と Agent タブを中心に作業を開始したい。

**Acceptance Scenarios**:

1. **Given** gwt-tui を起動する、**When** メイン画面が表示される、**Then** Shell タブ（Assistant PTY）がアクティブで表示される
2. **Given** Agent が起動されている、**When** Agent タブを選択する、**Then** 該当 Agent の PTY 出力がリアルタイムで表示される
3. **Given** 複数の Agent が稼働中、**When** タブを切り替える、**Then** 各 Agent の PTY 出力が独立して保持される

### User Story 2 - 管理画面でブランチ・Issue を管理

**Priority**: P0

開発者として、Ctrl+G で管理画面に切り替え、Branches タブでブランチ一覧を確認し、worktree を作成したい。

**Acceptance Scenarios**:

1. **Given** メイン画面を表示中、**When** Ctrl+G を押す、**Then** 管理画面に切り替わり Branches タブが表示される
2. **Given** 管理画面の Branches タブ、**When** ブランチを選択して worktree 作成を実行する、**Then** worktree が作成され Agent タブに反映される
3. **Given** 管理画面を表示中、**When** ESC または Ctrl+G を押す、**Then** メイン画面に戻る

### User Story 3 - セッション状態の復元

**Priority**: P1

開発者として、gwt-tui を再起動した後にタブ構成とセッション状態が復元されてほしい。

**Acceptance Scenarios**:

1. **Given** 複数の Shell/Agent タブが開いている、**When** gwt-tui を再起動する、**Then** タブ構成が復元される
2. **Given** 管理画面の最後のアクティブタブが Issues だった、**When** Ctrl+G で管理画面を開く、**Then** Issues タブがアクティブで表示される

## Functional Requirements

- **FR-001**: メイン画面は Shell タブ（Assistant PTY）と Agent タブ（各エージェント PTY）をタブバーで切り替え可能
- **FR-002**: 管理画面は Branches / Issues / PRs / Settings / Logs / SPECs をフラットなタブバーで表示
- **FR-003**: Ctrl+G でメイン画面と管理画面を切り替え
- **FR-004**: Agent 起動時に自動的に Agent タブが追加される
- **FR-005**: worktree 起点で Agent が起動された場合、タブ名に worktree/ブランチ情報を表示する
- **FR-006**: Branches タブは `#1644` の ref/worktree ドメイン正本を利用し local/remote/all を表示
- **FR-007**: セッション状態（タブ構成、アクティブタブ）を `~/.gwt/sessions/` に保存・復元する
- **FR-008**: タブの並び替え・クローズが可能
- **FR-009**: 管理画面の各タブはターミナル全体を使って表示する

## Non-Functional Requirements

- **NFR-001**: セッション復元は best-effort かつ non-blocking
- **NFR-002**: Git/GitHub 等の重い処理は UI スレッドで実行せず tokio async で処理
- **NFR-003**: 起動後 1000ms 以内にメイン画面が操作可能
- **NFR-004**: タブ切り替えは 100ms 以内に完了

## Success Criteria

- **SC-001**: 起動後に Shell タブが表示され、Agent タブが切り替え可能
- **SC-002**: Ctrl+G で管理画面に切り替わり、Branches / Issues / PRs 等のタブが機能する
- **SC-003**: Agent 起動時にタブが自動追加される
- **SC-004**: セッション復元後にタブ構成が再現される
- **SC-005**: 管理画面の各タブがターミナル全体を使って表示される
