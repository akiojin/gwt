# 実装計画: Worktree内でのコマンド実行制限機能

**仕様ID**: `SPEC-eae13040` | **日付**: 2025-11-09 | **仕様書**: [spec.md](../../SPEC-eae13040/spec.md)
**入力**: `/specs/SPEC-eae13040/spec.md` からの機能仕様

**注**: このテンプレートは `/speckit.plan` コマンドによって記入されます。実行ワークフローについては `.specify/templates/commands/plan.md` を参照してください。

## 概要

Claude CodeのBashツール実行時に、Worktree境界を越える操作(作業ディレクトリ変更、ブランチ切り替え、Worktree外のファイル操作)を検出・ブロックするPreToolUseフック機構を実装する。既存のフックスクリプト(block-cd-command.sh、block-git-branch-ops.sh)を改善し、参照系コマンドは許可しつつ変更系コマンドのみをブロックする細かい制御を実現する。

## 技術コンテキスト

**言語/バージョン**: Bash 4.0以上
**主要な依存関係**: jq (JSON解析)、git (Worktreeルート取得)、realpath (シンボリックリンク解決、フォールバック対応)
**ストレージ**: N/A (フックスクリプトは状態を持たない)
**テスト**: Bats (Bash Automated Testing System) または手動テストスクリプト
**ターゲットプラットフォーム**: Linux/macOS (Claude Code実行環境)
**プロジェクトタイプ**: シェルスクリプトベースのフックシステム
**パフォーマンス目標**: フック実行時間 <100ms (ユーザー体験に影響しないため)
**制約**: 既存のClaude Code PreToolUseフック機構に準拠、Bashツール呼び出し前に同期実行
**スケール/範囲**: 単一Worktree環境、数十〜数百のコマンドパターンを判定

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

Constitution.mdがテンプレートのため、プロジェクト固有の原則は未定義。以下の一般的なゲートを適用:

- ✅ **シンプルさ**: 既存のフックスクリプトを改善する最小限のアプローチ
- ✅ **テスト可能性**: 各コマンドパターンが個別にテスト可能
- ✅ **保守性**: スクリプトロジックが明確で、将来の拡張が容易

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-eae13040/
├── spec.md              # 機能仕様
├── checklists/
│   └── requirements.md  # 仕様品質チェックリスト
specs/feature/disallow-bash-command/
├── plan.md              # このファイル（/speckit.plan コマンド出力）
├── research.md          # フェーズ0出力（/speckit.plan コマンド）
├── data-model.md        # フェーズ1出力（/speckit.plan コマンド）
├── quickstart.md        # フェーズ1出力（/speckit.plan コマンド）
├── contracts/           # フェーズ1出力（/speckit.plan コマンド） - N/A (フックスクリプトにはAPI契約なし)
└── tasks.md             # フェーズ2出力（/speckit.tasks コマンド - /speckit.planでは作成されません）
```

### ソースコード（リポジトリルート）

```text
.claude/
├── settings.json                      # フック設定
└── hooks/
    ├── block-cd-command.sh            # cd コマンド制限フック (既存、改善対象)
    ├── block-git-branch-ops.sh        # git ブランチ操作制限フック (既存、改善対象)
    └── block-file-ops.sh              # ファイル操作制限フック (新規作成予定)

tests/
└── hooks/
    ├── test-cd-command.bats           # cd コマンドフックのテスト
    ├── test-git-branch-ops.bats       # git ブランチ操作フックのテスト
    └── test-file-ops.bats             # ファイル操作フックのテスト
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存のフックスクリプトの実装を分析し、改善箇所と新規実装箇所を特定する

**出力**: `specs/feature/disallow-bash-command/research.md`

### 調査項目

1. **既存のコードベース分析**
   - block-cd-command.shの実装詳細とWorktree境界判定ロジック
   - block-git-branch-ops.shの実装詳細とgit branchコマンド判定ロジック
   - is_read_only_git_branch()関数の動作原理と改善点

2. **技術的決定**
   - ファイル操作コマンド(mkdir、rm、touch等)のブロック方法
   - git checkout -- fileとgit checkout branchの区別方法
   - 複合コマンド内のコマンド分割ロジックの精度向上

