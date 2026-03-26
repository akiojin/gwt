### Phase 1: 基盤（セクションヘッダー・ラベルスタイル・タブ改名）

1. SettingsPanel.svelte にセクションヘッダー CSS を追加
2. フィールドラベルを sentence case に変更
3. タブを Appearance → General に改名、タブ順序変更
4. `.divider` を廃止

### Phase 2: タブ再配置

1. Terminal font size/family を General → Terminal タブに移動
2. Terminal タブを常時表示に変更（Shell セクションのみ条件表示）
3. General タブをセクション分割（DISPLAY / LANGUAGE / GIT / MAINTENANCE）
4. Terminal タブをセクション分割（FONT / SHELL）

### Phase 3: Profiles タブ再構成

1. ConfirmDialog.svelte を作成（汎用確認ダイアログ）
2. CreateProfileDialog.svelte を作成
3. Profiles タブのヘッダー領域を実装（ドロップダウン + Delete + New 横並び）
4. 削除確認を ConfirmDialog に接続
5. AI Configuration を単一カラムに変更
6. ラベル変更（Profile Language → AI response language）

### Phase 4: Voice Input 条件表示

1. Enable OFF 時にフィールドを非表示にする
2. Voice Input 内のセクション分割（HOTKEYS / RECOGNITION）

### Phase 5: Session Summary 廃止

1. UI から Session Summary チェックボックスを削除
2. Rust 側 summary_enabled フィールドを削除（後方互換対応含む）

### Phase 6: 検証

1. 全設定の Save/Load 動作確認
2. `cargo clippy` / `cargo test` / `svelte-check` / `vitest` パス確認
3. ラベル変更の一覧レビュー
