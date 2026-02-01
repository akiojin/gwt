# タスク: bareリポジトリ対応とヘッダーブランチ表示

**仕様ID**: `SPEC-a70a1ece`
**入力**: `/specs/SPEC-a70a1ece/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、research.md、data-model.md、quickstart.md

## ストーリー間の依存関係

```text
US1 (ヘッダー表示) ──────────────────────────────────────→ 独立
US2 (bare検出) ───────→ US3, US4, US5, US7, US8, US9 の基盤
US3 (clone wizard) ──→ US4 に依存（worktree作成が必要）
US4 (worktree作成) ──→ US2 に依存（bare検出が必要）
US5 (ディレクトリ構造) → US2, US4 に依存
US6 (submodule) ─────→ US4 に依存（P2、後回し可能）
US7 (強制マイグレーション) → US2, US4, US5 に依存
US8 (マイグレーション詳細) → US7 に依存
US9 (マイグレーション追加) → US7, US8 に依存
```

## フェーズ1: セットアップ（共有基盤）

**目的**: 全ストーリーで共有する型定義と基盤構造

### データモデル定義

- [x] **T001** [P] [共通] `crates/gwt-core/src/git/repository.rs` に `RepoType` enum を追加
- [x] **T002** [P] [共通] `crates/gwt-core/src/worktree/location.rs` に `WorktreeLocation` enum を追加
- [x] **T003** [P] [共通] `crates/gwt-core/src/git/repository.rs` に `HeaderContext` 構造体を追加
- [x] **T004** [共通] T001の後に `crates/gwt-core/src/git/mod.rs` に repository モジュールを公開

## フェーズ2: US1 - ヘッダーにブランチ名を表示 (P1)

**ストーリー**: 開発者がgwtを起動すると、ヘッダーのWorking Directory行にブランチ名が角括弧`[]`で表示される

**価値**: UIの一貫性と視認性向上、現在のコンテキストを即座に把握可能

**独立テスト条件**: 通常リポジトリ、bareリポジトリ、worktree内の各環境で起動しヘッダー表示を確認

### ヘッダー表示変更

- [x] **T101** [US1] `crates/gwt-cli/src/tui/app.rs` の `render_header()` で現在ブランチ名を取得する処理を追加
- [x] **T102** [US1] T101の後に `crates/gwt-cli/src/tui/app.rs` のWorking Directory行に `[branch-name]` を追加表示
- [x] **T103** [US1] T102の後に `crates/gwt-cli/src/tui/app.rs` で起動時のブランチ名を固定保持する変数を追加

### (current)ラベル削除

- [x] **T104** [US1] `crates/gwt-cli/src/tui/screens/branch_list.rs` の `current_label` 変数を空文字列に固定

### テスト

- [x] **T105** [P] [US1] `crates/gwt-cli/src/tui/` にヘッダー表示のユニットテストを追加

**✅ MVP1チェックポイント**: US1完了後、ヘッダーにブランチ名が表示される

## フェーズ3: US2 - bareリポジトリの検出 (P1)

**ストーリー**: gwtがbareリポジトリ内で起動された場合、それを正しく検出し適切なUIを表示する

**価値**: bareリポジトリ対応の基盤、他のbare関連機能がこれに依存

**独立テスト条件**: bareリポジトリ内でgwtを起動して正しく認識されることを確認

### 検出ロジック実装

- [x] **T201** [US2] `crates/gwt-core/src/git/repository.rs` に `is_bare_repository()` 関数を実装
- [x] **T202** [US2] T201の後に `crates/gwt-core/src/git/repository.rs` に `is_empty_dir()` 関数を実装
- [x] **T203** [US2] T202の後に `crates/gwt-core/src/git/repository.rs` に `is_git_repo()` 関数を実装
- [x] **T204** [US2] T203の後に `crates/gwt-core/src/git/repository.rs` に `detect_repo_type()` 関数を実装

### TUI統合

- [x] **T205** [US2] T204の後に `crates/gwt-cli/src/tui/app.rs` の起動処理で `detect_repo_type()` を呼び出し
- [x] **T206** [US2] T205の後に `crates/gwt-cli/src/tui/app.rs` で `RepoType::Bare` の場合に `[bare]` をヘッダーに表示

### テスト

- [x] **T207** [P] [US2] `crates/gwt-core/src/git/` に `detect_repo_type()` のユニットテストを追加

**✅ MVP2チェックポイント**: US2完了後、bareリポジトリが正しく検出される

## フェーズ4: US3 - 空ディレクトリでのbare clone (P1)

**ストーリー**: 空のプロジェクトディレクトリでgwtを起動するとURL入力ウィザードが表示される

**価値**: bare推奨ワークフローの入口、新規プロジェクト開始時のUX

**独立テスト条件**: 空ディレクトリでgwtを起動してウィザードが表示されることを確認

### clone設定

- [ ] **T301** [US3] `crates/gwt-core/src/git/clone.rs` に `CloneConfig` 構造体を追加
- [ ] **T302** [US3] T301の後に `crates/gwt-core/src/git/clone.rs` に `clone_bare()` 関数を実装
- [ ] **T303** [US3] T302の後に `crates/gwt-core/src/git/mod.rs` に clone モジュールを公開

### ウィザードUI

- [ ] **T304** [US3] `crates/gwt-cli/src/tui/screens/clone_wizard.rs` に `CloneWizardState` 構造体を作成
- [ ] **T305** [US3] T304の後に `crates/gwt-cli/src/tui/screens/clone_wizard.rs` に `CloneWizardStep` enum を作成
- [ ] **T306** [US3] T305の後に `crates/gwt-cli/src/tui/screens/clone_wizard.rs` に URL入力ステップを実装
- [ ] **T307** [US3] T306の後に `crates/gwt-cli/src/tui/screens/clone_wizard.rs` に clone種別選択ステップを実装
- [ ] **T308** [US3] T307の後に `crates/gwt-cli/src/tui/screens/clone_wizard.rs` に clone実行・進捗表示を実装
- [ ] **T309** [US3] T308の後に `crates/gwt-cli/src/tui/screens/mod.rs` に clone_wizard モジュールを公開

### TUI統合

- [ ] **T310** [US3] T309の後に `crates/gwt-cli/src/tui/app.rs` で `RepoType::Empty` の場合にcloneウィザードを表示
- [ ] **T311** [US3] T310の後に `crates/gwt-cli/src/tui/app.rs` で `RepoType::NonRepo` の場合に警告+cloneウィザードを表示

### CLIオプション

- [ ] **T312** [US3] `crates/gwt-cli/src/main.rs` に `gwt init <url>` サブコマンドを追加
- [ ] **T313** [US3] T312の後に `crates/gwt-cli/src/main.rs` で `init` コマンドがデフォルトでshallow clone（--depth=1）を実行

### テスト

- [ ] **T314** [P] [US3] `crates/gwt-core/src/git/` に `clone_bare()` のユニットテストを追加

**✅ MVP3チェックポイント**: US3完了後、空ディレクトリからbare cloneが可能

## フェーズ5: US4 - worktree作成ウィザード (P1)

**ストーリー**: bareリポジトリからブランチを選択してworktreeを作成できる

**価値**: bareリポジトリからの実際の作業開始に必要

**独立テスト条件**: bareリポジトリでworktreeを作成して正しく動作することを確認

### worktree作成ロジック

- [ ] **T401** [US4] `crates/gwt-core/src/worktree/manager.rs` に `WorktreeLocation` を考慮した `create_for_branch_bare()` を追加
- [ ] **T402** [US4] T401の後に `crates/gwt-core/src/worktree/manager.rs` で bare方式の場合は親ディレクトリにworktreeを作成
- [ ] **T403** [US4] T402の後に `crates/gwt-core/src/worktree/manager.rs` でスラッシュを含むブランチ名のサブディレクトリ構造を処理

### TUI統合

- [ ] **T404** [US4] T403の後に `crates/gwt-cli/src/tui/app.rs` で bareリポジトリ+worktree0件の場合にブランチ選択画面を表示
- [ ] **T405** [US4] T404の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` で bare方式のworktree作成を呼び出し

