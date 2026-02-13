# 仕様一覧

**最終更新**: 2026-02-14

- 現行の仕様・要件: `specs/SPEC-XXXXXXXX/spec.md`
- 以前までのTUIの仕様・要件（archive）: `specs/archive/SPEC-XXXXXXXX/spec.md`

## 運用ルール

- `カテゴリ: GUI` は、現行のTauri GUI実装で有効な要件（binding）です。
- `カテゴリ: Porting` は、TUI/WebUI由来の移植待ち（non-binding）です。未実装でも不具合ではありません。
- Porting を実装対象にする場合は、次のどちらかを実施します:
1. 既存 spec の内容を GUI 前提に更新し、`カテゴリ` を `GUI` に変更する
2. 新しい GUI spec を作成し、元の Porting spec を `**依存仕様**:` で参照する

## 現行仕様（GUI）

| SPEC ID | タイトル | 作成日 |
| --- | --- | --- |
| [SPEC-d6949f99](SPEC-d6949f99/spec.md) | 機能仕様: PR Status Preview: Worktreeツリー上でPR/CIステータスをプレビュー表示 | 2026-02-14 |
| [SPEC-4e2f1028](SPEC-4e2f1028/spec.md) | 機能仕様: Windows 移行プロジェクトで Docker 起動時に mount エラーを回避する | 2026-02-13 |
| [SPEC-6e2a9d4c](SPEC-6e2a9d4c/spec.md) | 機能仕様: Host OS 起動時の空タブ防止（Issue #1029） | 2026-02-13 |
| [SPEC-6f291006](SPEC-6f291006/spec.md) | 機能仕様: Migration backup copy の Windows 互換修正 | 2026-02-13 |
| [SPEC-7d1a4b2e](SPEC-7d1a4b2e/spec.md) | 機能仕様: Playwrightベースの実装テスト基盤整備（WebView UI） | 2026-02-13 |
| [SPEC-8c6f4a21](SPEC-8c6f4a21/spec.md) | 機能仕様: Windows での外部プロセス実行時コンソール点滅抑止 | 2026-02-13 |
| [SPEC-9f3c2a11](SPEC-9f3c2a11/spec.md) | 機能仕様: Voice Input Mode（GUI） | 2026-02-13 |
| [SPEC-a3daf499](SPEC-a3daf499/spec.md) | 機能仕様: 起動時アプリ更新チェックの取りこぼし防止（遅延 + 再試行） | 2026-02-13 |
| [SPEC-b7f7b9ad](SPEC-b7f7b9ad/spec.md) | 機能仕様: gwt メニューショートカットと終了保護 | 2026-02-13 |
| [SPEC-c6ba640a](SPEC-c6ba640a/spec.md) | 機能仕様: GitHub Issue連携によるブランチ作成（GUI版） | 2026-02-13 |
| [SPEC-d7f2a1b3](SPEC-d7f2a1b3/spec.md) | バグ修正仕様: Cleanup「Select All Safe」が機能しない | 2026-02-13 |
| [SPEC-f466bc68](SPEC-f466bc68/spec.md) | 機能仕様: プロジェクトを開いたときに前回のエージェントタブを復元する | 2026-02-13 |
| [SPEC-f490dded](SPEC-f490dded/spec.md) | 機能仕様: シンプルターミナルタブ | 2026-02-13 |
| [SPEC-3a1b7c2d](SPEC-3a1b7c2d/spec.md) | 機能仕様: GUI Worktree Summary のスクロールバック要約（実行中対応） | 2026-02-12 |
| [SPEC-133bf64f](SPEC-133bf64f/spec.md) | 機能仕様: Project Version History（タグ単位のAI要約 + 簡易CHANGELOG） | 2026-02-10 |
| [SPEC-1b98b6d7](SPEC-1b98b6d7/spec.md) | 機能仕様: Claude Code Hooks 経由の gwt-tauri hook 実行で GUI を起動しない | 2026-02-10 |
| [SPEC-ab9b7d08](SPEC-ab9b7d08/spec.md) | 機能仕様: Aboutダイアログにバージョン表示 + タイトルにプロジェクトパス表示（GUI） | 2026-02-10 |
| [SPEC-b28ab8d9](SPEC-b28ab8d9/spec.md) | 機能仕様: CI の Node ツールチェーンを pnpm に統一する（gwt-gui + commitlint） | 2026-02-10 |
| [SPEC-c4e8f210](SPEC-c4e8f210/spec.md) | 機能仕様: Worktree Cleanup（GUI） | 2026-02-10 |
| [SPEC-4470704f](SPEC-4470704f/spec.md) | 機能仕様: gwt GUI マルチウィンドウ + Native Windowメニュー | 2026-02-09 |
| [SPEC-5b7a0f9c](SPEC-5b7a0f9c/spec.md) | 機能仕様: Terminal ANSI Diagnostics（GUI） | 2026-02-09 |
| [SPEC-ba3f610c](SPEC-ba3f610c/spec.md) | 機能仕様: エージェントモード | 2026-01-22 |
| [SPEC-735cbc5d](SPEC-735cbc5d/spec.md) | GitView in Worktree Summary Panel | - |