3. **制約と依存関係**
   - jqコマンドの利用可能性とバージョン互換性
   - realpathコマンドのフォールバック実装
   - Python3の利用可能性(shell

解析のため)

## フェーズ1: 設計(アーキテクチャと契約)

**目的**: フックスクリプトの改善設計と新規フックの設計を定義する

**出力**:
- `specs/feature/disallow-bash-command/data-model.md`
- `specs/feature/disallow-bash-command/quickstart.md`
- `specs/feature/disallow-bash-command/contracts/` - N/A (フックスクリプトにはAPI契約なし)

### 1.1 データモデル設計

**ファイル**: `data-model.md`

フックスクリプトで扱う主要なデータ構造:
- JSON入力スキーマ(tool_name、tool_input.command)
- コマンドパターン定義(正規表現、許可/禁止リスト)
- エラー応答スキーマ(decision、reason、stopReason)
- Worktree境界情報(ルートパス、判定結果)

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド:
- フックスクリプトのテスト方法
- 新しいコマンドパターンの追加方法
- デバッグ手順
- トラブルシューティング

### 1.3 契約/インターフェース(該当する場合)

**ディレクトリ**: `contracts/` - N/A

フックスクリプトは内部コンポーネントのため、公開API契約は不要。ただし、以下の内部契約は存在:
- Claude Code PreToolUseフック仕様(JSON入出力フォーマット)
- フックスクリプトの終了コード規約(0=許可、2=ブロック)

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/feature/disallow-bash-command/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装:
1. **P1 - ユーザーストーリー1**: Bashコマンドによる作業ディレクトリ保護 (block-cd-command.shの改善)
2. **P1 - ユーザーストーリー2**: Gitブランチ操作の制御 (block-git-branch-ops.shの改善)
3. **P2 - ユーザーストーリー3**: ファイル・ディレクトリ操作の範囲制限 (block-file-ops.shの新規作成)
4. **P2 - ユーザーストーリー4**: 複合コマンドでの制限適用 (既存スクリプトのコマンド分割ロジック改善)

### 独立したデリバリー

各ユーザーストーリーは独立して実装・テスト・デプロイ可能:
- ストーリー1を完了 → cdコマンド制限が動作
- ストーリー2を追加 → gitブランチ操作制限が動作
- ストーリー3を追加 → ファイル操作制限が動作
- ストーリー4を追加 → 複合コマンド制限が完全動作

## テスト戦略

機能仕様のテスト要件に基づく:

- **ユニットテスト**: 各関数(is_within_worktree、is_read_only_git_branch等)を個別にテスト
- **統合テスト**: フックスクリプト全体をJSON入力でテストし、期待される終了コードと出力を検証
- **エンドツーエンドテスト**: 実際のClaude Code環境でBashツールを呼び出し、ブロック/許可動作を検証
- **エッジケーステスト**: シンボリックリンク、相対パス、存在しないディレクトリ等のエッジケースを網羅

テストフレームワーク: Bats (Bash Automated Testing System) または手動テストスクリプト

## リスクと緩和策

### 技術的リスク

1. **realpathコマンドの非互換性**: 一部の環境でrealpathが利用不可
   - **緩和策**: Pythonやpwdコマンドを使ったフォールバック実装

2. **複雑な複合コマンドの解析失敗**: ヒアドキュメント、クォート、エスケープの複雑な組み合わせ
   - **緩和策**: Python shlex.split()を使った堅牢な解析

3. **git checkout -- fileとgit checkout branchの誤判定**: 引数解析の曖昧性
   - **緩和策**: --の有無を明示的にチェックし、ファイル復元パターンを優先判定

### 依存関係リスク

1. **jqコマンドの非互換性**: バージョンによるJSON解析の違い
   - **緩和策**: jq 1.5以上を必須とし、互換性のある構文のみ使用

2. **Bash 4.0未満の環境**: 配列構文やreadarray等の機能が利用不可
   - **緩和策**: Bash 4.0以上を推奨し、古い環境向けのフォールバックを提供

## 次のステップ

1. ⏭️ フェーズ0: 調査と技術スタック決定
2. ⏭️ フェーズ1: 設計とアーキテクチャ定義
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
