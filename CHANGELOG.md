# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`.releaserc.json` による semantic-release 設定の明示化**
  - デフォルト設定への暗黙的な依存を排除
  - リリースプロセスの可視化と保守性向上
  - 全6つのプラグイン設定を明示的に定義 (commit-analyzer, release-notes-generator, changelog, npm, git, github)
- semantic-release と必要なプラグインを devDependencies に追加
- 完全なテストカバレッジ（104+ tests）
  - ユニットテスト: Git operations, Worktree management, UI components
  - 統合テスト: Branch selection, Remote branch handling, Branch creation workflows
  - E2Eテスト: Complete user workflows
- 包括的なドキュメント
  - API documentation (docs/api.md)
  - Architecture documentation (docs/architecture.md)
  - Contributing guidelines (CONTRIBUTING.md)
  - Troubleshooting guide (docs/troubleshooting.md)

### Changed

- **リリースプロセスのドキュメント化**
  - README.md にリリースプロセスセクションを追加
  - Conventional Commits のガイドライン記載
  - semantic-release の動作説明を追加
  - .releaserc.json の詳細説明を追加
  - リリースプロセスガイド (specs/SPEC-23bb2eed/quickstart.md) へのリンク追加
- テストフレームワークをVitestに移行
- CI/CDパイプラインの強化
- **bunx移行**: Claude Code起動方式をnpxからbunxへ完全移行
  - Claude Code: `bunx @anthropic-ai/claude-code@latest`で起動
  - Codex CLI: 既存のbunx対応を維持
  - UI表示文言をbunx表記へ統一

### Breaking Changes

- **Bun 1.0+が必須**: Claude Code起動にはBun 1.0.0以上が必要
- npx対応の廃止: `npx`経由でのClaude Code起動は非対応
- ユーザーへの移行ガイダンス:
  - Bunインストール: `curl -fsSL https://bun.sh/install | bash` (macOS/Linux)
  - Bunインストール: `powershell -c "irm bun.sh/install.ps1|iex"` (Windows)
  - エラー時に詳細なインストール手順を表示

## [0.6.1] - 2024-09-06

### Fixed
- Docker環境での動作改善
- パスハンドリングの修正

### Added
- Dockerサポートの完全実装
- Docker使用ガイド (docs/docker-usage.md)

## [0.6.0] - 2024-09-06

### Added
- @akiojin/spec-kit統合による仕様駆動開発サポート
- Codex CLI対応
  - Claude CodeとCodex CLIの選択機能
  - ワークツリー起動時のAIツール選択
  - `--tool`オプションによる直接指定

### Changed
- npmコマンドからnpx経由での実行に変更
- npxコマンドを最新版指定に更新

## [0.5.0] - 2024-08-XX

### Added
- セッション管理機能
  - `-c, --continue`: 最後のセッションを継続
  - `-r, --resume`: セッション選択して再開
  - セッション情報の永続化 (~/.config/claude-worktree/sessions.json)

### Changed
- Claude Code統合の改善
- UI/UXの向上

## [0.4.0] - 2024-07-XX

### Added
- GitHub PR統合
  - マージ済みPRの自動検出
  - ブランチとワークツリーの一括クリーンアップ
  - 未プッシュコミットの処理

### Changed
- エラーハンドリングの改善
- パフォーマンスの最適化

## [0.3.0] - 2024-06-XX

### Added
- スマートブランチ作成ワークフロー
  - feature/hotfix/releaseブランチタイプのサポート
  - releaseブランチでの自動バージョン管理
  - package.jsonの自動更新

### Changed
- ブランチタイプの自動検出
- ワークツリーパス生成ロジックの改善

## [0.2.0] - 2024-05-XX

### Added
- ワークツリー管理機能
  - 既存ワークツリーの一覧表示
  - ワークツリーの開く/削除操作
  - ブランチも含めた削除オプション

### Changed
- CLI UIの改善
- エラーメッセージの分かりやすさ向上

## [0.1.0] - 2024-04-XX

### Added
- 対話型ブランチ選択
  - ローカル・リモートブランチの統合表示
  - ブランチタイプ別の視覚的識別
  - 既存ワークツリーの表示
- ワークツリー自動作成
  - ブランチ選択からワークツリー作成まで
  - 自動パス生成 (.git/worktree/)
- Claude Code統合
  - ワークツリー作成後の自動起動
  - 引数パススルー機能
- 変更管理
  - AIツール終了後の未コミット変更検出
  - commit/stash/discard オプション

### Technical
- TypeScript 5.8.3
- Bun 1.3.1+ サポート（必須ランタイム）
- Node.js 18+ サポート（開発ツール向けオプション）
- Git 2.25+ 必須
- execa for Git command execution
- inquirer for interactive prompts

## [0.0.1] - 2024-03-XX

### Added
- 初期リリース
- 基本的なワークツリー管理機能

---

## Release Process

リリースは自動化されています:

1. PRがmainブランチにマージ
2. GitHub Actionsがテスト実行
3. Semantic Releaseがコミットメッセージからバージョンを決定
4. npmに自動公開
5. このCHANGELOG.mdが自動更新
6. GitHubリリースノート自動生成

## Migration Guides

### v0.6.x → v0.7.x (Unreleased)

Breaking changes: なし

新機能:
- テストスイートの追加（ユーザーへの影響なし）
- ドキュメントの拡充

推奨アクション:
- 特になし、通常通りアップグレード可能

### v0.5.x → v0.6.x

Breaking changes: なし

新機能:
- Codex CLI対応
- Docker対応

推奨アクション:
- Codex CLIを使用したい場合は`codex`コマンドをインストール
- Docker環境で使用したい場合はdocs/docker-usage.mdを参照

### v0.4.x → v0.5.x

Breaking changes: なし

新機能:
- セッション管理 (-c, -r オプション)

推奨アクション:
- セッション機能を活用して開発効率を向上

## Deprecation Notices

現在、非推奨となっている機能はありません。

## Known Issues

See [GitHub Issues](https://github.com/akiojin/claude-worktree/issues) for current known issues.

## Links

- [Repository](https://github.com/akiojin/claude-worktree)
- [npm Package](https://www.npmjs.com/package/@akiojin/claude-worktree)
- [Documentation](https://github.com/akiojin/claude-worktree/tree/main/docs)
- [Issue Tracker](https://github.com/akiojin/claude-worktree/issues)