## 移植待ち（Porting）

| SPEC ID | タイトル | 作成日 |
| --- | --- | --- |

## 過去要件（archive）

| SPEC ID | タイトル | 作成日 |
| --- | --- | --- |
| [SPEC-488af8e2](archive/SPEC-488af8e2/spec.md) | 機能仕様: gwt GUI Docker Compose 統合（起動ウィザード + Quick Start） | 2026-02-09 |
| [SPEC-86bb4e7c](archive/SPEC-86bb4e7c/spec.md) | 機能仕様: gwt GUI ブランチフィルター修正・エージェント起動ウィザード・Profiles設定 | 2026-02-09 |
| [SPEC-90217e33](archive/SPEC-90217e33/spec.md) | 機能仕様: gwt GUI コーディングエージェント機能のTUI完全移行（Quick Start / Mode / Skip / Reasoning / Version） | 2026-02-09 |
| [SPEC-1ad9c07d](archive/SPEC-1ad9c07d/spec.md) | 機能仕様: エージェント起動ウィザード統合 | 2026-02-08 |
| [SPEC-1d6dd9fc](archive/SPEC-1d6dd9fc/spec.md) | 機能仕様: マルチターミナル（gwt内蔵ターミナルエミュレータ） | 2026-02-08 |
| [SPEC-d6210238](archive/SPEC-d6210238/spec.md) | 機能仕様: TUI→Tauri GUI完全移行 Phase 1: 基盤構築 | 2026-02-08 |
| [SPEC-dfb1611a](archive/SPEC-dfb1611a/spec.md) | 機能仕様: gwt GUI プロジェクト管理 Phase 2 追加機能 | 2026-02-08 |
| [SPEC-92053c0d](archive/SPEC-92053c0d/spec.md) | 機能仕様: commitlint を npm ci 無しで実行可能にする | 2026-02-03 |
| [SPEC-f5f5657e](archive/SPEC-f5f5657e/spec.md) | 機能仕様: Docker環境統合（エージェント自動起動） | 2026-02-03 |
| [SPEC-1ea18899](archive/SPEC-1ea18899/spec.md) | 機能仕様: GitView画面 | 2026-02-02 |
| [SPEC-a3f4c9df](archive/SPEC-a3f4c9df/spec.md) | 機能仕様: 設定ファイル統合・整理 | 2026-02-02 |
| [SPEC-a70a1ece](archive/SPEC-a70a1ece/spec.md) | 機能仕様: bareリポジトリ対応とヘッダーブランチ表示 | 2026-02-01 |
| [SPEC-f8dab6e2](archive/SPEC-f8dab6e2/spec.md) | 機能仕様: Claude Code プラグインマーケットプレイス自動登録 | 2026-01-30 |
| [SPEC-71f2742d](archive/SPEC-71f2742d/spec.md) | 機能仕様: カスタムコーディングエージェント登録機能 | 2026-01-26 |
| [SPEC-f59c553d](archive/SPEC-f59c553d/spec.md) | 機能仕様: npm postinstall ダウンロード安定化 | 2026-01-26 |
| [SPEC-2ca73d7d](archive/SPEC-2ca73d7d/spec.md) | 機能仕様: エージェント履歴の永続化 | 2026-01-22 |
| [SPEC-e66acf66](archive/SPEC-e66acf66/spec.md) | 機能仕様: エラーポップアップ・ログ出力システム | 2026-01-22 |
| [SPEC-067a8026](archive/SPEC-067a8026/spec.md) | 機能仕様: LLMベースリリースワークフロー | 2026-01-21 |
| [SPEC-861d8cdf](archive/SPEC-861d8cdf/spec.md) | 機能仕様: エージェント状態の可視化 | 2026-01-20 |
| [SPEC-4b893dae](archive/SPEC-4b893dae/spec.md) | 機能仕様: ブランチサマリーパネル | 2026-01-19 |
| [SPEC-6408df0c](archive/SPEC-6408df0c/spec.md) | 機能仕様: HuskyでCIと同等のLintを実行 | 2026-01-19 |
| [SPEC-b7bde3ff](archive/SPEC-b7bde3ff/spec.md) | 機能仕様: tmuxマルチモードサポート | 2026-01-18 |
| [SPEC-925c010b](archive/SPEC-925c010b/spec.md) | 機能仕様: Docker Compose の Playwright noVNC を arm64 で起動可能にする | 2026-01-17 |
| [SPEC-77b1bc70](archive/SPEC-77b1bc70/spec.md) | 機能仕様: リリースフロー要件の明文化とリリース開始時 main→develop 同期 | 2026-01-16 |
| [SPEC-902a89dc](archive/SPEC-902a89dc/spec.md) | 機能仕様: Worktreeパス修復機能 | 2026-01-05 |
| [SPEC-29e16bd0](archive/SPEC-29e16bd0/spec.md) | 機能仕様: tools.json スキーママイグレーション | 2026-01-04 |
| [SPEC-d27be71b](archive/SPEC-d27be71b/spec.md) | SPEC-d27be71b: Ink.js から OpenTUI への移行 | 2026-01-04 |
| [SPEC-c1d5bad7](archive/SPEC-c1d5bad7/spec.md) | ログ一覧・詳細表示・クリップボードコピー機能 | 2025-12-25 |
| [SPEC-a0d7334d](archive/SPEC-a0d7334d/spec.md) | 機能仕様: Dependabot PR の向き先を develop に固定 | 2025-12-22 |
| [SPEC-b0b1b0b1](archive/SPEC-b0b1b0b1/spec.md) | 機能仕様: AIツール終了時の未コミット警告と未プッシュ確認 | 2025-12-20 |
| [SPEC-96e694b4](archive/SPEC-96e694b4/spec.md) | 機能仕様: Codex CLI gpt-5.3-codex 追加（spark 含む） | 2025-12-18 |
| [SPEC-dafff079](archive/SPEC-dafff079/spec.md) | 機能仕様: 環境変数プロファイル機能 | 2025-12-15 |
| [SPEC-1f56fd80](archive/SPEC-1f56fd80/spec.md) | 機能仕様: Web UI システムトレイ統合 | 2025-12-12 |
| [SPEC-c8e7a5b2](archive/SPEC-c8e7a5b2/spec.md) | 機能仕様: CLI起動時Web UIサーバー自動起動 | 2025-12-12 |
| [SPEC-b9f5c4a1](archive/SPEC-b9f5c4a1/spec.md) | ログ運用統一仕様 | 2025-12-11 |
| [SPEC-40c7b4f1](archive/SPEC-40c7b4f1/spec.md) | 機能仕様: ブランチ選択時のdivergence/FF失敗ハンドリング（起動継続） | 2025-12-09 |
| [SPEC-3b0ed29b](archive/SPEC-3b0ed29b/spec.md) | 機能仕様: コーディングエージェント対応 | 2025-12-06 |
| [SPEC-f47db390](archive/SPEC-f47db390/spec.md) | 機能仕様: セッションID永続化とContinue/Resume強化 | 2025-12-06 |
| [SPEC-d2f4762a](archive/SPEC-d2f4762a/spec.md) | 機能仕様: ブランチ選択画面（Branch Selection Screen） | 2025-11-18 |
| [SPEC-5e0c1c49](archive/SPEC-5e0c1c49/spec.md) | 機能仕様: Codex CLI gpt-5.1 デフォルト更新 | 2025-11-14 |
| [SPEC-33317a3c](archive/SPEC-33317a3c/spec.md) | 機能仕様: 共通環境変数とローカル環境取り込み機能 | 2025-11-11 |
| [SPEC-8adfd99e](archive/SPEC-8adfd99e/spec.md) | 機能仕様: Web UI 環境変数編集機能 | 2025-11-11 |
| [SPEC-55fe506f](archive/SPEC-55fe506f/spec.md) | 機能仕様: Worktreeクリーンアップ選択機能 | 2025-11-10 |
| [SPEC-e4798383](archive/SPEC-e4798383/spec.md) | 機能仕様: GitHub Issue連携によるブランチ作成 | 2025-01-25 |
| [SPEC-1defd8fd](archive/SPEC-1defd8fd/spec.md) | 機能仕様: bugfixブランチタイプのサポート追加 | 2025-01-18 |
| [SPEC-1d62511e](archive/SPEC-1d62511e/spec.md) | SPEC-1d62511e: TypeScript/Bun から Rust への完全移行 | - |
| [SPEC-62c129ca](archive/SPEC-62c129ca/spec.md) | SPEC-62c129ca: ブランチリストのマウスクリック動作改善 | - |
| [SPEC-fdebd681](archive/SPEC-fdebd681/spec.md) | Codex collaboration_modes Support | - |