### テスト

- [ ] **T406** [P] [US4] `crates/gwt-core/src/worktree/` に bare方式worktree作成のユニットテストを追加

**✅ MVP4チェックポイント**: US4完了後、bareリポジトリからworktree作成が可能

## フェーズ6: US5 - ディレクトリ構造 (P1)

**ストーリー**: bare方式ではbareリポジトリとworktreeが同階層に配置される

**価値**: ファイル構造の一貫性、すべての機能に影響

**独立テスト条件**: worktreeを作成してディレクトリ構造を確認

### 設定管理

- [ ] **T501** [US5] `crates/gwt-core/src/config/bare_project.rs` に `BareProjectConfig` 構造体を追加
- [ ] **T502** [US5] T501の後に `crates/gwt-core/src/config/bare_project.rs` に設定ファイルの読み書き関数を実装
- [ ] **T503** [US5] T502の後に `crates/gwt-core/src/config/mod.rs` に bare_project モジュールを公開

### パス解決

- [ ] **T504** [US5] T503の後に `crates/gwt-core/src/worktree/manager.rs` で `.gwt/` をプロジェクトルート（bareの親）に配置
- [ ] **T505** [US5] T504の後に `crates/gwt-core/src/worktree/manager.rs` で URL から bareリポジトリ名（`{repo-name}.git`）を抽出

