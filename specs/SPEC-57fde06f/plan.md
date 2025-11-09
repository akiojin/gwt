# 実装計画: releaseブランチ経由の自動リリース＆Auto Mergeフロー

**仕様ID**: `SPEC-57fde06f` | **日付**: 2025-11-07 | **仕様書**: [specs/SPEC-57fde06f/spec.md](./spec.md)
**入力**: `specs/SPEC-57fde06f/spec.md` からの機能仕様

## 概要

現状の `/release` コマンドは develop→main PR を生成する旧フローが残っている。本計画では unity-mcp-server と同じ release ブランチ方式を完全導入し、(1) `/release` / helper script が `create-release.yml` を介して `release/vX.Y.Z` を生成、(2) `release.yml` が release ブランチ上で semantic-release を実行して main へ直接マージ、(3) `publish.yml` が npm publish（任意）と develop へのバックマージを自動化するという流れへ統一する。GitHub Branch Protection では main への直接 push を禁止し、CI のみが変更できる設計とする。

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8.x（Bun 1.0+ ランタイム） / GitHub Actions YAML / shell scripts
**主要な依存関係**: semantic-release 22.x、gh CLI、GitHub Actions (`actions/checkout`, `actions/github-script`)、bun / pnpm
**ストレージ**: N/A（Git refs と GitHub メタデータのみ）
**テスト**: Bun + Vitest（CLIユニット）、GitHub Actions の workflow_run、可能であれば `act` などローカルシミュレーション
**ターゲットプラットフォーム**: GitHub Actions runner (ubuntu-latest) / 開発者ローカル CLI (macOS/Linux)
**プロジェクトタイプ**: 単一 CLI + ワークフロー（モノレポ）
**パフォーマンス目標**: release→main PR が Required チェック成功後 10 分以内に自動マージ / `/release` 実行から release branch push まで 2 分以内
**制約**: 新規サービスを追加せず、既存 GitHub Secrets/Token を流用。main 直接 push は不可。worktree 既存ブランチで完結。
**スケール/範囲**: `CLAUDE.md`, `.claude/commands/release.md`, `.github/workflows/*`, `.releaserc.json`, release CLI 実装、Docs を中心に 6〜8 ファイルが対象。

**Language/Version**: TypeScript 5.8.x / Bun 1.0+ / GitHub Actions YAML
**Primary Dependencies**: semantic-release 22.x, gh CLI, GitHub Actions (`actions/checkout`, `actions/github-script`)
**Storage**: N/A
**Tests**: Bun + Vitest, GitHub Actions workflow checks
**Project Type**: CLI + workflow automation (single repo)

## 原則チェック

- **シンプルさ最優先**: release ブランチという 1 つの制御点に集約し、git コマンドと既存ワークフローのみで構成 → ✅
- **Spec Kit順守**: specify→plan→tasks→implement の順序で進行（現在 plan フェーズ）→ ✅
- **Worktree運用**: 現在の feature/auto-release worktree で完結し、新規ブランチを作らない → ✅
- **ドキュメント集中管理**: 手順は CLAUDE.md / `.claude/commands/` / README へ集約し、他所に分散させない → ✅

