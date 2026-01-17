# タスク: コーディングエージェント起動の互換性整備（Rust）

**仕様ID**: `SPEC-3b0ed29b`
**ポリシー**: CLAUDE.md の TDD ルールに基づき、必ず RED→GREEN→リグレッションチェックの順に進める。
**テスト**: `cargo test`（必要な範囲）

## ユーザーストーリー3 - 権限スキップモードでの起動 (優先度: P2)

- [x] **T3101** [P] [US3] `crates/gwt-core/src/agent/codex.rs` にCodexの権限スキップフラグ選択ヘルパーを追加し、v0.79.x/v0.80.0+/不明のユニットテストを追加する
- [x] **T3102** [US3] T3101の後に `crates/gwt-cli/src/main.rs` のCodex起動引数で新ヘルパーを使用し、skip permissionsに正しいフラグを渡す

## ユーザーストーリー11 - 起動直後の異常終了を検知して可視化する (優先度: P1)

- [x] **T3111** [P] [US11] `crates/gwt-cli/src/main.rs` に終了判定（成功/中断/異常）を行うヘルパーを追加し、単体テストを追加する
- [x] **T3112** [US11] T3111の後に `crates/gwt-cli/src/main.rs` の起動処理で異常終了をエラーとして扱い、終了コード/シグナル/経過時間をログ出力する
- [x] **T3113** [US11] `crates/gwt-cli/src/tui/app.rs` でエラー画面をEnter/Escで閉じられるようにし、必要なテストを追加する

## ユーザーストーリー13 - セッション完了後にブランチ一覧へ戻る (優先度: P2)

- [x] **T3121** [US13] `crates/gwt-cli/src/tui.rs` / `crates/gwt-cli/src/tui/app.rs` に起動結果のコンテキスト注入を追加し、成功/失敗メッセージを表示できるようにする
- [x] **T3122** [US13] `crates/gwt-cli/src/main.rs` の対話ループを更新し、起動終了後にブランチ一覧へ復帰する

## ユーザーストーリー14 - 起動/終了時のTUI遷移が固まらない (優先度: P1)

- [x] **T3201** [P] [US14] `crates/gwt-cli/src/main.rs` に起動開始メッセージの即時出力を検証するテストを追加する
- [x] **T3202** [P] [US14] `crates/gwt-cli/src/main.rs` にセッション更新停止時の待機遅延が短いことを検証するテストを追加する
- [x] **T3203** [US14] `crates/gwt-cli/src/main.rs` で起動開始メッセージを依存インストール前に出力する
- [x] **T3204** [US14] `crates/gwt-cli/src/main.rs` のセッション更新待機を停止シグナルで即時解除できるようにする

## ユーザーストーリー1 - デフォルト起動ログの整備 (優先度: P1)

- [x] **T3131** [P] [US1] `crates/gwt-cli/src/main.rs` に起動ログの整形ヘルパーを追加し、Working directory/Model/Reasoning/Mode/Skip/Args/Version/実行方法が出力されることをテストする
- [x] **T3132** [US1] T3131の後に `crates/gwt-cli/src/main.rs` の起動ログをヘルパー経由で統一する

## ユーザーストーリー12 - OpenCodeモデル選択が空で止まらない (優先度: P1)

- [x] **T3141** [P] [US12] `crates/gwt-cli/src/tui/screens/wizard.rs` のOpenCodeモデル選択に default/custom の選択肢を含め、空リストにならないことを担保する
- [x] **T3142** [US12] `crates/gwt-cli/src/tui/screens/wizard.rs` にOpenCodeモデル選択が空にならないことを検証するユニットテストを追加する

## 統合

- [x] **T3190** [統合] `cargo build --release` を実行してビルド確認する
- [x] **T3205** [統合] `cargo test -p gwt-cli` と `cargo build --release` を実行して失敗がないことを確認する

## 追加作業: Codexモデル/スキップ権限の互換修正 (2026-01-14)

- [x] **T3150** [Test] `crates/gwt-core/src/agent/codex.rs` のデフォルトモデルとスキップ時引数構成のテストを更新する
- [x] **T3151** [Test] `crates/gwt-cli/src/main.rs` にCodexのスキップフラグが末尾に来ることを確認するテストを追加する
- [x] **T3152** [実装] `crates/gwt-core/src/agent/codex.rs` のデフォルトモデルを仕様に合わせて更新し、スキップ時はsandbox設定を抑止する
- [x] **T3153** [実装] `crates/gwt-cli/src/tui/screens/wizard.rs` のCodexモデル選択肢を仕様に合わせて更新する
- [x] **T3154** [実装] `crates/gwt-cli/src/main.rs` でClaude Codeのスキップ時に`IS_SANDBOX=1`を必ず付与する
