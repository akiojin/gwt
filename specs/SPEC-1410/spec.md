### 背景

Windows 環境で gwt のターミナルタブ間（agent-1 → term-1 など）を切り替えると、1フレームの背景フラッシュが発生する。ネイティブメニュー領域までターミナル表示が被るように見える。

### Root Cause

`MainArea.svelte` の `isTerminalTabVisible()` が `visibleTerminalTabId`（`$effect` で非同期更新）に依存。
タブ切り替え時に `activeTerminalTabId`（`$derived`、同期）が先に変わり、`visibleTerminalTabId` は `$effect` 実行まで旧値のまま → 1フレーム全ターミナル非表示 → 背景フラッシュ。

### User Scenario

- **P0**: Windows 環境で初期化済みターミナルタブ間を切り替えた際、1フレームの背景フラッシュが発生する

### Acceptance Scenarios (Tests)

- S-1: 初期化済みターミナルタブ間の切り替えで、rerender 直後に常に1つの `.terminal-wrapper.active` が存在すること（ギャップゼロ）
- S-2: 未初期化ターミナルタブへの切り替えでは、既存のフォールバック動作（非表示 → 120ms後に表示）が維持されること

### Functional Requirements

- FR-1: `isTerminalTabVisible()` が初期化済みターミナル（`terminalReadyTabIds` に含まれるタブ）に対して同期的に `true` を返す

### Success Criteria

- SC-1: 既存テスト全通過
- SC-2: 新規テスト（S-1 検証）通過
- SC-3: svelte-check 通過
