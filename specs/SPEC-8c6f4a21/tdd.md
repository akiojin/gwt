# TDDノート: Windows での Git 実行時コンソール点滅抑止

## 対象

- `crates/gwt-core/src/process.rs`
- `crates/gwt-core/tests/no_direct_git_command.rs`
- Git 実行を持つ既存モジュール群（`gwt-core` / `gwt-tauri`）

## テスト戦略

1. 既存テストの回帰確認を最優先する（機能互換性）。
2. 新規テストで「再発防止ルール」を固定する。
3. Windows 専用挙動は API レベルで副作用なく呼べることを担保し、実機確認を受け入れ条件にする。

## Red / Green 記録

### T1: 本番ソース直書き `Command::new("git")` を禁止

- **Red**: `no_direct_git_command` テスト追加前は混入を検知できない。
- **Green**: `crates/gwt-core/tests/no_direct_git_command.rs` 追加後、混入時にテスト失敗。

### T2: 共通ヘルパー API の基本保証

- **Red**: `process.rs` 追加前は `git_command()` が存在しない。
- **Green**: `process.rs` のユニットテストで `git_command().get_program() == "git"` を確認。

### T3: 機能回帰なし

- **Red**: 一括置換直後は未使用 import 警告が発生。
- **Green**: import 調整後に `cargo check` / `cargo test` が通過。

## 実行ログ（要約）

- `cargo check -q` : pass
- `cargo test -q` : pass
- `cargo test -q -p gwt-core` : pass

## 残課題

- Windows 実機での手動確認（点滅コンソール再現なし）を最終受け入れとして記録する。
