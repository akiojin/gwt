### 技術コンテキスト

- 対象:
  - `crates/gwt-core/src/terminal/runner.rs`
  - `crates/gwt-core/src/terminal/pty.rs`
  - `crates/gwt-tauri/src/commands/terminal.rs`
  - `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
  - `gwt-gui/src/lib/components/StatusBar.svelte`
- 非対象: 外部 API 仕様変更

### 実装アプローチ

1. Windows command 正規化を launch/probe/pty で統一する。
2. Launch Agent 実行解決は「事前検出」ではなく「実行時解決」に寄せる。
3. ランナー優先は `bunx`、`npx` は `--yes` を強制。
4. `installed` は UI で常時保持し、実行時失敗に責務を移す。
5. StatusBar から agent 可用性表示を除去する。

### フェーズ

- Phase A: 正規化ヘルパーと回帰テスト
- Phase B: Launch 解決ロジック更新（bunx 優先 / installed 実行時失敗）
- Phase C: UI 仕様整合（installed 常時表示 / StatusBar 簡素化）
- Phase D: 検証と記録更新