### ヘッダー表示拡張

- [ ] **T506** [US5] T505の後に `crates/gwt-cli/src/tui/app.rs` で bare方式worktree内の場合に `[branch] (repo.git)` を表示

### テスト

- [ ] **T507** [P] [US5] `crates/gwt-core/src/config/` に `BareProjectConfig` のユニットテストを追加

**✅ MVP5チェックポイント**: US5完了後、bare方式のディレクトリ構造が完成

## フェーズ7: US7 - 強制マイグレーション (P1)

**ストーリー**: 既存の`.worktrees/`方式ユーザーにbare方式への強制マイグレーションダイアログを表示

**価値**: 全ユーザーをbare方式に統一、将来のメンテナンス性向上

**独立テスト条件**: 既存の通常リポジトリでマイグレーションダイアログが表示され正しく移行されることを確認

### マイグレーションモジュール基盤

- [ ] **T701** [US7] `crates/gwt-core/src/migration/mod.rs` にマイグレーションモジュールを作成
- [ ] **T702** [US7] T701の後に `crates/gwt-core/src/migration/config.rs` に `MigrationConfig` 構造体を追加
- [ ] **T703** [US7] T702の後に `crates/gwt-core/src/migration/state.rs` に `MigrationState` enum を追加
- [ ] **T704** [US7] T703の後に `crates/gwt-core/src/migration/error.rs` に `MigrationError` enum を追加

### マイグレーションダイアログUI

- [ ] **T705** [US7] T704の後に `crates/gwt-cli/src/tui/screens/migration_dialog.rs` にダイアログ構造体を作成
- [ ] **T706** [US7] T705の後に `crates/gwt-cli/src/tui/screens/migration_dialog.rs` に続行/拒否の選択肢を実装
- [ ] **T707** [US7] T706の後に `crates/gwt-cli/src/tui/screens/migration_dialog.rs` にステップ別進捗表示を実装
- [ ] **T708** [US7] T707の後に `crates/gwt-cli/src/tui/screens/mod.rs` に migration_dialog モジュールを公開

### TUI統合

- [ ] **T709** [US7] T708の後に `crates/gwt-cli/src/tui/app.rs` で `.worktrees/` 方式検出時にマイグレーションダイアログを表示
- [ ] **T710** [US7] T709の後に `crates/gwt-cli/src/tui/app.rs` で拒否選択時にgwtを終了

### テスト

- [ ] **T711** [P] [US7] `crates/gwt-core/src/migration/` にマイグレーション状態遷移のユニットテストを追加

**✅ MVP6チェックポイント**: US7完了後、マイグレーションダイアログが表示される

## フェーズ8: US8 - マイグレーション詳細動作 (P1)

**ストーリー**: バックアップ、ファイル移動、hooks/submodules/shallow対応、ロールバック

