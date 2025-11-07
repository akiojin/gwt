# 実装計画: releaseブランチ経由の自動リリース＆Auto Mergeフロー

**仕様ID**: `SPEC-57fde06f` | **日付**: 2025-11-07 | **仕様書**: [specs/SPEC-57fde06f/spec.md](./spec.md)
**入力**: `specs/SPEC-57fde06f/spec.md` からの機能仕様

## 概要

現状の `/release` コマンドと `release-trigger` ワークフローは develop→main を直接マージし main push で semantic-release を実行している。本計画では release ブランチを常設し、(1) `/release` が develop を release に fast-forward、(2) release ブランチ push をトリガーに semantic-release と検証ジョブを実行、(3) release→main PR を自動作成して Required チェック通過後に Auto Merge させる、という 3 層構造へ置き換える。GitHub Branch Protection で main への直接 push を禁止し、ドキュメントと CLI 手順を release フローへ合わせて更新する。

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
├── research.md              # Phase 0 で作成
├── data-model.md            # Phase 1 で作成
├── quickstart.md            # Phase 1 で作成
├── contracts/
│   └── release-automation.md
└── tasks.md                 # /speckit.tasks で生成
```

### ソースコード（リポジトリルート）

```text
.claude/commands/release.md          # CLIドキュメント
.github/workflows/release-trigger.yml
.github/workflows/release.yml        # semantic-release 実行ワークフロー
scripts/                             # release コマンド実装（gh呼び出し等）
CLAUDE.md / README.*                 # ルール・フローの周知
.releaserc.json                      # semantic-release 対象ブランチ定義
```

## フェーズ0: 調査（技術スタック選定）

**目的**: release ブランチを中心とした CI 設計を固め、gh CLI・GitHub API・semantic-release の設定ポイントを明確にする。

**出力**: `specs/SPEC-57fde06f/research.md`

### 調査項目

1. **既存のコードベース分析**
   - `/release` コマンドの実装箇所（scripts or CLAUDE command）とワークフロー連携
   - 現行 `release-trigger.yml` のステップと権限
   - `.releaserc.json` の `branches` 設定（現在は `main` のみ）

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

- `ReleaseBranchState`: develop との差分、バージョンタグ、semantic-release 成功状態
- `ReleasePullRequest`: release→main PR のラベル、Auto Merge 設定、Required チェック結果
- `RequiredCheck`: lint/test/semantic-release job の名前と Gate 条件
- `BranchProtectionConfig`: main/release 双方の保護設定（direct push 禁止、auto-merge 許可）

### 1.2 クイックスタートガイド

- `/release` コマンド実行前の前提（develop up-to-date, `git status` clean）
- release ブランチ同期コマンド例（gh / git）
- release→main PR 監視と Required チェック再実行フロー
- main で hotfix が必要なときのエスカレーション手順

### 1.3 契約/インターフェース

- release コマンド契約: 入力（develop HEAD, confirm flag）、出力（release push, PR URL）
- GitHub Actions 契約: release push 時の job 群、Required チェック ID、Artifact
- Branch Protection 契約: main への禁止アクション、Auto Merge 設定手順

### Agent Context Update

フェーズ1完了後に `SPECIFY_FEATURE=SPEC-57fde06f .specify/scripts/bash/update-agent-context.sh claude` を実行し、CLAUDE.md の Active Technologies / Recent Changes を release フロー内容で同期する。

## フェーズ2: タスク生成

- `/speckit.tasks` を実行し、P1（release ブランチ同期 + semantic-release 実行）、P1（Auto Merge & Required チェック構成）、P2（ドキュメント/ガバナンス更新）に分けたチェックリストを生成。
- tasks.md では CLI／ワークフロー／docs の並列実行可能タスクを明示する。

## 実装戦略

1. **P1-1: release ブランチ同期 + semantic-release 移行**
   - `/release` コマンドと `release-trigger.yml` を release ブランチ更新用に書き換え
   - `.releaserc.json` で `release` をリリース対象ブランチに設定
2. **P1-2: release→main Auto Merge**
   - release branch push 後に PR を作成/更新し `gh pr merge --auto --squash` または `github-script` で Auto Merge を ON
   - Branch Protection を Required チェック（lint/test/semantic-release）に制限
3. **P2: ドキュメントと運用**
   - CLAUDE.md / `.claude/commands/release.md` / README のリリースフロー節を更新
   - トラブルシュート（チェック失敗時のリカバリ）を quickstart.md と docs に反映

## テスト戦略

- **ユニットテスト**: release CLI (if scripted) の関数を Vitest でモックし develop→release push ロジックを検証。
- **統合テスト**: GitHub Actions の dry-run / `workflow_dispatch` で release trigger を実行し release branch push → release PR Auto Merge を確認。
- **エンドツーエンド**: 手動で `/release` を走らせ、release PR が Required チェック後に自動マージされることを監視。
- **パフォーマンス**: release job 所要時間と Auto Merge までの時間を計測し、SC-002 (≤10分) を満たす。

## リスクと緩和策

1. **Auto Merge が無効化されるリスク**: 既存 Branch Protection が Auto Merge を許さない場合。→ 緩和: 設定変更手順を docs に追加し、gh CLI で `gh pr merge --auto` を即時実行。
2. **semantic-release のブランチ切替リスク**: main 以外で実行するとタグ/CHANGELOG が release のみに残る。→ 緩和: release job 後に PR マージで main へ反映し、`.releaserc.json` の `branches` に `release` + `main` (maintenance) を記載。
3. **直接 push 禁止による運用混乱**: 既存スクリプトが main へ push しようとして失敗。→ 緩和: CLI と docs を更新し、CI で main push を検出したら失敗ロギング。

### 依存関係リスク

- **GitHub トークン権限**: `SEMANTIC_RELEASE_TOKEN` が release ブランチや PR Auto Merge に必要な `pull_request:write` を持たないと失敗。→ 緩和: トークン権限を文書化し不足時に早期検知。

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定（本計画で定義）
2. ✅ フェーズ1完了: データモデル / quickstart / 契約のアウトライン作成
3. ⏭️ `/speckit.tasks` を実行してタスクリストを生成
4. ⏭️ `/speckit.implement` で実装を開始
