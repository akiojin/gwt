### Phase 1: 基盤（セクションヘッダー・ラベルスタイル・タブ改名）

1. SettingsPanel.svelte にセクションヘッダー CSS を追加
2. フィールドラベルを sentence case に変更
3. タブを Appearance → General に改名、タブ順序変更
4. `.divider` を廃止

### Phase 2: タブ再配置

5. Terminal font size/family を General → Terminal タブに移動
6. Terminal タブを常時表示に変更（Shell セクションのみ条件表示）
7. General タブをセクション分割（DISPLAY / LANGUAGE / GIT / MAINTENANCE）
8. Terminal タブをセクション分割（FONT / SHELL）

### Phase 3: Profiles タブ再構成

9. ConfirmDialog.svelte を作成（汎用確認ダイアログ）
10. CreateProfileDialog.svelte を作成
11. Profiles タブのヘッダー領域を実装（ドロップダウン + Delete + New 横並び）
12. 削除確認を ConfirmDialog に接続
13. AI Configuration を単一カラムに変更
14. ラベル変更（Profile Language → AI response language）

### Phase 4: Voice Input 条件表示

15. Enable OFF 時にフィールドを非表示にする
16. Voice Input 内のセクション分割（HOTKEYS / RECOGNITION）

### Phase 5: Session Summary 廃止

17. UI から Session Summary チェックボックスを削除
18. Rust 側 summary_enabled フィールドを削除（後方互換対応含む）

### Phase 6: 検証

19. 全設定の Save/Load 動作確認
20. `cargo clippy` / `cargo test` / `svelte-check` / `vitest` パス確認
21. ラベル変更の一覧レビュー
