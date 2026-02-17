# 実装計画: Worktree一覧のエージェント状態アニメーション

**仕様ID**: `SPEC-b80e7996`
**作成日**: 2026-02-16

## 実装フェーズ

### Phase 1: データモデル拡張 (バックエンド)

**目的**: BranchInfo / WorktreeInfo に `agent_status` フィールドを追加し、セッションから読み取った AgentStatus をフロントエンドに伝達する。

**変更ファイル**:

- `crates/gwt-core/src/config/session.rs` — `check_idle_timeout` をブランチ一覧取得時に呼び出し
- `crates/gwt-tauri/src/commands/` — `list_branches` / `list_worktrees` のレスポンスに `agent_status` を追加
- `gwt-gui/src/lib/types.ts` — `BranchInfo` と `WorktreeInfo` に `agent_status` フィールド追加

**設計判断**:

- `is_agent_running` は後方互換のため残す。`agent_status === "running"` と `is_agent_running === true` は同値
- `AgentStatus::Unknown` はフロントエンドで `"unknown"` として伝達。タブが存在するが状態が不明な場合

### Phase 2: fs 監視 (バックエンド)

**目的**: `~/.gwt/sessions/` ディレクトリの変更を監視し、Tauri イベントでフロントエンドに通知する。

**変更ファイル**:

- `crates/gwt-tauri/src/` — fs watcher モジュール追加（`notify` crate 利用）
- `Cargo.toml` — `notify` crate 依存追加

**設計判断**:

- `notify::RecommendedWatcher` を使用し、プラットフォーム最適なバックエンドを自動選択
- 500ms debounce: `notify-debouncer-mini` を使用
- Tauri の `setup` フック内で watcher を起動し、`AppHandle` 経由で `agent-status-changed` イベントを emit
- watcher はアプリケーション終了時に自動停止（Drop）

### Phase 3: フロントエンド状態同期

**目的**: `agent-status-changed` イベントの受信でブランチ一覧を再取得し、ポーリングフォールバックも実装する。

**変更ファイル**:

- `gwt-gui/src/lib/components/Sidebar.svelte` — イベントリスナー追加、ポーリングタイマー追加

**設計判断**:

- `listen("agent-status-changed", callback)` で変更を受信
- ポーリングは5秒間隔。イベント受信時はポーリングタイマーをリセット（不要な重複取得を防止）
- ブランチ一覧の再取得は既存の `loadBranches()` を再利用

### Phase 4: インジケーターUI (フロントエンド)

**目的**: 2層構造のインジケーターを実装し、インデント問題を修正する。

**変更ファイル**:

- `gwt-gui/src/lib/components/Sidebar.svelte` — テンプレート・CSS 修正
- `gwt-gui/src/lib/components/CleanupModal.svelte` — 同様のインジケーター対応

**設計判断**:

- **全行予約幅**: 全ブランチ行に 12px の固定幅スペースを確保。エージェントタブがない行は空白スペース
- **レイヤー1（静的）**: タブが開いているブランチに `●` (small filled circle) を dim cyan で表示
- **レイヤー2（アニメーション）**: AgentStatus が Running のブランチに CSS `pulse` アニメーション（opacity の明滅）を適用
- **CSS keyframes**:

```css
@keyframes agent-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}
```

- **`prefers-reduced-motion`**: アニメーション無効時は `@` フォールバック（既存動作維持）

### Phase 5: Codex/他エージェントの状態推測

**目的**: Hook を持たないエージェントの状態をペイン出力解析とプロセス生存確認で推測する。

**変更ファイル**:

- `crates/gwt-core/src/config/session.rs` — 推測ロジック追加（またはモジュール分離）
- `crates/gwt-tauri/src/commands/` — ブランチ一覧取得時にペイン状態も参照

**設計判断**:

- プロンプトパターン検出: 末尾行が `>`, `→`, `$`, `>>>`, `Input:` 等にマッチ
- プロセス生存確認: PID が存在する場合は `kill -0` 相当で確認
- 60秒アイドル: `last_activity_at` ベース（既存の `check_idle_timeout`）
- 推測精度は Hook ベースより低いことを前提に、最善を尽くす

## 依存関係

```text
Phase 1 (データモデル)
  ↓
Phase 2 (fs監視) ← 並行可能 → Phase 3 (フロントエンド同期)
  ↓                              ↓
Phase 4 (インジケーターUI) ← Phase 1, 3 完了後
  ↓
Phase 5 (状態推測) ← Phase 1, 4 完了後
```

## リスクと軽減策

| リスク | 影響 | 軽減策 |
|--------|------|--------|
| `notify` crate の macOS FSEvents が sessions dir 外の変更も通知 | 不要なリフレッシュ | debounce + sessions dir 内のファイルのみフィルタ |
| Codex のプロンプトパターンが変わる | 誤った状態推測 | パターンを設定可能にするか、フォールバックで Running 扱い |
| Hook の高頻度発火（PreToolUse/PostToolUse が連続） | リフレッシュの嵐 | 500ms debounce で統合 |
