# TDD記録: 設定画面フォントファミリー選択

**仕様ID**: `SPEC-7b36bdc7`  
**更新日**: 2026-02-20

## 対象

- `SettingsPanel` のフォントファミリー選択と保存
- `TerminalView` のフォントファミリー初期化とイベント反映
- Playwright による保存・Closeロールバックの統合確認

## RED（先に失敗条件を固定）

1. `SettingsPanel.test.ts` にフォントファミリー保存検証を追加し、未実装時に失敗を確認
2. `SettingsPanel.test.ts` に Close 時ロールバック検証を追加し、未実装時に失敗を確認
3. `TerminalView.test.ts` に初期フォント適用・イベント更新検証を追加し、未実装時に失敗を確認
4. Playwright にフォント保存・ロールバックのケースを追加し、未実装時に失敗を確認

## GREEN（実装して通す）

1. Rust 設定モデルへ `ui_font_family` / `terminal_font_family` を追加
2. Settings UI のフォントセレクトと即時プレビュー反映を実装
3. Save/Close の保存・復元挙動を実装
4. 起動時フォント復元（`main.ts`）と terminal event 連携を実装
5. Unit / E2E テストを実行して GREEN を確認

## 実行ログ

- `cargo test -p gwt-core test_appearance_ -- --test-threads=1`
- `cargo test -p gwt-tauri test_settings_data_`
- `pnpm test src/lib/components/SettingsPanel.test.ts src/lib/terminal/TerminalView.test.ts`
- `pnpm exec svelte-check --tsconfig ./tsconfig.json`
- `pnpm exec playwright test e2e/windows-shell-selection.spec.ts --grep "font"`
- `pnpm exec playwright test e2e/windows-shell-selection.spec.ts`

## 実行結果（2026-02-20）

- `cargo test -p gwt-core test_appearance_ -- --test-threads=1`:
  - `test_appearance_default`
  - `test_appearance_save_load`
  - `test_appearance_backward_compat`
  - 3件すべて PASS
- `cargo test -p gwt-tauri test_settings_data_`:
  - `test_settings_data_round_trip` 含む 7件 PASS
- `pnpm test src/lib/components/SettingsPanel.test.ts src/lib/terminal/TerminalView.test.ts`:
  - 2ファイル / 63テスト PASS
- `pnpm exec svelte-check --tsconfig ./tsconfig.json`:
  - 0 errors / 0 warnings
- `pnpm exec playwright test e2e/windows-shell-selection.spec.ts --grep "font"`:
  - `saves UI and terminal font families from Appearance tab` PASS
  - `restores font family preview on Close without saving` PASS
- `pnpm exec playwright test e2e/windows-shell-selection.spec.ts`:
  - 6件すべて PASS（既存 shell selection ケースの回帰なし）

## 備考

- Rust テストで既存コード由来の `unused import` 警告が出るが、今回変更による失敗はなし
