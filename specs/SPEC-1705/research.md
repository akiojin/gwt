<!-- GWT_SPEC_ARTIFACT:doc:research.md -->
doc:research.md

# Research: パフォーマンスプロファイリング基盤 (#1705)

## 調査結果 (rev2)

### 1. 現状の profiling で分かること / 分からないこと

- 分かること
  - backend `#[instrument]` span
  - heartbeat watchdog による freeze detection
  - frontend invoke RTT (`tauriInvoke.ts` → `report_frontend_metrics`)
- 分からないこと
  - project open / cold-start restore から hydration 完了までの total time
  - frontend hydration 3 本のどれが遅いか
  - startup path の backend blocking section がどこか

### 2. frontend 側の stable 候補

`App.svelte` の project 起動後は次の 3 本が並列に走る。

- `fetchCurrentBranch`
- `refreshCanvasWorktrees`
- `restoreProjectAgentTabs`

この 3 本が current token に対してすべて settle した時点が、現在の UI 実装における「初期表示が落ち着いた」地点に最も近い。`version_history` prefetch や project index build は background なので stable 条件から除外するのが妥当。

### 3. startup 入口

- Open Project: `openProjectAndApplyCurrentWindow()` → `invoke("open_project")`
- Cold Start Restore: `restoreCurrentWindowSession()` → 内部で `probe_path` と `open_project`

両者を別 metric 名で持てば、ユーザー操作起因と app startup 起因を混同せず比較できる。

### 4. deep trace 対象候補

- `open_project` 本体とその subphase
- `list_worktree_branches_impl`
- `list_worktrees_impl`
- `list_terminals`
- `prefetch_version_history_inner`
- `spawn_background_index`
- issue cache bootstrap/full sync (project open hook 内)

### 5. 実装判断

- `stable` は Frontend Hydrated
- token mismatch は破棄
- deep trace は startup/hydration path に集中
- frontend metrics は既存 `report_frontend_metrics` を schema 拡張して継続利用
