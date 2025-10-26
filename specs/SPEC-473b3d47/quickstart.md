# クイックスタート: Claude Code / Codex CLI 対応の対話型Gitワークツリーマネージャー

**仕様ID**: `SPEC-473b3d47` | **日付**: 2025-10-24
**関連ドキュメント**: [spec.md](./spec.md) | [plan.md](./plan.md) | [data-model.md](./data-model.md)

## 概要

このガイドは、`@akiojin/claude-worktree`を最短で使い始めるための手順を提供します。5分で基本的な使い方を習得できます。

## 前提条件

### 必須

- **Git** 2.5+ (worktree機能サポート)
- **Bun** 1.0.0+
- **AIツール**: 以下のいずれか1つ以上
  - Claude Code ([https://claude.ai/code](https://claude.ai/code))
  - Codex CLI

### オプション

- **Node.js** 18.0.0+（開発ツール利用時）
- **GitHub CLI** (`gh`) - PR自動クリーンアップ機能に必要

## インストール

### グローバルインストール（推奨）

```bash
bun add -g @akiojin/claude-worktree
```

### 一回限りの実行

```bash
bunx @akiojin/claude-worktree
```

## 基本的な使い方

### 1. 初回起動

任意のGitリポジトリのルートディレクトリで実行：

```bash
claude-worktree
```

![Main Menu](https://via.placeholder.com/600x300?text=Main+Menu+Screenshot)

**表示される内容**:
- ローカルブランチ一覧
- リモートブランチ一覧
- 「新規ブランチ作成」メニュー
- 「ワークツリー管理」メニュー
- 「マージ済みPRクリーンアップ」メニュー
- 「終了」オプション

### 2. 既存ブランチでワークツリーを開く

**ステップ1**: ブランチを選択

キーボードの矢印キー（↑/↓）でブランチを選択し、Enterで決定。

**ステップ2**: ワークツリー作成を確認

既存のワークツリーがない場合、以下のような確認が表示されます：

```text
ワークツリーを作成しますか？
ブランチ: feature/new-feature
パス: /path/to/parent/worktree-feature-new-feature
```

**ステップ3**: AIツールを選択

Claude CodeとCodex CLIが両方インストールされている場合、選択画面が表示されます：

```text
使用するAIツールを選択してください:
> Claude Code
  Codex CLI
```

片方のみインストールされている場合は自動選択されます。

**ステップ4**: 実行モードを選択

```text
実行モードを選択してください:
> 通常起動
  Resume（前回の続きから）
  Continue（別のセッション）
```

**ステップ5**: 権限設定（Claude Codeの場合）

```text
権限設定をスキップしますか？ (y/N)
```

### 3. 新規ブランチを作成してワークツリーを開く

**ステップ1**: メニューから「新規ブランチ作成」を選択

**ステップ2**: ブランチタイプを選択

```text
ブランチタイプを選択してください:
> feature  - 新機能開発
  hotfix   - 緊急バグ修正
  release  - リリース準備
```

**ステップ3**: ブランチ名を入力

```text
ブランチ名を入力してください（プレフィックスなし）:
> user-authentication
```

自動的に `feature/user-authentication` が生成されます。

**ステップ4**: ベースブランチを選択

```text
ベースブランチを選択してください:
> main
  develop
```

**ステップ5**: 残りは「既存ブランチでワークツリーを開く」と同じ流れ

### 4. セッションを継続する

前回使用したワークツリーで作業を再開：

```bash
claude-worktree -c
```

数秒で前回のワークツリーが開き、AIツールが起動します。

### 5. セッションを選択して復元する

過去のセッションから選択：

1. 起動直後の実行モード画面で「セッション再開」を選択
2. 直近24時間のセッション一覧から目的のセッションを選択
3. 選択したセッションのワークツリーでAIツールが起動

## 高度な使い方

### AIツール選択のヒント

- Claude Code と Codex CLI の両方が検出された場合、ツール選択画面で上下キーを使って切り替えできます。
- 権限確認をスキップしたい場合は、ツール選択画面で `s` キーを押してトグルします。
- 利用可能なツールが1つだけの場合は自動的に選択されます。

### ワークツリー管理

**ステップ1**: メニューから「ワークツリー管理」を選択

**ステップ2**: 管理したいワークツリーを選択

```text
ワークツリーを選択してください:
> feature/user-auth - /path/to/worktree-feature-user-auth
  hotfix/bug-123 - /path/to/worktree-hotfix-bug-123
  [戻る]
```

**ステップ3**: アクションを選択

```text
アクションを選択してください:
> 開く
  削除（ワークツリーのみ）
  削除（ワークツリーとブランチ）
  [戻る]
```

### マージ済みPRのクリーンアップ

**前提条件**: GitHub CLI (`gh`)がインストールされ、認証済み

**ステップ1**: メニューから「マージ済みPRクリーンアップ」を選択

**ステップ2**: クリーンアップ対象を確認

```text
マージ済みPR:
✓ feature/user-auth (PR #123) - ワークツリーあり
✓ hotfix/bug-456 (PR #124) - ブランチのみ
```

**ステップ3**: 削除対象を複数選択

スペースキーで選択/解除、Enterで決定

**ステップ4**: 確認して実行

```text
以下を削除しますか？
- feature/user-auth (ワークツリー + ブランチ)
- hotfix/bug-456 (ブランチのみ)

> はい
  いいえ
```

**ステップ5**: リモートブランチ削除の確認

```text
リモートブランチも削除しますか？ (y/N)
```

### リリースブランチの作成

**ステップ1**: 「新規ブランチ作成」→「release」を選択

**ステップ2**: バージョンバンプタイプを選択

現在のバージョン（例: 1.2.3）から自動計算：

```text
バージョンバンプタイプを選択してください:
> major (2.0.0)
  minor (1.3.0)
  patch (1.2.4)
```

**ステップ3**: ワークツリーが作成され、package.jsonが自動更新される

**ステップ4**: AIツールで作業後、リリースアクションを選択

```text
リリースアクションを選択してください:
> complete  - プッシュしてPRを作成
  continue  - 保存して後で続ける
  nothing   - 何もせずに終了
```

「complete」を選択すると：
1. ブランチがプッシュされる
2. mainブランチへのPRが自動作成される（GitHub CLI使用時）
3. タグ作成とdevelopへのマージバック手順が表示される

## 作業終了時の変更管理

AIツールでの作業を終えると、未コミットの変更がある場合、以下のオプションが表示されます：

```text
変更を確認してください:
> status  - git statusを表示
  commit  - 変更をコミット
  stash   - 変更をstash
  discard - 変更を破棄
  continue - 何もせず続行
```

### commit を選択した場合

```text
コミットメッセージを入力してください:
> Add user authentication feature
```

### discard を選択した場合

```text
警告: すべての変更が失われます。
本当に破棄しますか？ (y/N)
```

## トラブルシューティング

### エラー: "Current directory is not a Git repository."

**原因**: Gitリポジトリでないディレクトリで実行

**解決策**: `git init`でリポジトリを初期化するか、既存のGitリポジトリに移動

### エラー: "Claude Code is not available."

**原因**: Claude CodeまたはCodex CLIがインストールされていない

**解決策**:
- Claude Code: [https://claude.ai/code](https://claude.ai/code)からインストール
- Codex CLI: プロジェクトのドキュメントを参照

### 警告: "Running from a worktree directory is not recommended."

**原因**: ワークツリーディレクトリから実行

**解決策**: メインリポジトリのルートディレクトリに移動して実行

### エラー: "GitHub CLI is not installed."

**原因**: GitHub CLI (`gh`)がインストールされていない（PR機能使用時のみ）

**解決策**: [https://cli.github.com/](https://cli.github.com/)からインストール

### セッション継続時に "Worktree no longer exists" 警告

**原因**: 前回のワークツリーが削除または移動された

**解決策**: 通常のメニューから新しいワークツリーを作成

## ベストプラクティス

### 1. ブランチ命名規則

Git Flowに準拠したブランチ名を使用：

- `feature/` - 新機能開発
- `hotfix/` - 緊急バグ修正
- `release/` - リリース準備

### 2. ワークツリーの整理

定期的に「マージ済みPRクリーンアップ」を実行して、不要なワークツリーとブランチを削除

### 3. セッション継続の活用

頻繁に同じブランチで作業する場合は `-c` オプションを活用

### 4. リリースブランチのバージョン管理

Semantic Versioning（Major.Minor.Patch）に従ってバージョンを選択

## 次のステップ

- [仕様書](./spec.md) - 全機能の詳細な説明
- [実装計画](./plan.md) - アーキテクチャと技術スタック
- [データモデル](./data-model.md) - エンティティとデータフロー
- [GitHub リポジトリ](https://github.com/akiojin/claude-worktree) - ソースコードとissue

## サポート

問題やバグを見つけた場合は、GitHubのissueで報告してください：
[https://github.com/akiojin/claude-worktree/issues](https://github.com/akiojin/claude-worktree/issues)