**価値**: マイグレーションの信頼性と安全性を担保

**独立テスト条件**: 各条件（dirty worktree、git hooks存在等）でマイグレーションを実行して動作確認

### 検証処理

- [ ] **T801** [US8] `crates/gwt-core/src/migration/validator.rs` に `check_disk_space()` 関数を実装
- [ ] **T802** [US8] T801の後に `crates/gwt-core/src/migration/validator.rs` に `check_locked_worktrees()` 関数を実装
- [ ] **T803** [US8] T802の後に `crates/gwt-core/src/migration/validator.rs` に `validate_migration()` 関数を実装

### バックアップ処理

- [ ] **T804** [US8] T803の後に `crates/gwt-core/src/migration/backup.rs` に `create_backup()` 関数を実装
- [ ] **T805** [US8] T804の後に `crates/gwt-core/src/migration/backup.rs` に `restore_backup()` 関数を実装

### worktree移行処理

- [ ] **T806** [US8] T805の後に `crates/gwt-core/src/migration/executor.rs` に `WorktreeMigrationInfo` 構造体を追加
- [ ] **T807** [US8] T806の後に `crates/gwt-core/src/migration/executor.rs` に `is_worktree_dirty()` 関数を実装
- [ ] **T808** [US8] T807の後に `crates/gwt-core/src/migration/executor.rs` に `migrate_dirty_worktree()` 関数を実装（ファイル移動方式）
- [ ] **T809** [US8] T808の後に `crates/gwt-core/src/migration/executor.rs` に `migrate_clean_worktree()` 関数を実装（re-clone方式）
- [ ] **T810** [US8] T809の後に `crates/gwt-core/src/migration/executor.rs` に `copy_git_hooks()` 関数を実装
- [ ] **T811** [US8] T810の後に `crates/gwt-core/src/migration/executor.rs` に `preserve_submodules()` 関数を実装
- [ ] **T812** [US8] T811の後に `crates/gwt-core/src/migration/executor.rs` に `exclude_gitignored_files()` 関数を実装

### ロールバック処理

- [ ] **T813** [US8] T812の後に `crates/gwt-core/src/migration/rollback.rs` に `rollback_migration()` 関数を実装
- [ ] **T814** [US8] T813の後に `crates/gwt-core/src/migration/rollback.rs` にネットワークエラー時のリトライ（最大3回）を実装

### 実行オーケストレーション

- [ ] **T815** [US8] T814の後に `crates/gwt-core/src/migration/executor.rs` に `execute_migration()` 関数を実装（順次処理）
- [ ] **T816** [US8] T815の後に `crates/gwt-core/src/migration/mod.rs` にモジュール公開を追加

### テスト

- [ ] **T817** [P] [US8] `crates/gwt-core/src/migration/` に dirty/clean worktree移行のユニットテストを追加
- [ ] **T818** [P] [US8] `crates/gwt-core/src/migration/` にロールバック処理のユニットテストを追加

**✅ MVP7チェックポイント**: US8完了後、マイグレーションが安全に実行される

## フェーズ9: US9 - マイグレーション追加仕様 (P1)

**ストーリー**: パーミッション保持、stash統合、バックアップ削除、設定ファイル配置、トラッキング維持

**価値**: マイグレーションの完全性と使いやすさを担保

**独立テスト条件**: 各条件でマイグレーション後の状態を確認

### パーミッション・stash処理

- [ ] **T901** [US9] `crates/gwt-core/src/migration/executor.rs` に `preserve_file_permissions()` 関数を実装
- [ ] **T902** [US9] T901の後に `crates/gwt-core/src/migration/executor.rs` に `migrate_stash()` 関数を実装

### クリーンアップ・通知

- [ ] **T903** [US9] T902の後に `crates/gwt-core/src/migration/executor.rs` に `cleanup_old_worktrees()` 関数を実装（即時削除）
- [ ] **T904** [US9] T903の後に `crates/gwt-cli/src/tui/screens/migration_dialog.rs` に簡潔な完了メッセージを実装

### 設定・トラッキング

