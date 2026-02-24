# 機能仕様: Host OS 起動時にターミナルが空白になる問題の修正

**仕様ID**: `SPEC-1029fix`
**作成日**: 2026-02-24
**更新日**: 2026-02-24
**ステータス**: 承認済み
**カテゴリ**: Terminal / PTY
**依存仕様**: なし

**入力**: Issue #1029 - Windows 11 で Host OS ランタイム選択時にターミナルタブが空白になる

## 背景

- Windows 11 環境で Host OS ランタイム選択時にターミナルが空白になる問題が v7.10.2 で再発
- 過去5回の修正（PR #1031, #1045, #1052, #1058, #1142）がマージ済みだが根本原因が未修正
- `resolve_spawn_command_for_platform` が全コマンドを PowerShell `-NonInteractive -Command` でラップしてしまう
- ワーキングディレクトリの存在チェックが欠如
- スクロールバックの未フラッシュ

## ユーザーシナリオとテスト

### ユーザーストーリー 1 - Windows でインタラクティブシェル起動 (優先度: P0)

ユーザーとして、Windows 上で spawn_shell を実行したとき、シェルが正常に起動してほしい。

**受け入れシナリオ**:

1. **前提条件** Windows 環境、**操作** spawn_shell を呼ぶ、**期待結果** PowerShell ラッピングなしでシェルが直接起動する
2. **前提条件** Windows 環境 + interactive=true、**操作** resolve_spawn_command_for_platform を呼ぶ、**期待結果** コマンドとargsがそのまま返る

### ユーザーストーリー 2 - 存在しない WD でのエラー (優先度: P0)

ユーザーとして、ワーキングディレクトリが存在しない場合に明確なエラーメッセージを受け取りたい。

**受け入れシナリオ**:

1. **前提条件** 存在しないパスを working_dir に指定、**操作** PtyHandle::new を呼ぶ、**期待結果** PtyCreationFailed エラーが返る
2. **前提条件** worktree パスが削除済み、**操作** resolve_worktree_path を呼ぶ、**期待結果** エラー文字列が返る

### ユーザーストーリー 3 - スクロールバックフラッシュ (優先度: P1)

ユーザーとして、高速終了するエージェントの出力がスクロールバックに保存されてほしい。

**受け入れシナリオ**:

1. **前提条件** PTY ストリーム終了直後、**操作** capture_scrollback_tail を呼ぶ、**期待結果** データが空でない

## エッジケース

- 非 Windows 環境では interactive フラグが動作に影響しないこと
- shell パラメータが明示指定されている場合は interactive フラグより shell が優先されること

## 要件

### 機能要件

- **FR-001**: interactive=true 時、Windows でも PowerShell ラッピングをスキップする
- **FR-002**: PtyHandle::new で working_dir の存在を検証し、存在しない場合 PtyCreationFailed を返す
- **FR-003**: resolve_worktree_path で取得したパスの存在を検証する
- **FR-004**: stream_pty_output のリードループ終了後に flush_scrollback を呼ぶ

### 非機能要件

- **NFR-001**: 既存テストが全て通ること
- **NFR-002**: clippy 警告なし、fmt チェック通過

## 成功基準

- **SC-001**: `cargo test -p gwt-core` が全テスト通過
- **SC-002**: `cargo test -p gwt-tauri` が全テスト通過
- **SC-003**: `cargo clippy --all-targets --all-features -- -D warnings` が通過
