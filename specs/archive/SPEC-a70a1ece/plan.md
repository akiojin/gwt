# 実装計画: bareリポジトリ対応とヘッダーブランチ表示

**仕様ID**: `SPEC-a70a1ece` | **日付**: 2026-02-01 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-a70a1ece/spec.md` からの機能仕様

## 概要

gwtにbareリポジトリ対応を追加し、ヘッダーにブランチ名を表示する。主な変更点：

1. **ヘッダー表示変更**: Working Directory行にブランチ名を `[branch]` 形式で追加
2. **`(current)`ラベル削除**: ブランチリストから削除し、ヘッダーに集約
3. **bareリポジトリ検出**: 起動時にbare/通常/worktree/空を判定
4. **空ディレクトリ対応**: bare cloneウィザードを表示
5. **bare方式worktree作成**: bareと同階層にworktreeを配置

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui 0.29, crossterm 0.28, serde, serde_json, chrono
**ストレージ**: ファイルシステム（.gwt/設定、gitメタデータ）
**テスト**: cargo test（統合テストでローカルbareリポジトリ使用）
**ターゲットプラットフォーム**: Linux, macOS, Windows
**プロジェクトタイプ**: 単一（gwt-core + gwt-cli）
**パフォーマンス目標**: 起動時のbare検出は100ms以内
**制約**: gitコマンド使用（libgit2不使用）、認証はgit/OS委譲
**スケール/範囲**: 数百ブランチ、数十worktree

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。*

| 原則 | 状態 | 備考 |
|------|------|------|
| I. シンプルさの追求 | ✅ | 既存UIパターンを踏襲、新規概念を最小化 |
| II. テストファースト | ✅ | 統合テストでローカルbareリポジトリを使用 |
| III. 既存コードの尊重 | ✅ | 既存ファイル（app.rs, branch_list.rs, manager.rs）を改修 |
| IV. 品質ゲート | ✅ | clippy, fmt, testを通過させる |
| V. 自動化の徹底 | ✅ | Conventional Commits遵守 |

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-a70a1ece/
├── spec.md              # 機能仕様
├── plan.md              # このファイル
├── research.md          # フェーズ0出力
├── data-model.md        # フェーズ1出力
├── quickstart.md        # フェーズ1出力
└── tasks.md             # フェーズ2出力
```

### ソースコード（変更対象）

```text
crates/
├── gwt-core/src/
│   ├── git/
│   │   ├── branch.rs         # ブランチ情報取得（変更）
│   │   └── repository.rs     # リポジトリ種別検出（追加）
│   └── worktree/
│       └── manager.rs        # worktree作成ロジック（変更）
├── gwt-cli/src/
│   ├── main.rs               # CLIオプション追加（gwt init）
│   └── tui/
│       ├── app.rs            # ヘッダー表示変更
│       └── screens/
│           ├── branch_list.rs    # (current)ラベル削除
│           └── clone_wizard.rs   # 新規: bare cloneウィザード
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存コードパターンを理解し、実装方針を確定する

**出力**: `specs/SPEC-a70a1ece/research.md`

### 調査項目

1. **既存のコードベース分析**
   - ヘッダー表示: `app.rs:5354-5365` - Working Directory行
   - (current)表示: `branch_list.rs:1855` - `is_current`フラグ
   - worktree管理: `worktree/manager.rs` - create/list/remove
   - gitコマンド実行: `std::process::Command`パターン

2. **技術的決定**
   - bareリポジトリ検出: `git rev-parse --is-bare-repository`
   - リポジトリ種別: 新規enum `RepoType { Normal, Bare, Worktree, Empty, NonRepo }`
   - worktree配置: `WorktreeLocation { Subdir, Sibling }`で分岐

3. **制約と依存関係**
   - 既存のウィザードUIパターン（`worktree_create.rs`）を踏襲
   - 既存のgitコマンド実行パターンを踏襲
   - 後方互換性: 通常リポジトリ + `.worktrees/`方式を維持

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-a70a1ece/data-model.md`
- `specs/SPEC-a70a1ece/quickstart.md`

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要なエンティティ:

- **RepoType**: リポジトリ種別 (Normal, Bare, Worktree, Empty, NonRepo)
- **WorktreeLocation**: worktree配置方式 (Subdir, Sibling)
- **CloneConfig**: clone設定 (url, shallow, depth)
- **HeaderContext**: ヘッダー表示コンテキスト (branch_name, repo_type, bare_name)

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド:
- ヘッダー表示変更のテスト方法
- bareリポジトリのローカル作成方法
- 統合テストの実行方法

### 1.3 契約/インターフェース

この機能ではAPIエンドポイントは不要。内部インターフェース:

- `detect_repo_type(path: &Path) -> RepoType`
- `get_header_context(repo_type: &RepoType) -> HeaderContext`
- `clone_bare(url: &str, shallow: bool) -> Result<PathBuf>`

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-a70a1ece/tasks.md`

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装:

1. **P1-1**: ヘッダーブランチ表示 + (current)削除（独立して完了可能）
2. **P1-2**: bareリポジトリ検出（他のbare機能の基盤）
3. **P1-3**: 空ディレクトリ検出 + bare cloneウィザード
4. **P1-4**: bare方式worktree作成
5. **P1-5**: ディレクトリ構造（bare同階層配置）
6. **P1-6**: 強制マイグレーション機能
7. **P1-7**: マイグレーション詳細動作（バックアップ、ロールバック等）
8. **P1-8**: マイグレーション追加仕様（パーミッション、stash、hooks等）
9. **P2**: submodule対応

### 独立したデリバリー

各ユーザーストーリーは独立して実装・テスト・デプロイ可能:

- ストーリー1（ヘッダー表示）完了 → 即座にユーザー価値を提供
- ストーリー2（bare検出）追加 → bareリポジトリでの起動が可能に
- ストーリー3-5（clone/worktree）追加 → 完全なbareワークフロー
- ストーリー7-9（マイグレーション）追加 → 既存ユーザーの自動移行

## テスト戦略

- **ユニットテスト**: `detect_repo_type()`, `get_header_context()`
- **統合テスト**: ローカルbareリポジトリを作成してテスト
  - テスト用bareリポジトリ: `git init --bare`で作成
  - テスト用worktree: `git worktree add`で作成
- **TUIテスト**: 既存のratatui test utilityを使用
- **後方互換性テスト**: 既存の通常リポジトリでの動作確認

## リスクと緩和策

### 技術的リスク

1. **bareリポジトリの誤検出**
   - **緩和策**: `git rev-parse --is-bare-repository`の結果を厳密にチェック

2. **ディレクトリ構造の複雑化**
   - **緩和策**: `RepoType`と`WorktreeLocation`で明確に分岐

3. **マイグレーション中のデータ損失**
   - **緩和策**: マイグレーション前にバックアップ必須、エラー時は即時自動ロールバック

4. **マイグレーション中のディスク容量不足**
   - **緩和策**: 事前に必要容量を計算し、不足時はブロック

5. **locked worktreeによるマイグレーション失敗**
   - **緩和策**: マイグレーション前にlocked worktreeを検出してブロック

6. **ネットワークエラーによるマイグレーション中断**
   - **緩和策**: 最大3回の自動リトライ、失敗時はロールバック

### 依存関係リスク

1. **既存機能への影響**
   - **緩和策**: マイグレーション完了後に全機能が動作することをテスト

### ユーザー体験リスク

1. **強制マイグレーションへの反発**
   - **緩和策**: 明確なエラーメッセージと将来のメンテナンス性向上のメリットを説明

2. **マイグレーション時間の長さ**
   - **緩和策**: ステップ別詳細進捗表示でユーザーに状況を伝える

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ✅ フェーズ1完了: 設計ドキュメント生成
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