- [ ] **T905** [US9] T904の後に `crates/gwt-core/src/migration/executor.rs` に `create_project_config()` 関数を実装（bareの親に.gwt/配置）
- [ ] **T906** [US9] T905の後に `crates/gwt-core/src/migration/executor.rs` に `derive_bare_repo_name()` 関数を実装（`{元のリポジトリ名}.git`形式）
- [ ] **T907** [US9] T906の後に `crates/gwt-core/src/migration/executor.rs` に `preserve_tracking_relationships()` 関数を実装

### 特殊ケース

- [ ] **T908** [US9] T907の後に `crates/gwt-core/src/migration/executor.rs` に `migrate_local_only_repo()` 関数を実装（ローカル変換）
- [ ] **T909** [US9] T908の後に `crates/gwt-cli/src/tui/screens/migration_dialog.rs` に locked worktree検出時のunlock指示表示を実装

### テスト

- [ ] **T910** [P] [US9] `crates/gwt-core/src/migration/` にパーミッション保持のユニットテストを追加
- [ ] **T911** [P] [US9] `crates/gwt-core/src/migration/` にstash統合のユニットテストを追加

**✅ MVP8チェックポイント**: US9完了後、マイグレーションが完全に動作

## フェーズ10: US6 - submodule対応 (P2)

**ストーリー**: submoduleを含むリポジトリでworktree作成時に自動初期化

**価値**: submoduleを含むプロジェクトでの利便性向上

**独立テスト条件**: submoduleを含むリポジトリでworktreeを作成して確認

### submodule処理

- [ ] **T1001** [US6] `crates/gwt-core/src/git/submodule.rs` に `has_submodules()` 関数を実装
- [ ] **T1002** [US6] T1001の後に `crates/gwt-core/src/git/submodule.rs` に `init_submodules()` 関数を実装
- [ ] **T1003** [US6] T1002の後に `crates/gwt-core/src/git/mod.rs` に submodule モジュールを公開

### worktree作成統合

- [ ] **T1004** [US6] T1003の後に `crates/gwt-core/src/worktree/manager.rs` で worktree作成後に `init_submodules()` を呼び出し

### エラーハンドリング

- [ ] **T1005** [US6] T1004の後に `crates/gwt-core/src/worktree/manager.rs` で submodule初期化失敗時は警告のみ（worktree作成は成功）

### テスト

- [ ] **T1006** [P] [US6] `crates/gwt-core/src/git/` に submodule処理のユニットテストを追加

**✅ 完全な機能**: US6完了後、submodule対応が完了

## フェーズ11: 統合とポリッシュ

**目的**: すべてのストーリーを統合し、品質を確保

### 統合テスト

- [ ] **T1101** [統合] `tests/integration/` にbare clone → worktree作成のエンドツーエンドテストを追加
- [ ] **T1102** [統合] `tests/integration/` にマイグレーションのエンドツーエンドテストを追加
- [ ] **T1103** [統合] エッジケース（書き込み権限なし、ネットワークエラー等）のテストを追加

### 品質チェック

- [ ] **T1104** [統合] `cargo clippy --all-targets --all-features -- -D warnings` を実行して警告を解消
- [ ] **T1105** [統合] `cargo fmt` を実行してフォーマットを統一
- [ ] **T1106** [統合] `cargo test` を実行して全テストがパスすることを確認

### ドキュメント

- [ ] **T1107** [P] [ドキュメント] `specs/SPEC-a70a1ece/quickstart.md` を最終版に更新
- [ ] **T1108** [P] [ドキュメント] `README.md` にbare方式の使い方を追記

## タスク凡例

**優先度**:

- **P1**: 必須 - 基本機能に必要
- **P2**: 重要 - 完全な機能に必要

**依存関係**:

- **[P]**: 並列実行可能
- **T{n}の後に**: 依存タスク完了後に実行

**ストーリータグ**:

- **[US1]** - **[US9]**: 各ユーザーストーリー
- **[共通]**: すべてのストーリーで共有
- **[統合]**: 複数ストーリーにまたがる
- **[ドキュメント]**: ドキュメント専用

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