ゲート結果: PASS（Phase 0 の調査へ進行可能）

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-57fde06f/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── release-automation.md
└── tasks.md
```

### ソースコード（リポジトリルート）

```text
.claude/commands/release.md          # CLI スラッシュコマンドの説明
.github/workflows/create-release.yml # release ブランチ生成
.github/workflows/release.yml        # semantic-release + main merge
.github/workflows/publish.yml        # npm publish + develop back-merge
scripts/create-release-branch.sh     # ローカル helper
CLAUDE.md / README.* / docs/release-guide*.md
.releaserc.json                      # release/* ブランチを対象にする設定
```

## フェーズ0: 調査（技術スタック選定）

**目的**: release ブランチを中心とした CI 設計を固め、gh CLI・GitHub API・semantic-release の設定ポイントを明確にする。

**出力**: `specs/SPEC-57fde06f/research.md`

### 調査項目

1. **既存のコードベース分析**
   - `/release` コマンドと `scripts/create-release-branch.sh` の役割分担
   - `create-release.yml` / `release.yml` / `publish.yml` の依存関係と権限
   - `.releaserc.json` の `branches` 設定が `release/*` になっているか

2. **技術的決定**
   - release ブランチ更新方式：gh CLI の merge? git fetch & push? fast-forward enforce?（現状 CLI vs Actions どちら）
   - Auto Merge の有効化手段：`gh pr merge --auto` vs `github-script`
   - Required チェックの一覧と Actions job 名称

3. **制約と依存関係**
   - Branch Protection 設定の更新手段（手動案内 or API）
   - Secrets (`SEMANTIC_RELEASE_TOKEN`, `NPM_TOKEN`) の適用タイミング

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: release ブランチを起点とするデータフロー（Git refs、PR、CIジョブ）とオペレーションガイドを設計する。

**出力**: `data-model.md`, `quickstart.md`, `contracts/release-automation.md`

### 1.1 データモデル設計

- `ReleaseBranchState`: develop と release ブランチの整合状態、semantic-release の最新結果
- `ReleaseWorkflowRun`: `release.yml` の実行ステータス、ジョブ URL、出力されたバージョン
- `PublishWorkflowRun`: `publish.yml` の結果、npm publish 成否、バックマージ結果
- `BranchProtectionConfig`: main の Required Checks / push 制限設定
- `ReleaseCommandInvocation`: `/release` もしくは helper script の実行メタデータ

### 1.2 クイックスタートガイド

- `/release` 実行前の前提（develop clean、`gh auth login` 済み）
- `scripts/create-release-branch.sh` の利用手順と `create-release.yml` 監視方法
- `release.yml` / `publish.yml` の確認ポイント（成功/失敗時の対応）
- main で hotfix が必要なときのエスカレーション手順

### 1.3 契約/インターフェース

- release コマンド契約: 入力（develop HEAD), 出力（release branch ref, workflow run IDs）
- GitHub Actions 契約: `create-release.yml` → `release.yml` → `publish.yml` の順序と必要な secrets
- Branch Protection 契約: main の `required_status_checks`, direct push 禁止, workflow 書き込み権限

### Agent Context Update

フェーズ1完了後に `SPECIFY_FEATURE=SPEC-57fde06f .specify/scripts/bash/update-agent-context.sh claude` を実行し、CLAUDE.md の Active Technologies / Recent Changes を release フロー内容で同期する。

## フェーズ2: タスク生成

- `/speckit.tasks` を実行し、P1（release ブランチ同期 + semantic-release 実行）、P1（Auto Merge & Required チェック構成）、P2（ドキュメント/ガバナンス更新）に分けたチェックリストを生成。
- tasks.md では CLI／ワークフロー／docs の並列実行可能タスクを明示する。

## 実装戦略

1. **P1-1: release ブランチ生成パイプライン**
   - `/release` コマンド + `scripts/create-release-branch.sh` を `create-release.yml` 起動用に統一
   - `.releaserc.json` を `release/*` ターゲットへ固定し、ワークフローの secrets/権限を整理
2. **P1-2: release.yml / publish.yml オーケストレーション**
   - release push で semantic-release を実行し main へ直接マージ、publish で npm + back-merge を保証
   - 失敗時のリカバリー手順とログ出力を整備
3. **P2: ドキュメントと Spec 整備**
   - CLAUDE.md / `.claude/commands/release.md` / README / docs を最新フローで同期
   - specs/quickstart/contracts で回復手順やチェックリストを明文化

## テスト戦略

- **ユニットテスト**: release CLI (if scripted) の関数を Vitest でモックし develop→release push ロジックを検証。
- **統合テスト**: GitHub Actions の `workflow_dispatch` で `create-release.yml` → `release.yml` → `publish.yml` の連携を確認。
- **エンドツーエンド**: 手動で `/release` を走らせ、release ブランチ push → semantic-release 成功 → main への直接コミット → develop バックマージまでを監視。
- **パフォーマンス**: release job 所要時間と Auto Merge までの時間を計測し、SC-002 (≤10分) を満たす。

## リスクと緩和策

1. **release.yml が main merge に失敗するリスク**: 権限不足やコンフリクトで main 更新が止まる。→ 緩和: PAT の権限チェックを実装し、失敗時は release branch を保持したままログ出力。
2. **semantic-release のブランチ切替リスク**: `branches` 設定が誤っていると main で再実行される。→ 緩和: `.releaserc.json` を `release/*` 固定にし、テストで dry-run を実施。
3. **直接 push 禁止による運用混乱**: 既存スクリプトが main へ push しようとして失敗。→ 緩和: CLI と docs を更新し、main push を検出したらガイドへ誘導する。

### 依存関係リスク

- **GitHub トークン権限**: `SEMANTIC_RELEASE_TOKEN` が release ブランチや PR Auto Merge に必要な `pull_request:write` を持たないと失敗。→ 緩和: トークン権限を文書化し不足時に早期検知。

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定（本計画で定義）
2. ✅ フェーズ1完了: データモデル / quickstart / 契約のアウトライン作成
3. ⏭️ `/speckit.tasks` を実行してタスクリストを生成
4. ⏭️ `/speckit.implement` で実装を開始
