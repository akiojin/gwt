# CLAUDE.md

このファイルは、このリポジトリでコードを扱う際のガイダンスを提供します。

## 開発指針

### 🛠️ 技術実装指針

- **設計・実装は複雑にせずに、シンプルさの極限を追求してください**
- **ただし、ユーザビリティと開発者体験の品質は決して妥協しない**
- 実装はシンプルに、開発者体験は最高品質に
- CLI操作の直感性と効率性を技術的複雑さより優先

### 📝 設計ガイドライン

- 設計に関するドキュメントには、ソースコードを書かないこと

## 開発品質

### 完了条件

- エラーが発生している状態で完了としないこと。必ずエラーが解消された時点で完了とする。

## 開発ワークフロー

### 基本ルール

- 作業（タスク）を完了したら、変更点を日本語でコミットログに追加して、コミット＆プッシュを必ず行う
- 作業（タスク）は、最大限の並列化をして進める
- 作業（タスク）は、最大限の細分化をしてToDoに登録する
- Spec Kitを用いたSDD/TDDの絶対遵守を義務付ける。Spec Kit承認前およびSpec準拠のTDD完了前の実装着手を全面禁止し、違反タスクは即時差し戻す。
- 作業（タスク）の開始前には、必ずToDoを登録した後に作業を開始する
- 作業（タスク）は、忖度なしで進める
- **エージェントはユーザーからの明示的な指示なく新規ブランチの作成・削除を行ってはならない。Worktreeは起動ブランチで作業を完結する設計。**

### コミットメッセージポリシー

- semantic-release によりバージョン判定とリリースノート生成を自動化しているため、コミットメッセージは必ず Conventional Commits 形式で記述する（例: `feat: ...`、`fix: ...`、`docs: ...`）。
- リリース種別はプレフィックスで決定される。`feat:` はマイナーバージョン、`fix:` はパッチ、`type!:` または本文の `BREAKING CHANGE:` はメジャーバージョンとして扱われる。
- `chore:` や `docs:` などリリース対象外のタイプでも必ずプレフィックスを付け、曖昧な自然文だけのコミットメッセージを禁止する。
- 1コミットで複数タスクを抱き合わせない。変更内容とコミットメッセージの対応関係を明確に保ち、semantic-release の解析精度を担保する。
- コミット前に commitlint ルール（subject 空欄禁止・100文字以内など）を自己確認し、CI での差し戻しを防止する。

### ローカル検証/実行ルール（bun）

- このリポジトリのローカル検証・実行は bun を使用する
- 依存インストール: `bun install`
- ビルド: `bun run build`
- 実行: `bunx .`（一発実行）または `bun run start`
- グローバル実行: `bun add -g @akiojin/claude-worktree` → `claude-worktree`
- CI/CD・Docker環境ではpnpmを使用（ハードリンクによるnode_modules効率化のため）

## コミュニケーションガイドライン

- 回答は必ず日本語

## ドキュメント管理

- ドキュメントはREADME.md/README.ja.mdに集約する

## コードクオリティガイドライン

- マークダウンファイルはmarkdownlintでエラー及び警告がない状態にする
- コミットログはcommitlintに対応する

## 開発ガイドライン

- Spec Kitを用いたSDD/TDDの絶対遵守を組織ルールとする。Spec Kit外での設計・テスト着手、およびSpec未承認・TDD未完了での実装開始を検知次第即時差し戻す。
- 既存のファイルのメンテナンスを無視して、新規ファイルばかり作成するのは禁止。既存ファイルを改修することを優先する。

## ドキュメント作成ガイドライン

- README.mdには設計などは書いてはいけない。プロジェクトの説明やディレクトリ構成などの説明のみに徹底する。設計などは、適切なファイルへのリンクを書く。

## リリースワークフロー

- feature/\* ブランチは develop へ Auto Merge し、develop で次回リリース候補を蓄積する。
- `/release` コマンド（または `gh workflow run create-release.yml --ref develop`）で semantic-release のドライランを実行し、次のバージョンを決定して `release/vX.Y.Z` ブランチを自動作成する。
- `release/vX.Y.Z` ブランチへの push をトリガーに `.github/workflows/release.yml` が以下を実行：
  1. semantic-release で CHANGELOG/タグ/GitHub Release を作成
  2. `release/vX.Y.Z` → `main` へ直接マージ
  3. `main` → `develop` へバックマージ
  4. `release/vX.Y.Z` ブランチを削除
- すべての処理が release.yml 内で完結し、PR を経由せずに高速に実行される。
- main への push をトリガーに `.github/workflows/publish.yml` が npm publish（設定時）を実行する。

## 最近の変更

### 2025-01-06: Codex CLI対応機能の計画

- worktree起動時にClaude CodeとCodex CLIを選択可能にする機能を計画中
- 詳細: `/specs/001-codex-cli-worktree/`
- 技術スタック: Bun 1.0+, TypeScript, inquirer（必要に応じてNode.js 18+を併用）

### 2025-01-07: unity-mcp-server型リリースフロー完全導入

- unity-mcp-serverの直接マージ方式を完全導入（PRを経由せず高速化）
- release.yml内で semantic-release → main直接マージ → developバックマージ → ブランチ削除を一括実行
- PRベース方式から直接マージ方式に変更し、シンプルで高速なリリースフローを実現
- 詳細: `.github/workflows/create-release.yml`, `.github/workflows/release.yml`, `.github/workflows/publish.yml`

### 2025-01-06: リリースフロー変更

- develop ブランチを導入し、手動リリースフローに移行
- feature → develop (Auto Merge) → /release（develop→main PR）→ main push → semantic-release
- 詳細: `.github/workflows/release-trigger.yml`, `.claude/commands/release.md`, `scripts/create-release-pr.sh`

## Active Technologies

- TypeScript 5.8.x / React 19 / Ink 6 / Bun 1.0+ + Vitest 2.1.x, happy-dom 20.0.8, @testing-library/react 16.3.0, execa 9.6.0 (SPEC-a5a44f4c)
- TypeScript 5.8.x / Bun 1.0+ / GitHub Actions YAML + semantic-release 22.x, gh CLI, GitHub Actions (`actions/checkout`, `actions/github-script`) (SPEC-57fde06f)

## Recent Changes

- SPEC-a5a44f4c: Added TypeScript 5.8.x / React 19 / Ink 6 / Bun 1.0+ + Vitest 2.1.x, happy-dom 20.0.8, @testing-library/react 16.3.0, execa 9.6.0
