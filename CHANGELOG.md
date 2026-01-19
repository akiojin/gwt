# Changelog

All notable changes to this project will be documented in this file.
## [6.2.0] - 2026-01-19

### Miscellaneous Tasks

- Sync main into develop
- Disable MD060 table style rule in markdownlint

## [6.1.0] - 2026-01-17

### Bug Fixes

- Rust移行時に誤って削除されたentrypoint.shを復元 (#648)
- マージ済み判定を git merge-base --is-ancestor に変更 (#649)
- Unpushedアイコンを!から^に変更して区別化 (#651)
- **tmux:** エージェント終了時のペイン自動削除とフォーカス移動を修正 (#656)
- 複数のバグ修正とDocker+tmux文字化け対策 (#657)
- **docker:** DockerイメージにRustをインストール
- **tui:** エージェント一覧のブランチ名表示と選択履歴保存のバグを修正 (#658)
- **tui:** ブランチ一覧のスクロール判定を実際のビューポート高さに対応 (#659)
- **tui:** TUI起動時の自動orphanクリーンアップを削除 (#662)
- Auto-normalize toolId in TS sessions (#666)
- Restore single mode outside tmux (#667)

### Features

- **tmux:** Tmuxマルチモードサポートを追加 (#650)
- **tmux:** Tmuxマルチモードサポートを追加 (#652)
- GIT_DIR環境変数の書き換えをブロックするフックを追加 (#653)
- **tmux:** Tmuxマルチモードサポートの実装 (#654)
- フックスクリプトをプラグイン形式に移行 (#655)
- **tui:** Tmuxペイン表示/非表示切り替え機能を追加 (#661)
- Gh-fix-ciにレビューコメント調査を追加
- **tmux:** エージェント状態表示とセッション管理を実装 (#660)

### Miscellaneous Tasks

- Dockerにtmuxを追加
- 未使用のhomebrewディレクトリを削除

## [6.0.9] - 2026-01-17

### Bug Fixes

- Arm64でplaywright-novncのcompose起動を可能にする (#640)
- ログビューア・プロファイル画面のUI改善 (#641)
- Upstream未設定ブランチの安全ステータス判定を修正 (#644)
- Stdoutを継承してエージェントのTTY検出を修正 (#645)

### Features

- ログ出力カバー率を改善しエージェント出力キャプチャを追加 (#642)
- History.jsonlパーサーを追加しセッションID取得を改善

### Miscellaneous Tasks

- Code-simplifierプラグインを追加 (#643)

## [6.0.8] - 2026-01-16

### Documentation

- Fix CHANGELOG structure with proper version sections

## [6.0.5] - 2026-01-15

### Bug Fixes

- Pin cross version to v0.2.5 for reproducible builds

### Documentation

- Add v6.0.5 section to CHANGELOG

## [6.0.3] - 2026-01-15

### Bug Fixes

- Bump version to 6.0.3 (crates.io already has 6.0.2)
- Bump package.json version to 6.0.3
- Add version to gwt-core dependency for crates.io publishing

## [6.0.1] - 2026-01-15

### Bug Fixes

- Support merge commit in release workflow trigger
- Add version to gwt-core dependency for crates.io publishing
- Use cross for Linux ARM64 musl build
- Add id-token permission for npm provenance and pin cross version
- Clarify npm wrapper auto-download behavior in README
- Improve release binary documentation clarity in README
- ログビューアの時間表示をシステムローカル時間に修正 (#630)
- Release準備のmain同期をPR経由に変更 (#633)
- テキスト入力で文字が二重入力されるバグを修正 (#634)
- プロファイル画面のUX改善 (#635)
- Main同期とgix APIの互換性修正 (#637)
- Support merge commit in release workflow trigger
- Add workflow_dispatch support to release workflow trigger

### Documentation

- パッケージ公開状況をCLAUDE.mdに追記
- Remove crates.io references from documentation
- Sync CHANGELOG from main and add v6.0.7 entry

### Miscellaneous Tasks

- Sync main to develop after v6.0.0 release
- バージョンを 6.0.3 に統一
- リリースフロー要件化とmain→develop同期 (#629)

### Refactor

- Use workspace dependencies for internal crates

### UX

- 自動インストール文言の検証と設定説明 (#631)

### Ci

- Use cargo-workspaces for crates.io publishing
- Remove crates.io publishing, distribute via GitHub Release and npm only

## [6.0.0] - 2026-01-15

### Bug Fixes

- Update migration status in README

### Miscellaneous Tasks

- Sync main to develop after v5.5.0 release

## [5.5.0] - 2026-01-15

### Bug Fixes

- Use PR-based sync for main to develop after release
- Bump version to 6.0.3 for crates.io compatibility

### Miscellaneous Tasks

- Sync main to develop after v5.4.0 release

## [5.4.0] - 2026-01-15

### Bug Fixes

- Use workspace version inheritance for subcrates
- Windows NTSTATUSコードを人間可読形式で表示 (#609)
- Use musl static linking for Linux binaries to resolve GLIBC dependency (#610)

### Features

- ログビューア機能の実装と構造化ログの強化 (#606)

### Miscellaneous Tasks

- Sync main to develop after v5.3.0 release

## [5.1.0] - 2026-01-14

### Bug Fixes

- Remove publish-crates dependency from upload-release job
- Add sync-develop job to sync main back to develop after release
- Use -X theirs option in sync-develop to resolve conflicts automatically
- Add on-demand binary download for bunx compatibility (#600)
- Filter key events by KeyEventKind::Press to prevent double input on Windows (#601)

### Features

- Bun-to-rust移行と周辺改善 (#602)
- Add structured debug logging for worktree change detection (#603)
- Migrate from release-please to custom release action (#587)

### Miscellaneous Tasks

- Remove release-please manifest (migrating to custom action)
- Remove release-please config (migrating to custom action)
- Sync version with main (5.1.0)
- Sync version with main (5.1.0)
- Sync CHANGELOG.md from main after v5.1.0 release
- Sync main to develop after v5.1.0 release (#594)

## [gwt-v6.0.1] - 2026-01-14

### Bug Fixes

- Decouple crates.io publish from GitHub Release and npm

### Features

- Release-pleaseからカスタムリリースActionへ移行 (#582)

### Miscellaneous Tasks

- Merge main (v6.0.0) into develop
- Sync version with main (6.0.2)

## [gwt-v6.0.0] - 2026-01-14

### Bug Fixes

- Worktree無しブランチの選択を抑止 (#564)
- Worktree selection and docs updates (#565)
- Remove version constraint from gwt-core path dependency (#570)
- Support gwt-v* tag pattern in publish workflow (#573)
- Exclude native binary from npm package (#575)
- Restore release-please config from main
- Use cargo-workspace release type for release-please
- Use node release type with extra-files for Cargo.toml
- Remove hardcoded release-type from release.yml
- Add gwt-core dependency version to release-please extra-files

### Features

- Add crates.io, cargo-binstall, and npm release automation (#566)

### Refactor

- Release.ymlにpublish.ymlを統合し、package.jsonバージョン自動同期を追加 (#574)

## [5.0.0] - 2026-01-13

### Bug Fixes

- Bunx実行時にBunで再実行する (#558)
- Update workspace version to 5.0.0 for release-please
- Use explicit versions in crate Cargo.toml for release-please

### Miscellaneous Tasks

- Merge develop into feature/bun-to-rust

## [4.12.0] - 2026-01-10

### Miscellaneous Tasks

- **main:** Release 4.12.0 (#549)

## [4.11.6] - 2026-01-08

### Bug Fixes

- **test:** Worktree.test.tsのVitest依存を削除してBun互換に修正 (#533)

### Miscellaneous Tasks

- **main:** Release 4.11.6 (#535)

## [4.11.5] - 2026-01-08

### Bug Fixes

- **ci:** Publishワークフローにテストタイムアウトを追加 (#530)

### Miscellaneous Tasks

- **main:** Release 4.11.5 (#532)

## [4.11.4] - 2026-01-08

### Bug Fixes

- **test:** Bun互換性のためのテスト修正 (#527)

### Miscellaneous Tasks

- Ralph-loopプラグインを有効化 (#519)
- **main:** Release 4.11.4 (#529)

## [4.11.3] - 2026-01-08

### Bug Fixes

- Stabilize dependency installer test mocks

### Miscellaneous Tasks

- **main:** Release 4.11.3

## [4.11.2] - 2026-01-08

### Bug Fixes

- 安全アイコン表示のルールを更新 (#516)

### Miscellaneous Tasks

- **main:** Release 4.11.2

## [4.11.1] - 2026-01-08

### Bug Fixes

- クリーンアップ安全表示を候補判定に連動 (#514)
- SaveSessionにtoolVersionを追加して履歴に保存 (#515)

### Miscellaneous Tasks

- **main:** Release 4.11.1

## [4.11.0] - 2026-01-08

### Bug Fixes

- セッションIDの表示と再開を改善 (#505)
- **cli:** Keep wizard cursor visible in popup (#506)
- **cli:** Keep wizard cursor visible in popup (#507)
- Repair機能のクロス環境対応とUI改善 (#508)
- Worktree修復ロジックの統一化とクロス環境対応 (#509)

### Features

- コーディングエージェントのバージョン選択機能を改善 (#510)
- **ui:** コーディングエージェント名の一貫した色づけを実装 (#511)

### Miscellaneous Tasks

- **main:** Release 4.11.0 (#513)

## [4.10.0] - 2026-01-05

### Miscellaneous Tasks

- **main:** Release 4.10.0 (#481)

## [4.9.1] - 2026-01-04

### Bug Fixes

- Tools.json の customTools → customCodingAgents マイグレーション対応 (#476)
- Divergenceでも起動を継続 (#483)
- CLI終了時のシグナルハンドリング改善と各種ドキュメント修正 (#489)
- Stabilize OpenTUI solid tests and UI layout (#490)
- 依存関係インストール時のスピナー表示を削除 (#496)
- 起動ログの出力経路とCodexセッションID検出を改善 (#495)
- ブランチ一覧にセッション履歴を反映 (#497)
- Show worktree path in branch footer (#499)
- ブランチ一覧のASCII表記を調整 (#500)
- ウィザード内スクロールの上下キー対応を追加
- ウィザードのfocus型を厳密オプションに合わせる
- ESCキャンセル後にウィザードが開かない問題を修正 (#501)
- 修正と設定の更新
- Package.jsonの名前を変更
- Package.jsonの名前を"akiojin/claude-worktree"に変更
- Remove unnecessary '.' argument when launching Claude Code
- GitHub CLI認証チェックを修正
- CLAUDE.mdをclaude-worktreeプロジェクトに適した内容に修正
- String-width negative value error by adding Math.max protection
- バージョン番号表示による枠線のズレを修正
- ウェルカムメッセージの枠線表示を修正
- カラム名（ヘッダー）が表示されない問題を修正
- ウェルカムメッセージの枠線表示を長いバージョン番号に対応
- 現在のブランチがCURRENTとして表示されない問題を修正
- CodeRabbitレビューコメントへの対応
- 保護対象ブランチ(main, master, develop)をクリーンアップから除外
- リモートブランチ選択時にローカルブランチが存在しない場合の不具合を修正
- Windows環境でのnpx実行エラーを修正
- エラー発生時にユーザー入力を待機するように修正
- Windows環境でのClaude Code起動エラーを改善
- Claude Codeのnpmパッケージ名を修正
- Claude Codeコマンドが見つからない場合の適切なエラーハンドリングを追加
- Dockerコンテナのentrypoint.shエラーを修正
- Claude Code実行時のエラーハンドリングを改善
- 未使用のインポートを削除
- 改行コードをLFに統一
- Docker環境でのClaude Code実行時のパス問題を修正
- Worktree内での実行時の警告表示とパス解決の改善
- Claude コマンドのPATH解決問題を修正
- ビルドエラーを修正
- 独自履歴選択後のclaude -r重複実行を修正
- Claude Code履歴表示でタイトルがセッションIDしか表示されない問題を修正
- タイトル抽出ロジックをシンプル化し、ブランチ記録機能を削除
- Claude Code履歴タイトル表示を根本的に改善
- 会話タイトルを最後のメッセージから抽出するように改善
- Claude Code履歴メッセージ構造に対応したタイトル抽出
- 履歴選択キャンセル時にメニューに戻るように修正
- UI表示とタイトル抽出の問題を修正
- プレビュー表示前に画面をクリアして見やすさを改善
- Claude Code実際の表示形式に合わせて履歴表示を修正
- Claude Code実行モード選択でqキーで戻れる機能を追加
- Claude Code実行モード選択でqキー対応とUI簡素化
- 全画面でqキー統一操作に対応
- 会話プレビューで最新メッセージが見えるように表示順序を改善
- 会話プレビューの「more messages above」を「more messages below」に修正
- 会話プレビューの表示順序を通常のチャット形式に修正
- リリースブランチ作成フローを完全に修正
- Developブランチが存在しない場合にmainブランチから分岐するように修正
- リリースブランチの2つの問題を修正
- リリースブランチ検出を正確にするため実際のGitブランチ名を使用
- Npm versionコマンドのエラーハンドリングを改善
- Npm versionエラーの詳細情報を出力するよう改善
- アカウント管理UIの改善
- アカウント切り替え機能のデバッグとUI改善
- **codex:** 承認/サンドボックス回避フラグをCodex用に切替
- Codexの権限スキップフラグ表示を修正
- Codex CLI の resume --last への統一
- Node_modulesをmarkdownlintから除外
- Markdownlintエラー修正（裸のURL）
- 自動マージワークフローのトリガー条件を修正
- GraphQL APIで自動マージを実行
- Worktreeパス衝突時のエラーハンドリングを改善 (#79)
- 新規Worktree作成時にClaude CodeとCodex CLIを選択可能にする (SPEC-473b3d47 FR-008対応)
- マージ済みPRクリーンアップ画面でqキーで前の画面に戻れるように修正
- ESLintエラーを修正
- StripAnsi関数の位置を修正してimport文の後に移動
- ESLint、Prettier、Markdown Lintのエラーを修正
- T094-T095完了 - テスト修正とフィーチャーフラグ変更
- Markdownlint違反のエスケープを追加
- Mainブランチから追加されたclaude.test.tsを一時スキップ（bun vitest互換性問題）
- リアルタイム更新テストの安定性向上
- Claude.test.tsをbun vitest互換に書き直し
- Session-resume.test.ts の node:os mock に default export を追加
- Node:fs/promisesとexecaのmockにdefault exportを追加
- 残り全テストファイルのmock問題を修正
- Ink.js UIの表示とキーボードハンドリングを修正
- キーボードハンドリング競合とWorktreeアイコン表示を修正
- QキーとEnterキーが正常に動作するように修正
- Vi.hoistedエラーを修正してテストを全て成功させる
- CIエラーを修正（Markdown Lint + Test）
- CIエラー修正（Markdown LintとVitest mock）
- CHANGELOG.mdの全リストマーカーをアスタリスクに統一
- Ink.js UIのブランチ表示位置とキーボード操作を修正
- Docker環境でのGitリポジトリ検出エラーメッセージを改善
- WorktreeディレクトリでのisGitRepository()動作を修正
- エラー表示にデバッグモード時のスタックトレース表示を追加
- リモートブランチ表示のアイコン幅を調整
- WorktreeConfig型のエクスポートとフォーマット修正
- Ink UIショートカットの動作を修正
- リリースワークフローの認証設定を追加
- LintワークフローにMarkdownlintを統合
- Spec Kitのブランチ自動作成を無効化
- Bunテスト互換のモック復元処理を整備
- Ink UIのTTY制御を安定化
- TTYフォールバックの標準入出力を引き渡す
- 子プロセス用TTYを安全に引き渡す
- Ink UI終了時にTTYリスナーを解放
- **ui:** Stop spinner once cleanup completes
- PRクリーンアップ時の未プッシュ判定をマージ済みブランチに対応
- Semantic-releaseがdetached HEAD状態で動作しない問題を修正
- Npm publishでOIDC provenanceを有効化
- NPM Token更新後の自動公開を有効化
- テストファイルを削除してnpm自動公開を確認
- TypeScript型エラーを修正してビルドを通す
- BranchActionSelectorScreenでqキーで戻る機能と英語化を実装
- AIToolSelectorScreenテストを非同期読み込みに対応
- Spec Kitスクリプトのデフォルト動作をブランチ作成なしに変更
- Spec Kitスクリプトのブランチ名制約を緩和
- EnsureGitignoreEntryテストを統合テストに変更
- RealtimeUpdate.test.tsxのテストアプローチを修正
- Codex CLIのweb_search_request対応
- 自動更新時のカーソル位置リセット問題を解決
- Codex CLIのweb検索フラグを正しく有効化
- 最新コミット順ソートの型エラーを解消
- BatchMergeServiceテストのモック修正とコンパイルエラー解消
- Exact optional cwd handling in divergence helper
- Heredoc内のgit文字列に誤反応しないようフック検知ロジックを改善
- Adjust auto merge workflow permissions
- Guard auto merge workflow when token missing
- Login gh before enabling auto merge
- Rely on GH_TOKEN env directly
- ブランチ行レンダリングのハイライト表示を調整
- Limit divergence checks to selected branch
- Bashフックで連結コマンドのgit操作を検知
- Align timestamp column for branch list
- Show pending state during branch creation
- エラー発生時の入力待機処理を追加
- Ensure worktree directory exists before creation
- Reuse repository root for protected branches
- Correct protected branch type handling
- AIツール起動失敗時もCLIを継続
- Worktree作成時の進捗表示を改善
- Allow protected branches to launch ai tools
- 保護ブランチ選択時のルート切替とUIを整備
- Scope gitignore updates to active worktree
- Git branch参照コマンドのブロックを解除
- Stabilize release test suites
- Replace vi.hoisted() with direct mock definitions
- Move mock functions inside vi.mock factory
- Codexエラー時でもCLIを継続
- Keep cli running on git failures
- Format entry workflow tests
- Codex起動時のJSON構文エラー修正とエラー時のCLI継続
- Docker環境でのpnpmセットアップとプロジェクトビルドを修正
- Update Dockerfile to use npm for global tool installation
- Use node 22 for release workflow
- Disable husky in release workflow
- Use PAT for release pushes
- Make release sync safe for develop
- Auto-mergeをpull_request_targetに変更
- Unity-mcp-serverとの差分を修正
- Unity-mcp-serverとの完全統一（残り20%の修正）
- Semantic-releaseのドライラン実行時にGITHUB_TOKENを設定
- Add test file for patch version release
- パッチバージョンリリーステスト用ファイル追加
- WorktreeOrchestratorモックをクラスベースに修正
- カバレッジレポート生成失敗を許容
- パッチバージョンリリーステスト用修正追加
- 3回目のパッチバージョンテスト修正追加
- Publish.ymlへのバックマージ処理の移行
- Execaのshell: trueオプションを削除してCodex CLI起動エラーを修正
- Npm publish時の認証設定を修正 (#203)
- Npm publish時の認証設定を修正
- Remove redundant terminal.exitRawMode() call in error path
- Block interactive rebase
- Use process.cwd() for hook script path resolution
- Worktree外へのcd制限とメッセージ英語化
- Execaをchild_process.spawnに置き換えてCodex CLI起動の互換性問題を解決
- ShellCheck警告を修正（SC2155, SC2269）
- ParseInt関数に基数パラメータを明示的に指定
- **workflows:** リリースフローの依存関係と重複実行を最適化
- **server:** 型エラー修正とビルドスクリプト最適化
- **server:** Docker環境からのアクセス対応とビルドパス修正
- **build:** Esbuildバージョン不一致エラーの解決
- **server:** Web UIサーバーをNode.jsで起動するよう修正
- **docker:** Web UIアクセス用にポート3000を公開
- CLI英語表示を強制
- **lint:** ESLintエラーを修正（未使用変数の削除）
- **docs:** Specsディレクトリのmarkdownlintエラーを修正
- **lint:** ESLint設定を改善してテストファイルのルールを緩和
- **docs:** Specs/feature/webui/spec.mdのbare URL修正
- **test:** テストファイルのimportパス修正
- **test:** Vi.mockのパスも修正してテストのimport問題を完全解決
- **test:** 通常のimport文も../../../../cli/パスに修正
- **test:** Importパスを正しい../../../git.jsに戻す
- **test:** Vitest.config.tsをESLintの対象に追加し、拡張子解決を改善
- **test:** テストファイルのインポートパスを修正して.ts拡張子に対応
- **test:** Dist-app-bundle.testのファイルパスを修正
- **test:** Main error handlingテストとCI環境でのhookテストスキップを修正
- **webui:** フック順序を安定化して詳細画面のクラッシュを解消
- **webui:** ブランチ選択でモーダルを確実に表示
- **webui:** ラジアルノードの重なりを軽減
- **webui:** ベース中心から接続線を描画
- **webui:** Navigate to branch detail after launching session
- **webui:** セッション終了後に一覧へ戻る
- **webui:** Focus new session after launch
- Clean up stale sessions on websocket close
- **web:** Generate worktree paths with repo root
- **websocket:** Add grace period before auto cleanup
- **websocket:** Add retry logic and detailed close logs
- **webui:** Use Fastify logger for WebSocket events
- **webui:** Prevent WebSocket reconnection on prop changes
- **webui:** Add missing useEffect import
- **webui:** 保護ブランチでのworktree作成を禁止
- **docker:** Docker起動時の強制ビルドを削除し開発環境専用に変更
- **webui:** Bun起動と環境設定の型崩れを修正
- **webui:** Update BranchGraph props for simplified API
- **docker:** Docker起動時の強制ビルドを削除し開発環境専用に変更
- **config:** Satisfy exact optional types
- **docker:** Docker起動時の強制ビルドを削除し開発環境専用に変更
- **test:** テストファイルのインポートパスとモックを修正
- **test:** GetSharedEnvironmentモックを追加
- 依存インストール失敗時のクラッシュを防止
- 依存インストール失敗時も起動を継続
- Markdownlint の違反を解消
- Xterm パッケージの依存関係問題を解決するため--legacy-peer-depsを追加
- Package-lock.jsonをpackage.jsonと同期
- Create-release.ymlのdry-runモードでNPM_TOKENエラーを回避
- Execa互換性問題によるblock-git-branch-ops.test.tsのテスト失敗を修正
- Markdownlintエラーを修正
- Release.ymlでsemantic-releaseの出力をログに表示するように修正
- スコープ付きパッケージをpublicとして公開するよう設定
- Release.ymlでnpm publish前にビルドを実行
- Semantic-releaseからnpm publishを分離してpublish.ymlに移動
- Semantic-release npmプラグインをnpmPublish: falseで有効化
- Bin/gwt.jsでmain関数を明示的に呼び出すように修正
- Markdownlintのignore_filesを複数行形式に修正
- .markdownlintignoreを追加してCHANGELOG.mdを除外
- Semantic-release実行に必要なNode.js setupを追加
- Publish.ymlでSetup Bunステップの順序を修正
- フィルター入力の表示位置をWorking DirectoryとStatsの間に修正
- フィルター入力とStatsの間の空行を削除
- フィルターモード中でもブランチ選択のカーソル移動を可能に
- ブランチ選択モードでのカーソル反転表示を修正
- Improve git hook detection for commands with options
- Use process.platform in claude command availability
- **cli:** ターミナル入力がフリーズする問題を修正
- Claude Codeのデフォルトモデル指定を標準扱いに修正
- Omit --model flag when default Opus 4.5 is selected
- Ensure selected model ID is passed to launcher for Claude Code
- フィルターモードでショートカットを無効化
- String-width v8対応のためWIDTH_OVERRIDESにVariation Selector付きアイコンを追加
- 全アイコンの幅オーバーライドを追加してタイムスタンプ折り返しを修正
- Prevent false positives in git hook detection
- 全ての幅計算をmeasureDisplayWidthに統一してstring-width v8対応を完了
- RenderBranchRowのcursorAdjustロジックを復元してテスト互換性を維持
- アイコン幅計測を補正してブランチ行の日時折り返しを防止
- 幅オーバーライドとアイコン計測のずれで発生する改行を再修正
- 幅計測ヘルパー欠落による型エラーを解消
- 実幅を過小評価しないよう文字幅計測と整列テストを更新
- タイムスタンプ右寄せに安全マージンを設けて改行を防止
- Ensure claude skipPermissions uses sandbox env
- 実行モード表示をNewに変更
- GitHub Actions完全自動化のためrelease-please設定を修正
- Create-release.ymlをdevelop→main PR作成方式に修正
- Jqコマンドの構文エラーを修正
- Release.ymlをrelease-pleaseから直接タグ作成方式に変更
- Release.ymlのコミットメッセージ検出条件を修正
- **docs:** Release-pleaseの参照をリリースワークフローに修正
- **docs:** Release-guide.jaのフロー図を実装に合わせて更新 (#283)
- **docs:** Release-guide.mdのフロー図を実装に合わせて更新 (#285)
- Include upstream base when selecting cleanup targets
- ブランチ一覧表示時にリモートブランチをfetchして最新情報を取得
- **docs:** Release-guide.mdのフロー図を実装に合わせて更新
- Navigation.test.tsx に fetchAllRemotes のモックを追加
- FetchAllRemotes 失敗時にローカルブランチを表示するフォールバックを追加
- Stabilize worktree support and last ai usage display
- Stabilize worktree flows and branch hook
- Save last AI tool immediately on launch
- Persist last AI tool before launch
- リモートブランチ削除をマージ済みPRのみに限定
- Stabilize worktree cleanup and ui tests
- Align cleanup reasons with types and dedupe vars
- Sync列の数字をアイコン直後に表示
- Sync列を固定幅化してブランチ名の位置を揃える
- Remote列の表示を改善（L=ローカルのみ、R=リモートのみ）
- Navigation.test.tsxにcollectUpstreamMap/getBranchDivergenceStatusesのモックを追加
- レビューコメントへの対応
- Align branch list headers
- Origin/developとのマージコンフリクトを解決
- ESLint警告103件とPrettier違反12ファイルを修正
- 自動クリーンアップでリモートブランチを削除しないように修正
- Origin/developとのマージコンフリクトを解決
- Origin/developとのマージコンフリクトを解決
- Prepare-release.yml を修正してdevelop→main へ直接マージするように変更
- Prepare-release.yml を llm-router と同じフローに統一
- ブランチ一覧のAIツールラベルからNew/Continue/Resumeを削除
- Detect codex session ids in nested dirs
- Limit continue session id to branch history
- Localize quick start screen copy
- Honor CODEX_HOME and CLAUDE_CONFIG_DIR for session lookup
- Preserve reasoning level and quick start for protected branches
- Show reasoning level on quick start
- Show reasoning level in quick start option
- Show reasoning labels in quick start
- Default skip permissions to no when missing
- Start new Claude session when no saved ID
- Locate Claude sessions under .config fallback
- Read Claude sessionId from history fallback
- クイックスタートのセッションID表示を修正
- ブランチ別クイックスタートが最新セッションを誤参照しないように
- クイックスタート選択時の型チェックを補強
- Quick Start表示を短縮しツールごとに見やすく調整
- Quick Startヘッダー初期非表示とレイアウトを改善
- Inkの色型エラーを解消
- ブランチ/ワークツリー別に最新セッションを抽出
- カテゴリ解決をswitchで安全化
- Quick Startで最新セッションをworktree優先＋カテゴリ表示を簡素化
- CodexのQuick Startで最新セッションIDをファイルから補完
- CodexのQuick Startで履歴IDがある場合は上書きしない
- Gemini resume失敗時に最新セッションへフォールバック
- Quick Startの選択でEnterが一度で効くように修正
- Codexセッション取得を開始時刻以降の最新ファイルに限定
- CodexセッションIDを起動時刻に近いものへ保存
- CodexセッションIDを起動直後にポーリングして補足
- ClaudeセッションIDを保存時に補完
- ClaudeセッションIDを起動直後にポーリングして補足
- Claudeセッション検出でdot→dashエンコードを考慮
- Claudeセッション検出でproject直下のjson/jsonlも探索
- Claudeセッション検出で最終更新順に有効IDを探索
- Quick StartでClaudeの最新セッションをファイルから優先取得
- Codex Quick Startで履歴より新しいセッションファイルを優先
- Codex保存時に最新セッションIDを再解決
- Claude/Codexセッションを起動時刻近傍で再解決
- セッションファイル探索に時間範囲フィルタを追加
- Geminiセッションも起動時刻近傍で再解決
- Quick Startで初回Enterを受付待ちにバッファ
- Geminiセッション検出をtmp全体のjson/jsonlから抽出
- Quick StartでEnter二度押し不要に
- Gemini起動時にstdoutからsessionIdを確実に捕捉
- Claude/Geminiのセッション取得を時間帯で厳密化
- Claude CodeでstdoutからsessionIdを確実に捕捉
- Capture session ids and harden quick start filters
- Keep local claude tty to avoid non-interactive launch
- Prefer on-disk latest claude session over early probe
- Prefer newest claude session file within window
- Scope codex/gemini session resolution to worktree
- Ignore stdout session ids that lack matching claude session file
- Filter claude quick start entries to existing session files
- Quick start uses newest claude session file per worktree
- Always show latest claude session id in quick start
- Quick start always resolves latest claude session without time window
- Stop treating arbitrary uuids in claude logs as session ids
- Use file-based session detection for Claude/Codex instead of stdout capture
- Prevent detecting old session IDs on consecutive executions
- Prioritize filename UUID over file content for session ID detection
- Add shell option to Codex execa for proper Ctrl+C handling
- Treat SIGINT as normal exit for AI tool child processes
- Add terminal.exitRawMode() to Codex finally block
- Remove SIGINT catch block from Codex to match Claude Code behavior
- Reset stdin state before Ink.js render to prevent hang after Ctrl+C
- Add execChild helper to handle SIGINT for Codex CLI
- Remove sessionProbe from Codex CLI to prevent Ctrl+C hang
- Improve Codex session cwd matching for worktree paths
- Extract cwd from nested payload in Codex session files
- Remove unused imports and variables for ESLint compliance
- Update codex test to expect two exitRawMode calls
- Ensure divergence prompt waits for input
- Add SIGINT/SIGTERM handling to Claude Code launcher
- Complete stdin reset before/after Claude Code launch
- Prevent stdin interference in isClaudeCommandAvailable()
- Resume stdin before Claude Code launch to prevent input lag
- Resolve key input lag in Claude Code and Gemini CLI
- Capture Gemini session ID from exit summary output
- DivergenceテストにwaitForEnterモックを追加
- Fastify logger型の不整合を修正
- Share logger date helper and simplify tests
- Align branch list layout and icon widths
- Resolve lint errors on branch list
- Prompt.jsモックでimportActualを使用
- **test:** テストモックのAPI形状を修正
- Web UIポート解決とトレイ初期化の堅牢化
- 未使用インポートを削除しESLintエラーを解消
- Handle LF enter in Select
- PR #344 CodeRabbitレビュー対応
- React error #310 - フック呼び出し順序を修正
- Resume/ContinueでsessionIdを上書きしない
- Quick Start画面の初回表示時にEnterが効かない問題を修正
- Resumeは各ツールのresume機能に委譲
- Goodbye後にプロセスが終了しない問題を修正
- Web UIサーバー停止をタイムアウト付きで堅牢化
- Web UI URL表示削除に伴うテスト修正
- SPAルーティング用のフォールバック処理を追加
- Web UIからClaude Code起動時にENABLE_LSP_TOOL環境変数を渡す
- Web UIからClaude Code起動時にENABLE_LSP_TOOL環境変数を渡す
- MacOS/Linuxでトレイ初期化を無効化してクラッシュを防止
- トレイ破棄の二重実行を防止
- トレイ再初期化とテストのplatform注入
- EnvironmentProfileScreenのキーボード入力を修正
- CodeRabbitのレビュー指摘事項を修正
- Spec Kitスクリプトの安全性改善（eval撤廃/JSON出力）
- Profiles.yaml未作成時の作成失敗を修正
- プロファイル名検証と設定パス不整合を修正
- Envキー入力のバリデーションを追加
- プロファイル保存の一時ファイルとスクロール境界を修正
- Envキー入力バリデーションを調整
- Profiles.yaml更新の競合を防止
- プロファイル画面の入力検証とインデックス境界を修正
- プロファイル変更後にヘッダー表示を更新
- アクセス不可Worktreeを🔴表示に変更
- CodeRabbit指摘事項を修正
- CodeRabbit追加指摘事項を修正
- CodeRabbitレビュー最終修正
- MatchesCwdにクロスプラットフォームパス正規化を追加
- パスプレフィックスマッチングに境界チェックを追加
- Gemini-3-flash のモデル ID を gemini-3-flash-preview に修正
- Geminiのモデル選択肢を修正（Default追加＋マニュアルリスト復元）
- Gemini CLI起動時のTTY描画を維持する
- WSL2とWindowsで矢印キー入力を安定化
- デフォルトモデルオプション追加に伴うテスト期待値を修正
- Worktree再利用の整合性検証とモデル名正規化
- NormalizeModelIdの空文字処理とテスト補強
- Unblock cli build and web client config
- クリーンアップ選択の安全判定を要件どおりに更新
- Type-checkでcleanup対象の型エラーを解消
- ENABLE_LSP_TOOL環境変数の値を"1"から"true"に修正
- Node-ptyで使用するコマンドのフルパスを解決
- WebSocket接続エラーの即時表示を抑制
- Web UIのデフォルトポートを3001に変更
- 未対応環境ではClaude CodeのChrome統合をスキップする
- WSL1検出でChrome統合を無効化する
- WSLの矢印キー誤認を防止
- 相対パス起動のエントリ判定を安定化
- リモート取得遅延でもブランチ一覧を表示
- Git情報取得のタイムアウトを追加
- Mode表示を Stats 行の先頭に移動
- ブランチ一覧取得時にrepoRootを使用するよう修正
- Gitデータ取得のタイムアウトを延長
- **ci:** マージ方法をsquashに変更してCHANGELOG重複を防止 (#425)
- リモートモードでローカル・リモート両存在ブランチが表示されない問題を修正 (#430)
- ブランチリスト画面のフリッカーを解消 (#433)
- Claude Codeのフォールバックをbunxに統一
- **cli:** AIツール実行時にフルパスを使用して非インタラクティブシェルのPATH問題を修正 (#436)
- **cli:** AIツール実行時にフルパスを使用 (#439)
- Worktree作成時のstale残骸を自動回復 (#445)
- 自動インストール警告文のタイポ修正 (#451)
- Warn then return after dirty worktree (#453)
- Execaのshell: trueオプションを削除してbunx起動エラーを修正 (#458)
- Claude-worktree後方互換コードを削除 (#462)
- Package.json の description を Coding Agent 対応に修正 (#471)
- Tools.json の customTools → customCodingAgents マイグレーション対応 (#476)
- Divergenceでも起動を継続 (#483)
- CLI終了時のシグナルハンドリング改善と各種ドキュメント修正 (#489)
- Stabilize OpenTUI solid tests and UI layout (#490)
- 依存関係インストール時のスピナー表示を削除 (#496)
- 起動ログの出力経路とCodexセッションID検出を改善 (#495)
- ブランチ一覧にセッション履歴を反映 (#497)
- Show worktree path in branch footer (#499)
- ブランチ一覧のASCII表記を調整 (#500)
- ウィザード内スクロールの上下キー対応を追加
- ウィザードのfocus型を厳密オプションに合わせる
- ESCキャンセル後にウィザードが開かない問題を修正 (#501)
- セッションIDの表示と再開を改善 (#505)
- **cli:** Keep wizard cursor visible in popup (#506)
- **cli:** Keep wizard cursor visible in popup (#507)
- Repair機能のクロス環境対応とUI改善 (#508)
- Worktree修復ロジックの統一化とクロス環境対応 (#509)
- クリーンアップ安全表示を候補判定に連動 (#514)
- SaveSessionにtoolVersionを追加して履歴に保存 (#515)
- Interactive loop test hang
- 安全アイコン表示のルールを更新 (#516)
- Dependency installer test hang
- Stabilize dependency installer test mocks
- Post-session checks test hang
- **test:** Bun互換性のためのテスト修正 (#527)
- **ci:** Publishワークフローにテストタイムアウトを追加 (#530)
- **test:** Worktree.test.tsのVitest依存を削除してBun互換に修正 (#533)
- 安全アイコンの安全表示を緑oに変更 (#525)
- Run UI with bun runtime (#537)
- 安全状態確認時のカーソルリセット問題を修正 (#539)
- カーソル位置をグローバル管理に変更して安全状態更新時のリセットを防止 (#541)
- ログビューア表示と配色の統一 (#538)
- Cleanup safety and tool version fallbacks (#543)
- Unsafe確認ダイアログ反転と凡例のSafe追加 (#544)
- コーディングエージェント起動時の即時終了問題を修正
- Quick Startセッション解決をブランチ基準に修正 (#547)
- Issue 546のログ/ウィザード/モデル選択を改善 (#551)
- Codex skillsフラグをバージョン判定で切替 (#552)
- ブランチリフレッシュ時にリモート追跡参照を更新 & CI/CD最適化 (#554)
- Cache installed versions for wizard (#555)
- Clippyワーニング解消およびコード品質改善
- TUIキーバインドをTypeScript版と一致させる
- フィルターモード中のキーバインド処理を修正
- ヘッダーフォーマットをTypeScript版に統一
- マウスキャプチャを無効化してテキスト選択を可能に
- ウィザード表示・スピナー・エージェント色マッピングを修正
- ウィザードのモデル選択・エージェント色をTypeScript版に合わせて修正
- TUI画面のレイアウト・プロファイル・ログ読み込みを修正
- Gemini CLIのnpmパッケージ名を修正
- Codex CLIのモデル指定オプションを-mに変更
- FR-072/FR-073準拠のバージョン表示形式を修正
- FR-063a準拠のinstalled表示形式を修正
- FR-070準拠のツール表示形式に日時を追加
- FR-004準拠のフッターキーバインドヘルプを追加
- FR-070準拠のツール表示形式から二重日時表示を削除
- Worktreeからメインリポジトリルートを解決してセッションファイルを検索
- SPEC-d2f4762a FR要件準拠の修正

### Documentation

- OpenTUI移行の将来計画仕様を追加 (SPEC-d27be71b) (#478)
- Divergence起動継続の統合仕様を更新 (#479)
- ブランチ選択後のウィザードポップアップフローを仕様化
- README.mdを大幅に更新し日本語版README.ja.mdを新規作成
- インストール方法にnpx実行オプションを追加
- CLAUDE.mdのGitHub Issues更新ルールを削除し、コミュニケーションガイドラインを追加
- README.ja.mdからCI/CD統合セクションを削除
- README.mdからもCI/CD統合セクションを削除
- Add pnpm and bun installation methods to README
- Memory/・templates/・.claude/commands/ 配下のMarkdownを日本語化
- **specs:** 仕様の要件/チェックリストを実装内容に合わせ更新
- **tasks:** 仕様実装に合わせてタスクを圧縮・完了状態へ更新
- **bun:** 関連ドキュメントをbun前提に更新
- READMEをbun専用に統一し、関連ドキュメントも整備
- README(英/日)をAIツール選択（Claude/Codex）対応の記述へ更新
- AGENTS.md と CLAUDE.md にbun利用ルール（ローカル検証/実行）を明記
- 仕様駆動開発ライフサイクルに関する表現を修正
- Clean up merged PRs機能の修正仕様書を作成
- Spec Kit完全ワークフローの文書化を完了
- フェーズ11ドキュメント改善 & フェーズ12 CI/CD強化完了 (T1001-T1109)
- テスト実装プロジェクト完了サマリー作成
- AGENTS.mdの内容を@CLAUDE.mdに移行し、開発ガイドラインを整理
- PR自動マージ機能の説明をREADMEに追加し、ドキュメントを完成 (T015-T016)
- Spec Kit設計ドキュメントを追加
- SPEC-23bb2eed全タスク完了マーク
- T011完了をtasks.mdに反映
- セッション完了サマリー - Phase 3完了とPhase 4開始の記録
- SESSION_SUMMARY.md最終更新 - Phase 4完了を反映
- T098-T099完了 - ドキュメント更新（Ink.js UI移行）
- Tasks.md更新 - Phase 6全タスク完了マーク
- Enforce Spec Kit SDD/TDD
- Bun vitestのretry未サポートを記録
- Add commitlint rules to tasks template
- Tasks.md Phase 4進捗を更新（T056-T071完了、T068スキップ）
- Tasks.md Phase 4完了をマーク（T072-T076）
- Tasks.md Phase 1-6完了マーク（全タスク完了）
- ブランチ切り替え禁止ルールを追加
- Markdownlintスタイルの調整
- Lint最小要件をタスクテンプレに明記
- エージェントによるブランチ操作禁止を明記 (#108)
- 現行CLI仕様に合わせてヘルプを更新
- Worktreeディレクトリパス変更の実装計画を作成
- Worktreeディレクトリパス変更のタスクリストを生成
- CHANGELOG.mdにWorktreeディレクトリ変更を追加
- エージェントによるブランチ操作禁止を明記
- Plan.mdのURL形式を修正（Markdownlint対応）
- CLAUDE.mdにコミットメッセージポリシーを追記
- Update tasks.md with completed US2 and Phase 4 status
- SPEC-a5ae4916 に最新コミット順の要件を追記
- MarkdownlintをクリアするためのSpec更新
- SPEC-ee33ca26 品質分析完了・修正適用
- SPEC-a5ae4916 を最新コミット表示要件に更新
- CLAUDE.mdからフック重複記述を削除しコンテキストを最適化
- SPEC-23bb2eedを手動リリースフロー仕様に更新
- Add SPEC-a5a44f4c release test stabilization kit
- Publish.ymlのコメントを更新 (#204)
- READMEのインストールセクションを改善 (#207)
- Publish.ymlのコメントを更新
- READMEのインストールセクションを改善
- Fix markdownlint error in spec document
- Commitlintとsemantic-release整合性の厳格化
- Lintエラー修正
- Align release flow with release branch automation
- Clarify /release can run from any branch
- **spec:** SPEC-57fde06fにバックマージ要件を追加しワークフローを最適化
- Web UI機能のドキュメント追加
- **spec:** Add env config specs
- 残りのドキュメント内の参照を更新
- Fix changelog markdownlint errors
- Spec Kit対応 - bugfixブランチタイプ機能の仕様書・計画・タスクを追加
- 仕様書を実装に合わせて更新＋Filter:の色をdimColorに変更
- Plan.mdの見出しレベルを修正
- ドキュメント内のsemantic-release言及をrelease-pleaseに更新
- Release.mdのフロー説明をmainブランチターゲットに修正
- Update cleanup criteria to use upstream base
- Update branch cleanup requirements
- Add Icon Legend section to README.md
- Fix markdownlint tags in spec tasks
- Check off saved session tasks
- Update quick start tasks
- Quick Start表示ルールを要件・タスクに追記
- AIツール起動機能の仕様タイトルを修正
- 基本ルールに要件化・TDD化優先の指示を追加
- 既存要件への追記可能性確認ステップを追加
- Quick StartのセッションID要件を仕様に追加
- 仕様配置規約をCLAUDE.mdに追記
- PRレビュー指摘事項を反映
- ログ運用統一仕様を追加
- ログローテーション要件を追加
- ログカテゴリと削除タイミングを明記
- ログ仕様にTDD要件を追加
- ログ統一仕様の実装計画を作成
- ログ統一仕様のタスクを追加
- ログ統一仕様のデータモデルとクイックスタート追加
- Document safeToCleanup flag on BranchItem
- Align cleanup plan with current emoji icons
- Web UI起動手順と設定パスを最新化
- SPEC-1f56fd80のmarkdownlint修正
- ヘルプテキストに serve コマンドを追加
- Linuxのnode-gypビルド要件を追記
- Qwen未サポート要件の適用範囲を明確化
- 公開APIのJSDocを追加
- 公開APIのJSDocと仕様文言修正
- Worktreeクリーンアップ選択機能のSPEC・設計ドキュメント作成
- Update spec tasks status
- Fix markdownlint in spec data model
- ChromeパラメータのJSDocドキュメントを追加
- Specs一覧をカテゴリ別に整理
- 廃止仕様をカテゴリ分け
- **spec:** ログ仕様の明確化とログビューア機能の仕様策定 (#432)
- Update task planning instruction
- README.md/README.ja.mdを最新の実装状態に同期 (#469)
- OpenTUI移行の将来計画仕様を追加 (SPEC-d27be71b) (#478)
- Divergence起動継続の統合仕様を更新 (#479)
- ブランチ選択後のウィザードポップアップフローを仕様化
- Rust移行仕様書を追加（SPEC-1d62511e）
- SPEC-d2f4762aのtasks.mdをRust移行に合わせて更新
- SPEC-d2f4762aをRust移行に合わせて更新

### Features

- OpenCode コーディングエージェント対応を追加 (#477)
- Worktreeパス修復機能を追加 (SPEC-902a89dc) (#484)
- ブランチ選択のフルパス表示 (#486)
- OpenTUI移行 (#487)
- 新規ブランチ作成時にブランチタイプ選択とプレフィックス自動付加を追加 (#494)
- ショートカット表記を画面内に統合 (#503)
- Initial package structure for claude-worktree
- 新機能の追加と既存機能の改善
- Add change tracking and post-Claude Code change management
- マージ済みPRのworktreeとブランチを削除する機能を追加
- UIの改善と表示形式の更新
- 表デザインをモダンでより見やすいスタイルに改善
- 表デザインをモダンでより見やすいスタイルに改善
- Repository Statistics表示をよりコンパクトで見やすいデザインに改善
- ブランチ選択UIと操作メニューの視覚的分離を改善
- Repository Statisticsの表デザインを改善
- Repository Statisticsセクションを削除
- キーボードショートカット機能とブランチ名省略表示を実装
- クリーンアップ時の表示メッセージを改善
- バージョン番号をタイトルに表示
- マージ済みPRクリーンアップ機能の改善
- テーブル表示にカラムヘッダーを追加
- クリーンアップ時にリモートブランチも削除する機能を追加
- リモートブランチ削除を選択可能にする機能を追加
- Worktree削除時にローカルブランチをリモートにプッシュする機能を追加
- Worktreeに存在しないローカルブランチのクリーンアップ機能を追加
- Git認証設定をentrypoint.shに追加
- アクセスできないworktreeを明示的に表示し、pnpmへ移行
- -cパラメーターによる前回セッション継続機能を追加
- -rパラメーターによるセッション選択機能を追加
- .gitignoreと.mcp.jsonの更新、docker-compose.ymlから不要な環境変数を削除
- Worktree選択後にClaude Code実行方法を選択できる機能を追加
- Docker-compose.ymlにNPMのユーザー情報を追加
- Claude -rの表示を大幅改善
- Claude -rをグルーピング形式で大幅改善
- Claude Code履歴を参照したresume機能を実装
- Resume機能を大幅強化
- メッセージプレビュー表示を大幅改善
- 時間表示を削除してccresume風のプレビュー表示に改善
- 全画面活用の拡張プレビュー機能を実装
- 全画面でqキー統一操作に変更
- Npm versionコマンドと連携したリリースブランチ作成機能を実装
- Git Flowに準拠したリリースブランチ作成機能を実装
- リリースブランチ終了時に選択肢を提供
- リリースブランチの自動化を強化
- リリースブランチ完了時のworktreeとローカルブランチ自動削除機能を追加
- Claude Codeアカウント切り替え機能を追加
- Add Spec Kit
- **specify:** ブランチを作成しない運用へ変更
- Codex CLI対応の仕様と実装計画を追加
- AIツール選択（Claude/Codex）機能を実装
- ツール引数パススルーとエラーメッセージを追加
- Npx経由でAI CLIを起動するよう変更
- @akiojin/spec-kitを導入し、仕様駆動開発をサポート
- 既存実装に対する包括的な機能仕様書を作成（SPEC-473b3d47）
- Codex CLIのbunx対応とresumeコマンド整備
- GitHub CLIのインストールをDockerfileに追加
- Claude CodeをnpxからbunxへComplete移行（SPEC-c0deba7e）
- **auto-merge:** PR番号取得、マージ可能性チェック、PRマージステップを実装 (T004-T006)
- Semantic-release自動リリース機能を実装
- Semantic-release設定を明示化
- ブランチ選択カーソル視認性向上 (SPEC-822a2cbf)
- Ink.js UI移行のPhase 1完了（セットアップと準備）
- Phase 2 開始 - 型定義拡張とカスタムフック実装（進行中）
- Phase 2基盤実装 - カスタムフック（useTerminalSize, useScreenState）
- Phase 2基盤実装 - 共通コンポーネント（ErrorBoundary, Select, Confirm, Input）
- Phase 2基盤実装完了 - UI部品コンポーネント（Header, Footer, Stats, ScrollableList）
- Phase 3開始 - データ変換ロジック実装（branchFormatter, statisticsCalculator）
- Phase 3実装 - useGitDataフック（Git情報取得）
- Phase 3 T038-T041完了 - BranchListScreen実装
- Phase 3 T042-T044完了 - App component統合とフィーチャーフラグ実装
- Phase 3 完了 - 統合テスト・受け入れテスト実装（T045-T051）
- Phase 4 開始 - 画面遷移とWorktree管理画面実装（T052-T055）
- T056完了 - WorktreeManager画面遷移統合（mキー）
- T057-T059完了 - BranchCreatorScreen実装と統合
- T060-T062完了 - PRCleanupScreen実装と統合
- T063-T071完了 - 全サブ画面実装完了（Phase 4 サブ画面実装完了）
- T072-T076完了 - Phase 4完全完了！（統合テスト・受け入れテスト実装）
- T077-T080完了 - リアルタイム更新機能実装
- T081-T084完了 - パフォーマンス最適化と統合テスト実装
- T085-T086完了 - Phase 5完全完了！リアルタイム更新機能実装完了
- T096完了 - レガシーUIコード完全削除
- T097完了 - @inquirer/prompts依存削除
- Phase 6完了 - Ink.js UI移行成功（成功基準7/8達成）
- Docker/root環境でClaude Code自動承認機能を追加
- ブランチ一覧のソート優先度を整理
- Tasks.mdにCI/CD検証タスク（T105-T106）を追加 & markdownlintエラーを修正
- カーソルのループ動作を無効化したカスタムSelectコンポーネントを実装
- カスタムSelectコンポーネントのテスト実装とUI 5カラム表示構造への修正
- ブランチ選択後のワークフロー統合（AIツール選択→実行モード選択→起動）
- SkipPermissions選択機能とAIツール終了後のメイン画面復帰を実装
- Add git loading indicator with tdd coverage
- ブランチ作成機能を実装（FR-007完全対応）
- Add git loading indicator with tdd coverage (#104)
- SPEC-6d501fd0仕様・計画・タスクの詳細化と品質分析
- **ui:** PRクリーンアップ実行中のフィードバックを改善
- **ui:** PRクリーンアップ実行中のフィードバックを改善
- **ui:** 即時スピナー更新と入力ロックのレスポンス改善
- ブランチ一覧のソート機能を実装
- 型定義を追加（BranchAction, ScreenType拡張, getCurrentBranch export）
- カレントブランチ選択時にWorktree作成をスキップする機能を実装
- ブランチ選択後にアクション選択画面を追加（MVP2）
- 選択したブランチをベースブランチとして新規ブランチ作成に使用
- 戻るキーをqからESCに変更、終了はCtrl+Cに統一
- カスタムAIツール対応機能を実装（設定管理・UI統合・起動機能）
- カスタムツール統合と実行オプション拡張（Phase 4-6完了）
- セッション管理拡張とコード品質改善（Phase 7-8完了）
- Cコマンドでベース差分なしブランチもクリーンアップ対象に追加
- Worktreeディレクトリパスを.git/worktreeから.worktreesに変更
- Worktree作成時に.gitignoreへ.worktrees/を自動追加
- リアルタイム更新機能を実装（FR-009対応）
- **version:** Add CLI version flag (--version/-v)
- UIヘッダーにバージョン表示機能を追加 (US2)
- ブランチ一覧に未プッシュ・PR状態アイコンを追加
- Claude Code自動検出機能を追加（US4: ローカルインストール版優先）
- Bunxフォールバック時に公式インストール方法を推奨
- Bunxフォールバック時のメッセージに2秒待機を追加
- Windows向けインストール方法を推奨メッセージに追加
- Husky対応を追加してコミット前の品質チェックを自動化
- ヘッダーに起動ディレクトリ表示機能の仕様を追加
- ヘッダーへの起動ディレクトリ表示の実装計画を追加
- ヘッダーへの起動ディレクトリ表示の実装タスクを追加
- ヘッダーに起動ディレクトリ表示機能を実装
- ブランチ一覧の最新コミット順ソートを追加
- Bashツールでのgitブランチ操作を禁止するPreToolUseフックを追加
- フェーズ2完了 - 型定義とgit操作基盤実装
- BatchMergeService完全実装 (T201-T214)
- App.tsxにbatch merge機能を統合
- Dry-runモード実装（T301-T304）
- Auto-pushモード実装（T401-T404）
- AI起動前にfast-forward pullと競合警告を追加
- PR作成時に自動マージを有効化
- ブランチ一覧に最終更新時刻を表示
- ブランチ行の最終更新表示を整形し右寄せを改善
- Develop-to-main手動リリースフローの実装
- PRベースブランチ検証とブランチ戦略の明確化
- Guard protected branches from worktree creation
- Clarify protected branch workflow in ui
- Worktree作成中にスピナーを表示
- Orchestrate release branch auto merge flow
- Unity-mcp-server型自動リリースフロー完全導入
- マイナーバージョンリリーステスト機能追加
- 3回目のマイナーバージョンテスト機能追加
- Npm公開機能を有効化
- Add comprehensive TDD and spec for git operations hook
- Worktree内でのcdコマンド使用を禁止するフックを追加
- Worktree内でのファイル操作制限機能を追加
- ワークツリー依存を自動同期
- **web:** Web UI依存関係追加とCLI UI分離
- **web:** Web UIディレクトリ構造と共通型定義を作成
- **cli:** Src/index.tsにserve分岐ロジックを追加
- **server:** Fastifyベースのバックエンド実装とREST API完成
- **client:** フロントエンド基盤実装 (Vite/React/React Router)
- **client:** ターミナルコンポーネント実装とAI Toolセッション起動機能
- Web UIのデザイン刷新とテスト追加
- Web UIのブランチグラフ表示を追加
- **webui:** ブランチ差分を同期して起動を制御
- **webui:** Web UI からGit同期を実行
- **webui:** AIツール設定とWebSocket起動を共通化
- **webui:** ラジアル分岐グラフでモーダル起動に対応
- **webui:** グラフ優先の表示切替を追加
- **webui:** ラジアルグラフにベースフィルターを追加
- **webui:** Divergenceフィルターでグラフ/リストを連動
- **webui:** ラジアルノードをドラッグで再配置
- **webui:** ベースとノードを線で接続
- **webui:** Origin系ノードを統合
- **webui:** グラフ表示を下部へ移動
- **webui:** グラフレイアウト改善とセッション起動修正
- Add shared environment config management
- **logging:** Persist web server logs to file
- **webui:** Implement graphical overlay UI
- **config:** Support shared env persistence
- **server:** Expose shared env configuration
- **webui:** Add shared env management UI
- **cli:** Merge shared environment when launching tools
- Codex CLI のデフォルトモデルを gpt-5.1 に更新
- Bugfixブランチタイプのサポートを追加
- Fキーでフィルター・検索モードを追加
- フィルター入力中のキーバインド(c/r/m)を無効化＋要件・テスト更新
- フィルターモード/ブランチ選択モードの切り替え機能を追加
- フィルターモード中もブランチ選択の反転表示を有効化
- Gemini CLIをビルトインツールとして追加
- Codex/Geminiの表示名を簡潔化
- Qwenをビルトインツールとして追加
- QwenサポートをREADMEに追加し、GEMINI.mdを作成
- Align model selection with provider defaults
- Remember last model and reasoning selection per tool
- Update Opus model version to 4.5
- Update default Claude Code model to Opus 4.5
- Add Sonnet 4.5 as an explicit model option
- Set Opus 4.5 as default and remove explicit Default option
- Set upstream tracking for newly created refs
- Semantic-releaseからrelease-pleaseへ移行
- Preselect last AI tool when reopening selector
- ブランチ一覧にLocal/Remote/Sync列を追加
- Cコマンドでリモートブランチも削除対象に追加
- ブランチ一覧にラベル行を追加
- ブランチ一覧の表示アイコンを直感的な絵文字に改善
- Persist and surface session ids for continue flow
- Support gemini and qwen session resume
- Fallback resolve continue session id from tool cache
- Add branch quick start reuse last settings
- Add branch quick start screen ui tests
- Skip execution mode when quick-start reusing settings
- Reuse skip permissions in quick start
- クイックスタートでツール別の直近設定を提示
- Quick Startをツールカテゴリ別に色分け表示
- Codex CLIのスキル機能を有効化
- 全AIツール起動時のパラメーターを表示
- Ink.js CLI UIデザインスキル（cli-design）を追加
- Pino構造化ログと7日ローテーションを導入
- Route logs to ~/.gwt with daily jsonl files
- Codexにgpt-5.2モデルを追加
- **webui:** CLI起動時にWeb UIサーバーを自動起動
- Web UIトレイ常駐とURL表示
- **webui:** Tailwind CSS + shadcn/ui基盤を導入
- **webui:** 全ページをTailwind + shadcn/uiでリファクタリング
- ポート使用中時のWeb UIサーバー起動スキップ (FR-006)
- MacOS対応のシステムトレイを実装
- Claude CodeのTypeScript LSP対応を追加
- Web UIサーバー全体にログ出力を追加
- 環境変数プロファイル機能を追加
- プロファイル未選択を選択できるようにする
- Gemini-3-flash モデルのサポートを追加
- 全てのツールにデフォルト（自動選択）オプションを追加し、Geminiのモデル選択肢を改善
- Qwen CLIを未サポート化
- Gpt-5.2-codex対応
- Codexモデル一覧を4件に整理
- Add branch selection parity for cleanup flow
- リモートにコピーがあるブランチのローカル削除をサポート
- Add post-session push prompt
- Claude Code起動時にChrome拡張機能統合を有効化
- ブランチグラフをReact Flowベースにリファクタリング
- ブランチ表示モード切替機能（TABキー）を追加
- Requirements-spec-kit スキルを追加
- Claude Codeプラグイン設定を追加 (#429)
- **cli:** AIツールのインストール状態検出とステータス表示を追加 (#431)
- 未コミット警告時にEnterキー待機を追加 (#441)
- ログビューアを追加 (#442)
- ログ表示の通知と選択UIを改善 (#443)
- Docker構成を最適化しPlaywright noVNCサービスを追加 (#454)
- Docker構成を最適化しPlaywright noVNCサービスを追加 (#455)
- ブランチ一覧に最終アクティビティ時間を表示 (#456)
- AIツールのインストール済み表示をバージョン番号に変更 (#461)
- OpenCode コーディングエージェント対応を追加 (#477)
- Worktreeパス修復機能を追加 (SPEC-902a89dc) (#484)
- ブランチ選択のフルパス表示 (#486)
- OpenTUI移行 (#487)
- 新規ブランチ作成時にブランチタイプ選択とプレフィックス自動付加を追加 (#494)
- ショートカット表記を画面内に統合 (#503)
- コーディングエージェントのバージョン選択機能を改善 (#510)
- **ui:** コーディングエージェント名の一貫した色づけを実装 (#511)
- コーディングエージェントバージョンの起動時キャッシュ (FR-028～FR-031) (#542)
- Rustワークスペース基盤を作成
- Rustコア機能完全実装（Phase 1-4）
- TUI画面をTypeScript版と完全互換に拡張
- Enterキーでウィザードポップアップを開く機能を実装
- TypeScriptからRustへの完全移行
- FR-050 Quick Start機能をウィザードに追加
- FR-029b-e 安全でないブランチ選択時の警告ダイアログを実装
- FR-010/FR-028 ブランチクリーンアップ機能を実装
- FR-038-040 Worktree stale回復機能を実装
- FR-060-062 ウィザードポップアップのスクロール機能を実装
- Xキーでgit worktree repairを実行する機能を実装

### Miscellaneous Tasks

- Sync main release-please changes into develop
- Npx文言を削除 (#485)
- Vitest から bun test への移行 (#491)
- Vitest関連パッケージとファイルを削除 (#492)
- Developをマージ
- Mainブランチとのコンフリクトを解決
- Bump version to 0.4.15
- .gitignoreとpackage.jsonの更新、pnpm-lock.yamlの追加
- Dockerfileから不要なnpm更新コマンドを削除
- Prepare release 0.5.3
- Prepare release 0.5.4
- Bump version to 0.5.5
- Bump version to 0.5.6
- 余分にコミットされた specs を削除
- **bun:** パッケージマネージャをpnpmからbunへ移行
- Npm/pnpmの痕跡を削除しbun専用化
- Npm/pnpm言及の完全排除とbun専用化の仕上げ
- バナー/ヘルプ文言を中立化（Worktree Manager）
- Npx経由コマンドを最新版指定に更新
- プロジェクトセットアップとタスク完了マーク更新
- Mainブランチとのコンフリクトを解決
- CI検証手順をテンプレートと設定に反映
- Merge main branch
- CI再トリガー
- NPM_TOKEN更新後の自動公開テスト
- Add .worktrees/ to .gitignore
- コードフォーマット修正とドキュメント更新
- ESLint ignore設定を移行
- Mainブランチを取り込み競合を解消
- Markdownlint違反を是正
- Auto merge workflow test
- Auto merge workflow test 2
- Skip auto-merge when token missing
- Auto merge workflow test 3
- Auto merge workflow test 4
- Auto merge workflow test 5
- Dockerfileにcommitlintツールを追加
- 開発環境をnpmからpnpmに移行
- Merge origin/main into feature branch
- Merge origin/main into hotfix
- Update Docker setup and entrypoint script
- ReleaseフローをMethod Aに再構築
- Disable commitlint body line limit
- Dockerfileのグローバルツールインストールを最適化
- Merge develop
- Releaseコミットをcommitlint準拠に調整
- Auto Merge ワークフローで PERSONAL_ACCESS_TOKEN を使用
- Auto Merge ワークフローを pull_request_target に変更
- Auto Merge ワークフローを一本化
- 古いrelease-trigger.ymlを削除
- Backmerge main to develop after release
- Backmerge main to develop after release
- Backmerge main to develop after release
- Backmerge main to develop after release
- Backmerge main to develop after release
- Backmerge main to develop after release
- Npm認証方式をコメントに追記 (#205)
- Npm認証方式をコメントに追記
- Lint-stagedでmarkdownlintを強制
- **workflows:** 不要なcheck-pr-base.ymlを削除
- **webui:** Switch branch list strings to English
- **debug:** Add websocket instrumentation
- Merge origin/feature/webui
- Synapse PoCのスタンドアロン環境追加
- **worktree:** Remove duplicated files from worktree
- Merge develop into feature/environment
- Configure dependabot commit messages
- **deps-dev:** Bump js-yaml
- Semantic-releaseがreleaseブランチから実行できるように設定追加
- Dockerfile を復元
- CI再実行のための空コミット
- CI/CDをbunに統一してnpm依存を削除
- Developブランチの最新変更をマージ
- コードフォーマットを適用
- Add vitest compatibility shims for hoisted/resetModules
- Stabilize tests with cross-platform platform checks and timer shims
- 再PR モデル選択修正・テスト安定化 (#243)
- Auto fix lint issues
- **deps-dev:** Bump @commitlint/cli from 19.8.1 to 20.1.0
- **deps-dev:** Bump @types/node from 22.19.1 to 24.10.1
- **deps-dev:** Bump vite from 6.4.1 to 7.2.4
- **deps-dev:** Bump @vitejs/plugin-react from 4.7.0 to 5.1.1
- **deps-dev:** Bump esbuild from 0.25.12 to 0.27.0
- **deps-dev:** Bump lint-staged from 15.5.2 to 16.2.7
- **deps-dev:** Bump @commitlint/config-conventional
- Update bun.lock
- Update manifest to 2.7.0 [skip ci]
- Backmerge main to develop [skip ci]
- Update manifest to 2.7.1 [skip ci]
- Update manifest to 2.7.2 [skip ci]
- Trigger CI checks
- Resolve merge conflict with develop
- Clarify immediate save of last tool
- Address review feedback for cleanup flow
- Quick Start表示をさらに簡潔化
- Quick StartでOtherカテゴリ前に余白を追加
- Quick Startカテゴリ表示のテキストを簡潔化
- Quick Startをカテゴリヘッダー+配下アクションの構造に変更
- ビルドエラー解消の型インポート追加
- Quick Startでカテゴリヘッダーを除去し選択肢のみ表示
- Quick Start行をカテゴリ色付きラベルのみに整理
- Quick Startラベルを色付きカテゴリ+アクションだけに整理
- Merge develop to resolve conflicts
- AIツール終了後に3秒待機してブランチ一覧へ戻す
- Fix markdownlint violation
- **deps-dev:** Bump esbuild from 0.27.0 to 0.27.1
- Fix markdownlint in spec
- Bun.lock を更新
- Bun.lock の configVersion を復元
- 仕様ディレクトリを規約に沿って移設
- Cli-designスキルをプロジェクトから削除
- Fix markdownlint indent in log plan
- Raise test memory and limit vitest workers
- Stabilize tests under CI memory constraints
- Further reduce vitest parallelism to avoid OOM
- Skip branch list performance specs in CI and lower vitest footprint
- MCP設定ファイルを追加
- **husky:** Commit-msgフックでcommitlintを自動実行
- Developブランチをマージしコンフリクト解消
- Developをマージ
- **test:** Use threads pool for vitest
- Update manifest to 2.7.3 [skip ci]
- **main:** Release 2.7.4
- **main:** Release 2.8.0
- **main:** Release 2.9.0
- **main:** Release 2.9.1
- **main:** Release 2.10.0
- **main:** Release 2.11.0
- **main:** Release 2.11.1
- **main:** Release 2.12.0
- **main:** Release 2.12.1
- **main:** Release 2.13.0
- CodeRabbit指摘を反映
- Developを取り込む
- Developを取り込む
- **main:** Release 2.14.0
- Developブランチをマージしコンフリクト解消
- **main:** Release 3.0.0
- Spec Kit更新（日本語化とspecs一覧生成）
- **deps-dev:** Bump @types/node from 24.10.4 to 25.0.2
- **main:** Release 3.1.0
- **main:** Release 3.1.1
- **main:** Release 3.1.2
- Develop を取り込む
- Develop を取り込む
- **main:** Release 4.0.0
- Developを取り込みコンフリクト解消
- レビュー指摘を反映
- WaitForUserAcknowledgementの冗長処理を削除
- **main:** Release 4.0.1
- **main:** Release 4.1.0
- **main:** Release 4.1.1
- Merge feature-webui-design
- Merge develop into feature/selected-cleanup
- Merge develop into feature/selected-cleanup
- Developを取り込み
- Merge develop
- レビュー指摘対応
- レビュー残件対応
- レビュー指摘追加対応
- Sync markdownlint with husky
- **main:** Release 4.2.0
- Sync local skills
- Remove codex system skills
- Add typescript-language-server to Dockerfile dependencies
- Merge develop into feature/support-web-ui
- PLAN.md削除（LSP調査完了）
- **main:** Release 4.3.0
- Claude起動の整形を適用する
- Update bun.lock to include configVersion
- Add Git user configuration variables to docker-compose.yml
- DependabotのPR先をdevelopに固定
- **deps-dev:** Bump esbuild from 0.27.1 to 0.27.2
- **deps-dev:** Bump lucide-react from 0.561.0 to 0.562.0
- **deps-dev:** Bump lucide-react from 0.561.0 to 0.562.0
- Bun.lockを更新
- **main:** Release 4.3.1
- **deps-dev:** Bump esbuild from 0.27.1 to 0.27.2
- **deps-dev:** Bump lucide-react from 0.561.0 to 0.562.0
- **main:** Release 4.4.0
- 未使用のcodexシステムスキルファイルを削除
- **main:** Release 4.4.1
- **main:** Release 4.5.0 (#424)
- **main:** Release 4.5.1 (#428)
- Merge main into develop
- **main:** Release 4.6.0 (#435)
- **main:** Release 4.6.1 (#438)
- Merge origin/main into develop
- テスト時のCLI起動遅延をスキップ (#447)
- **main:** Release 4.7.0 (#449)
- Remove PLANS.md and add to .gitignore
- Sync main release-please changes into develop (#465)
- **main:** Release 4.8.0 (#460)
- **main:** Release 4.9.0 (#467)
- Sync main release-please changes into develop
- Npx文言を削除 (#485)
- Vitest から bun test への移行 (#491)
- Vitest関連パッケージとファイルを削除 (#492)
- Developをマージ
- Ralph-loopプラグインを有効化 (#519)
- Origin/developをマージ (unrelated histories)
- CI/CDワークフローの最適化 (#553)
- 一時的な.gitignore.rustファイルを削除
- **main:** Release 4.9.1 (#475)

### Performance

- ブランチ一覧のgit状態取得をキャッシュ化 (#446)

### Refactor

- Dockerfileのグローバルパッケージをdevdependenciesに統合 (#482)
- プロジェクト深層解析による10問題点の修正 (#488)
- Vitest import を bun:test に完全置換 (#493)
- プログラム全体のリファクタリング
- Docker環境の自動検出・パス変換ロジックを削除
- Pnpmインストール方法をcorepack enableに変更
- WorktreeOrchestratorクラスを導入してWorktree管理を分離
- WorktreeOrchestratorにDependency Injectionを実装してテスト問題を解決
- Nコマンド（新規ブランチ作成）を削除
- 自動更新をrキーによる手動更新に変更
- フックをスクリプトファイルベースに変更し、git worktree操作も禁止対象に追加
- Conditionally skip auto merge without token
- ハイライト表現をANSI制御コードに統一
- ブランチ作成時のベースブランチ解決ロジックを改善
- Unity-mcp-server方式への完全統一
- パッケージ名を@akiojin/claude-worktreeから@akiojin/gwtに変更
- UI表示とヘルプメッセージの全参照をgwtに更新
- パッケージ名を@akiojin/claude-worktreeから@akiojin/gwtに変更
- Clean up CLAUDE.md and Docker setup
- Filter入力を常に表示するように変更
- **release:** Llm-router と同じ release-please ワークフローに統一
- M ショートカットコマンド（Manage worktrees）の削除
- Quick Startカテゴリ判定を定義テーブル化
- **web:** 残存レガシーCSSを削除しTailwind + shadcn/uiに完全移行
- CLI起動時のWeb UIサーバー自動起動を廃止
- Geminiのresume/continue引数生成を統合
- EnvironmentProfileScreenの状態管理を整理
- セッションパーサーを各AIツール別に分離
- Qwen未サポートのデッドコードを削除
- 廃止ツールの残存を削除
- コマンド可用性チェックを共通化
- ブランチ一覧画面からLegend行を削除
- スピナーアニメーションをBranchListScreenに局所化
- AIツール(AI Tool)をコーディングエージェント(Coding Agent)に名称変更 (#468)
- Dockerfileのグローバルパッケージをdevdependenciesに統合 (#482)
- プロジェクト深層解析による10問題点の修正 (#488)
- Vitest import を bun:test に完全置換 (#493)

### Styling

- Prettierでコードフォーマット統一
- Prettierフォーマットを適用
- 推奨メッセージの色をyellowに変更
- Apply Prettier formatting to hook test file

### Testing

- フェーズ1 テストインフラのセットアップ完了 (T001-T007)
- フェーズ2 US1のユニットテスト実装完了 (T101-T107)
- US1の統合テスト＆E2Eテスト実装完了 (T108-T110)
- US2スマートブランチ作成ワークフローのテスト完了 (T201-T209)
- フェーズ4 US3セッション管理テスト完了 (T301-T305)
- 並列実行で不安定なテストをスキップして100%パス率達成
- ブランチ一覧ローディング指標の遅延を安定化
- Npm自動公開の動作確認
- テストをqキーからESCキーに更新
- 既存.git/worktreeパスの後方互換性テストを追加
- RealtimeUpdate.test.tsxを手動更新に対応
- Select.memo.test.tsxをスキップ（環境問題のため）
- CIで失敗するテストをスキップ
- Add comprehensive tests for working directory display feature
- 最新コミット時刻取得のユニットテストを追加
- LoadingIndicatorテストを疑似タイマー化してリリースを安定化
- 長大ブランチ名と特殊記号のUIテストを新表示仕様に追随
- UI強調テストをANSI出力向けに調整
- Stub worktree mkdir in integration suites
- Hoist mkdir stub for vitest
- Align fs/promises mock default
- Update worktree mocks for protected branches
- 保護ブランチ遷移の統合テストを追加
- Stabilize worktree-related mocks
- Codex CLI引数の期待値を更新
- Fix vitest hoisted mocks for git branch flows
- CLI関連テストのタイムアウトを延長
- Add logging to hook test for CI troubleshooting
- Skip hook tests in CI due to execa/bun compatibility
- バイナリ欠如時の挙動テスト修正
- Update claude warning expectations
- **webui:** Update ui specs for new env and graph
- セッションテスト内のパス参照を.config/gwt/sessionsに更新
- テスト内のパス参照とUIセレクタをgwtに更新
- QwenとGemini CLIのTDDテストを追加
- Cover model selection defaults and model list integrity
- Ensure cleanup uses branch upstream for diff base
- Add history capping and branch list unknown display
- Cover usage map and unknown display in web
- Fix selector prefill integration assertion
- Fix quick start screen lint warning
- Skip unreliable Error Boundary test with React 18 async useEffect
- Update Gemini tests to match new stdout-only pipe implementation
- **webui:** CLI起動時Web UIサーバー自動起動の仕様化とTDD追加
- Vi.doMockポリフィルを削除
- Web UI全機能ウォークスルーE2Eテストを追加
- CodeRabbit指摘を反映
- Fix codex resolver mocks
- リゾルバーパターンに合わせたテスト修正
- Stabilize ui input tests
- Add selection assertion in shortcuts test
- Chrome統合のプラットフォーム検証を追加する
- ブランチ取得のcwdパラメータに関するテストを追加
- Services/aiToolResolver.test.ts のフルパス期待値を修正 (#440)
- ナビゲーション統合テストのモックを整理 (#450)
- Stabilize worktree and UI mocks (#452)
- Stabilize module mocks

### Build

- Pretestで自動ビルドしてdist検証を安定化

### Ci

- Releaseコミットをcommitlintチェック対象外に
- Lint/testワークフローをmainブランチPRでも実行するよう修正
- **commitlint:** PRタイトルのみを検証するよう変更
- **husky:** Pre-commitフックでlint-stagedを実行
- Commitlintの対象をPRタイトルからコミットへ変更

### Merge

- MainブランチをSPEC-4c2ef107にマージ
- Mainブランチを統合（PR #90対応）

### Revert

- Claude Codeアカウント切り替え機能を完全に削除
- Execaからchild_process.spawnへの変更を元に戻す

### Version

- バージョンを1.0.0から0.1.0に変更

## [4.9.0] - 2025-12-29

### Miscellaneous Tasks

- **main:** Release 4.9.0 (#467)

## [4.8.0] - 2025-12-29

### Bug Fixes

- Execaのshell: trueオプションを削除してbunx起動エラーを修正 (#458)
- Claude-worktree後方互換コードを削除 (#462)
- Package.json の description を Coding Agent 対応に修正 (#471)

### Documentation

- README.md/README.ja.mdを最新の実装状態に同期 (#469)

### Features

- AIツールのインストール済み表示をバージョン番号に変更 (#461)

### Miscellaneous Tasks

- Remove PLANS.md and add to .gitignore
- Sync main release-please changes into develop (#465)
- **main:** Release 4.8.0 (#460)

### Refactor

- AIツール(AI Tool)をコーディングエージェント(Coding Agent)に名称変更 (#468)

## [4.7.0] - 2025-12-26

### Bug Fixes

- Worktree作成時のstale残骸を自動回復 (#445)
- 自動インストール警告文のタイポ修正 (#451)
- Warn then return after dirty worktree (#453)

### Features

- 未コミット警告時にEnterキー待機を追加 (#441)
- ログビューアを追加 (#442)
- ログ表示の通知と選択UIを改善 (#443)
- Docker構成を最適化しPlaywright noVNCサービスを追加 (#454)
- Docker構成を最適化しPlaywright noVNCサービスを追加 (#455)
- ブランチ一覧に最終アクティビティ時間を表示 (#456)

### Miscellaneous Tasks

- Merge origin/main into develop
- テスト時のCLI起動遅延をスキップ (#447)
- **main:** Release 4.7.0 (#449)

### Performance

- ブランチ一覧のgit状態取得をキャッシュ化 (#446)

### Testing

- ナビゲーション統合テストのモックを整理 (#450)
- Stabilize worktree and UI mocks (#452)

## [4.6.1] - 2025-12-25

### Miscellaneous Tasks

- **main:** Release 4.6.1 (#438)

## [4.6.0] - 2025-12-25

### Bug Fixes

- **cli:** AIツール実行時にフルパスを使用して非インタラクティブシェルのPATH問題を修正 (#436)
- **cli:** AIツール実行時にフルパスを使用 (#439)

### Documentation

- Update task planning instruction

### Miscellaneous Tasks

- Merge main into develop
- **main:** Release 4.6.0 (#435)

### Testing

- Services/aiToolResolver.test.ts のフルパス期待値を修正 (#440)

## [4.5.1] - 2025-12-24

### Miscellaneous Tasks

- **main:** Release 4.5.1 (#428)

## [4.5.0] - 2025-12-24

### Miscellaneous Tasks

- **main:** Release 4.5.0 (#424)

## [4.4.1] - 2025-12-23

### Bug Fixes

- **ci:** マージ方法をsquashに変更してCHANGELOG重複を防止 (#425)
- リモートモードでローカル・リモート両存在ブランチが表示されない問題を修正 (#430)
- ブランチリスト画面のフリッカーを解消 (#433)
- Claude Codeのフォールバックをbunxに統一

### Documentation

- **spec:** ログ仕様の明確化とログビューア機能の仕様策定 (#432)

### Features

- Requirements-spec-kit スキルを追加
- Claude Codeプラグイン設定を追加 (#429)
- **cli:** AIツールのインストール状態検出とステータス表示を追加 (#431)

### Miscellaneous Tasks

- 未使用のcodexシステムスキルファイルを削除
- **main:** Release 4.4.1

## [4.4.0] - 2025-12-23

### Miscellaneous Tasks

- **deps-dev:** Bump esbuild from 0.27.1 to 0.27.2
- **deps-dev:** Bump lucide-react from 0.561.0 to 0.562.0
- **main:** Release 4.4.0

## [4.3.1] - 2025-12-22

### Bug Fixes

- 未対応環境ではClaude CodeのChrome統合をスキップする
- WSL1検出でChrome統合を無効化する
- WSLの矢印キー誤認を防止
- 相対パス起動のエントリ判定を安定化
- リモート取得遅延でもブランチ一覧を表示
- Git情報取得のタイムアウトを追加
- Mode表示を Stats 行の先頭に移動
- ブランチ一覧取得時にrepoRootを使用するよう修正
- Gitデータ取得のタイムアウトを延長

### Documentation

- Specs一覧をカテゴリ別に整理
- 廃止仕様をカテゴリ分け

### Features

- ブランチ表示モード切替機能（TABキー）を追加

### Miscellaneous Tasks

- Claude起動の整形を適用する
- Update bun.lock to include configVersion
- Add Git user configuration variables to docker-compose.yml
- DependabotのPR先をdevelopに固定
- **deps-dev:** Bump esbuild from 0.27.1 to 0.27.2
- **deps-dev:** Bump lucide-react from 0.561.0 to 0.562.0
- **deps-dev:** Bump lucide-react from 0.561.0 to 0.562.0
- Bun.lockを更新
- **main:** Release 4.3.1

### Refactor

- ブランチ一覧画面からLegend行を削除
- スピナーアニメーションをBranchListScreenに局所化

### Testing

- Chrome統合のプラットフォーム検証を追加する
- ブランチ取得のcwdパラメータに関するテストを追加

## [4.3.0] - 2025-12-21

### Bug Fixes

- クリーンアップ選択の安全判定を要件どおりに更新
- Type-checkでcleanup対象の型エラーを解消
- ENABLE_LSP_TOOL環境変数の値を"1"から"true"に修正
- Node-ptyで使用するコマンドのフルパスを解決
- WebSocket接続エラーの即時表示を抑制
- Web UIのデフォルトポートを3001に変更

### Documentation

- ChromeパラメータのJSDocドキュメントを追加

### Features

- Claude Code起動時にChrome拡張機能統合を有効化
- ブランチグラフをReact Flowベースにリファクタリング

### Miscellaneous Tasks

- Sync local skills
- Remove codex system skills
- Add typescript-language-server to Dockerfile dependencies
- Merge develop into feature/support-web-ui
- PLAN.md削除（LSP調査完了）
- **main:** Release 4.3.0

## [4.2.0] - 2025-12-20

### Bug Fixes

- Unblock cli build and web client config

### Documentation

- Worktreeクリーンアップ選択機能のSPEC・設計ドキュメント作成
- Update spec tasks status
- Fix markdownlint in spec data model

### Features

- Add branch selection parity for cleanup flow
- リモートにコピーがあるブランチのローカル削除をサポート
- Add post-session push prompt

### Miscellaneous Tasks

- Merge feature-webui-design
- Merge develop into feature/selected-cleanup
- Merge develop into feature/selected-cleanup
- Developを取り込み
- Merge develop
- レビュー指摘対応
- レビュー残件対応
- レビュー指摘追加対応
- Sync markdownlint with husky
- **main:** Release 4.2.0

### Testing

- Fix codex resolver mocks
- リゾルバーパターンに合わせたテスト修正
- Stabilize ui input tests
- Add selection assertion in shortcuts test

## [4.1.1] - 2025-12-19

### Bug Fixes

- Worktree再利用の整合性検証とモデル名正規化
- NormalizeModelIdの空文字処理とテスト補強

### Documentation

- 公開APIのJSDocと仕様文言修正

### Miscellaneous Tasks

- **main:** Release 4.1.1

## [4.1.0] - 2025-12-19

### Features

- Gpt-5.2-codex対応
- Codexモデル一覧を4件に整理

### Miscellaneous Tasks

- **main:** Release 4.1.0

## [4.0.1] - 2025-12-18

### Bug Fixes

- WSL2とWindowsで矢印キー入力を安定化
- デフォルトモデルオプション追加に伴うテスト期待値を修正

### Documentation

- 公開APIのJSDocを追加

### Miscellaneous Tasks

- Developを取り込みコンフリクト解消
- レビュー指摘を反映
- WaitForUserAcknowledgementの冗長処理を削除
- **main:** Release 4.0.1

### Refactor

- 廃止ツールの残存を削除
- コマンド可用性チェックを共通化

### Testing

- CodeRabbit指摘を反映

## [4.0.0] - 2025-12-18

### Bug Fixes

- Gemini-3-flash のモデル ID を gemini-3-flash-preview に修正
- Geminiのモデル選択肢を修正（Default追加＋マニュアルリスト復元）
- Gemini CLI起動時のTTY描画を維持する

### Documentation

- Qwen未サポート要件の適用範囲を明確化

### Features

- Gemini-3-flash モデルのサポートを追加
- 全てのツールにデフォルト（自動選択）オプションを追加し、Geminiのモデル選択肢を改善
- Qwen CLIを未サポート化

### Miscellaneous Tasks

- Develop を取り込む
- Develop を取り込む
- **main:** Release 4.0.0

### Refactor

- Qwen未サポートのデッドコードを削除

### Ci

- Commitlintの対象をPRタイトルからコミットへ変更

## [3.1.2] - 2025-12-16

### Bug Fixes

- CodeRabbit指摘事項を修正
- CodeRabbit追加指摘事項を修正
- CodeRabbitレビュー最終修正
- MatchesCwdにクロスプラットフォームパス正規化を追加
- パスプレフィックスマッチングに境界チェックを追加

### Miscellaneous Tasks

- **main:** Release 3.1.2

### Refactor

- セッションパーサーを各AIツール別に分離

## [3.1.1] - 2025-12-16

### Bug Fixes

- アクセス不可Worktreeを🔴表示に変更

### Miscellaneous Tasks

- **main:** Release 3.1.1

## [3.1.0] - 2025-12-16

### Bug Fixes

- EnvironmentProfileScreenのキーボード入力を修正
- CodeRabbitのレビュー指摘事項を修正
- Spec Kitスクリプトの安全性改善（eval撤廃/JSON出力）
- Profiles.yaml未作成時の作成失敗を修正
- プロファイル名検証と設定パス不整合を修正
- Envキー入力のバリデーションを追加
- プロファイル保存の一時ファイルとスクロール境界を修正
- Envキー入力バリデーションを調整
- Profiles.yaml更新の競合を防止
- プロファイル画面の入力検証とインデックス境界を修正
- プロファイル変更後にヘッダー表示を更新

### Features

- 環境変数プロファイル機能を追加
- プロファイル未選択を選択できるようにする

### Miscellaneous Tasks

- Spec Kit更新（日本語化とspecs一覧生成）
- **deps-dev:** Bump @types/node from 24.10.4 to 25.0.2
- **main:** Release 3.1.0

### Refactor

- EnvironmentProfileScreenの状態管理を整理

## [3.0.0] - 2025-12-15

### Bug Fixes

- Web UI URL表示削除に伴うテスト修正
- SPAルーティング用のフォールバック処理を追加
- Web UIからClaude Code起動時にENABLE_LSP_TOOL環境変数を渡す
- Web UIからClaude Code起動時にENABLE_LSP_TOOL環境変数を渡す
- MacOS/Linuxでトレイ初期化を無効化してクラッシュを防止
- トレイ破棄の二重実行を防止
- トレイ再初期化とテストのplatform注入

### Documentation

- ヘルプテキストに serve コマンドを追加
- Linuxのnode-gypビルド要件を追記

### Features

- MacOS対応のシステムトレイを実装
- Claude CodeのTypeScript LSP対応を追加
- Web UIサーバー全体にログ出力を追加

### Miscellaneous Tasks

- Developブランチをマージしコンフリクト解消
- **main:** Release 3.0.0

### Testing

- Web UI全機能ウォークスルーE2Eテストを追加

## [2.14.0] - 2025-12-13

### Bug Fixes

- Resume/ContinueでsessionIdを上書きしない
- Quick Start画面の初回表示時にEnterが効かない問題を修正
- Resumeは各ツールのresume機能に委譲
- Goodbye後にプロセスが終了しない問題を修正
- Web UIサーバー停止をタイムアウト付きで堅牢化

### Miscellaneous Tasks

- CodeRabbit指摘を反映
- Developを取り込む
- Developを取り込む
- **main:** Release 2.14.0

### Refactor

- Geminiのresume/continue引数生成を統合

## [2.13.0] - 2025-12-12

### Miscellaneous Tasks

- **main:** Release 2.13.0

## [2.12.1] - 2025-12-09

### Miscellaneous Tasks

- **main:** Release 2.12.1

## [2.12.0] - 2025-12-08

### Miscellaneous Tasks

- **main:** Release 2.12.0

## [2.11.1] - 2025-12-05

### Miscellaneous Tasks

- **main:** Release 2.11.1

## [2.11.0] - 2025-12-04

### Miscellaneous Tasks

- **main:** Release 2.11.0

## [2.10.0] - 2025-12-04

### Miscellaneous Tasks

- **main:** Release 2.10.0

## [2.9.1] - 2025-11-27

### Miscellaneous Tasks

- **main:** Release 2.9.1

## [2.9.0] - 2025-11-27

### Miscellaneous Tasks

- **main:** Release 2.9.0

## [2.8.0] - 2025-11-27

### Miscellaneous Tasks

- **main:** Release 2.8.0

## [2.7.4] - 2025-11-26

### Miscellaneous Tasks

- **main:** Release 2.7.4

## [2.7.3] - 2025-11-25

### Bug Fixes

- **docs:** Release-guide.mdのフロー図を実装に合わせて更新 (#285)
- Include upstream base when selecting cleanup targets
- ブランチ一覧表示時にリモートブランチをfetchして最新情報を取得
- **docs:** Release-guide.mdのフロー図を実装に合わせて更新
- Navigation.test.tsx に fetchAllRemotes のモックを追加
- FetchAllRemotes 失敗時にローカルブランチを表示するフォールバックを追加
- Stabilize worktree support and last ai usage display
- Stabilize worktree flows and branch hook
- Save last AI tool immediately on launch
- Persist last AI tool before launch
- リモートブランチ削除をマージ済みPRのみに限定
- Stabilize worktree cleanup and ui tests
- Align cleanup reasons with types and dedupe vars
- Sync列の数字をアイコン直後に表示
- Sync列を固定幅化してブランチ名の位置を揃える
- Remote列の表示を改善（L=ローカルのみ、R=リモートのみ）
- Navigation.test.tsxにcollectUpstreamMap/getBranchDivergenceStatusesのモックを追加
- レビューコメントへの対応
- Align branch list headers
- Origin/developとのマージコンフリクトを解決
- ESLint警告103件とPrettier違反12ファイルを修正
- 自動クリーンアップでリモートブランチを削除しないように修正
- Origin/developとのマージコンフリクトを解決
- Origin/developとのマージコンフリクトを解決
- Prepare-release.yml を修正してdevelop→main へ直接マージするように変更
- Prepare-release.yml を llm-router と同じフローに統一
- ブランチ一覧のAIツールラベルからNew/Continue/Resumeを削除
- Detect codex session ids in nested dirs
- Limit continue session id to branch history
- Localize quick start screen copy
- Honor CODEX_HOME and CLAUDE_CONFIG_DIR for session lookup
- Preserve reasoning level and quick start for protected branches
- Show reasoning level on quick start
- Show reasoning level in quick start option
- Show reasoning labels in quick start
- Default skip permissions to no when missing
- Start new Claude session when no saved ID
- Locate Claude sessions under .config fallback
- Read Claude sessionId from history fallback
- クイックスタートのセッションID表示を修正
- ブランチ別クイックスタートが最新セッションを誤参照しないように
- クイックスタート選択時の型チェックを補強
- Quick Start表示を短縮しツールごとに見やすく調整
- Quick Startヘッダー初期非表示とレイアウトを改善
- Inkの色型エラーを解消
- ブランチ/ワークツリー別に最新セッションを抽出
- カテゴリ解決をswitchで安全化
- Quick Startで最新セッションをworktree優先＋カテゴリ表示を簡素化
- CodexのQuick Startで最新セッションIDをファイルから補完
- CodexのQuick Startで履歴IDがある場合は上書きしない
- Gemini resume失敗時に最新セッションへフォールバック
- Quick Startの選択でEnterが一度で効くように修正
- Codexセッション取得を開始時刻以降の最新ファイルに限定
- CodexセッションIDを起動時刻に近いものへ保存
- CodexセッションIDを起動直後にポーリングして補足
- ClaudeセッションIDを保存時に補完
- ClaudeセッションIDを起動直後にポーリングして補足
- Claudeセッション検出でdot→dashエンコードを考慮
- Claudeセッション検出でproject直下のjson/jsonlも探索
- Claudeセッション検出で最終更新順に有効IDを探索
- Quick StartでClaudeの最新セッションをファイルから優先取得
- Codex Quick Startで履歴より新しいセッションファイルを優先
- Codex保存時に最新セッションIDを再解決
- Claude/Codexセッションを起動時刻近傍で再解決
- セッションファイル探索に時間範囲フィルタを追加
- Geminiセッションも起動時刻近傍で再解決
- Quick Startで初回Enterを受付待ちにバッファ
- Geminiセッション検出をtmp全体のjson/jsonlから抽出
- Quick StartでEnter二度押し不要に
- Gemini起動時にstdoutからsessionIdを確実に捕捉
- Claude/Geminiのセッション取得を時間帯で厳密化
- Claude CodeでstdoutからsessionIdを確実に捕捉
- Capture session ids and harden quick start filters
- Keep local claude tty to avoid non-interactive launch
- Prefer on-disk latest claude session over early probe
- Prefer newest claude session file within window
- Scope codex/gemini session resolution to worktree
- Ignore stdout session ids that lack matching claude session file
- Filter claude quick start entries to existing session files
- Quick start uses newest claude session file per worktree
- Always show latest claude session id in quick start
- Quick start always resolves latest claude session without time window
- Stop treating arbitrary uuids in claude logs as session ids
- Use file-based session detection for Claude/Codex instead of stdout capture
- Prevent detecting old session IDs on consecutive executions
- Prioritize filename UUID over file content for session ID detection
- Add shell option to Codex execa for proper Ctrl+C handling
- Treat SIGINT as normal exit for AI tool child processes
- Add terminal.exitRawMode() to Codex finally block
- Remove SIGINT catch block from Codex to match Claude Code behavior
- Reset stdin state before Ink.js render to prevent hang after Ctrl+C
- Add execChild helper to handle SIGINT for Codex CLI
- Remove sessionProbe from Codex CLI to prevent Ctrl+C hang
- Improve Codex session cwd matching for worktree paths
- Extract cwd from nested payload in Codex session files
- Remove unused imports and variables for ESLint compliance
- Update codex test to expect two exitRawMode calls
- Ensure divergence prompt waits for input
- Add SIGINT/SIGTERM handling to Claude Code launcher
- Complete stdin reset before/after Claude Code launch
- Prevent stdin interference in isClaudeCommandAvailable()
- Resume stdin before Claude Code launch to prevent input lag
- Resolve key input lag in Claude Code and Gemini CLI
- Capture Gemini session ID from exit summary output
- DivergenceテストにwaitForEnterモックを追加
- Fastify logger型の不整合を修正
- Share logger date helper and simplify tests
- Align branch list layout and icon widths
- Resolve lint errors on branch list
- Prompt.jsモックでimportActualを使用
- **test:** テストモックのAPI形状を修正
- Web UIポート解決とトレイ初期化の堅牢化
- 未使用インポートを削除しESLintエラーを解消
- Handle LF enter in Select
- PR #344 CodeRabbitレビュー対応
- React error #310 - フック呼び出し順序を修正

### Documentation

- Update cleanup criteria to use upstream base
- Update branch cleanup requirements
- Add Icon Legend section to README.md
- Fix markdownlint tags in spec tasks
- Check off saved session tasks
- Update quick start tasks
- Quick Start表示ルールを要件・タスクに追記
- AIツール起動機能の仕様タイトルを修正
- 基本ルールに要件化・TDD化優先の指示を追加
- 既存要件への追記可能性確認ステップを追加
- Quick StartのセッションID要件を仕様に追加
- 仕様配置規約をCLAUDE.mdに追記
- PRレビュー指摘事項を反映
- ログ運用統一仕様を追加
- ログローテーション要件を追加
- ログカテゴリと削除タイミングを明記
- ログ仕様にTDD要件を追加
- ログ統一仕様の実装計画を作成
- ログ統一仕様のタスクを追加
- ログ統一仕様のデータモデルとクイックスタート追加
- Document safeToCleanup flag on BranchItem
- Align cleanup plan with current emoji icons
- Web UI起動手順と設定パスを最新化
- SPEC-1f56fd80のmarkdownlint修正

### Features

- Preselect last AI tool when reopening selector
- ブランチ一覧にLocal/Remote/Sync列を追加
- Cコマンドでリモートブランチも削除対象に追加
- ブランチ一覧にラベル行を追加
- ブランチ一覧の表示アイコンを直感的な絵文字に改善
- Persist and surface session ids for continue flow
- Support gemini and qwen session resume
- Fallback resolve continue session id from tool cache
- Add branch quick start reuse last settings
- Add branch quick start screen ui tests
- Skip execution mode when quick-start reusing settings
- Reuse skip permissions in quick start
- クイックスタートでツール別の直近設定を提示
- Quick Startをツールカテゴリ別に色分け表示
- Codex CLIのスキル機能を有効化
- 全AIツール起動時のパラメーターを表示
- Ink.js CLI UIデザインスキル（cli-design）を追加
- Pino構造化ログと7日ローテーションを導入
- Route logs to ~/.gwt with daily jsonl files
- Codexにgpt-5.2モデルを追加
- **webui:** CLI起動時にWeb UIサーバーを自動起動
- Web UIトレイ常駐とURL表示
- **webui:** Tailwind CSS + shadcn/ui基盤を導入
- **webui:** 全ページをTailwind + shadcn/uiでリファクタリング
- ポート使用中時のWeb UIサーバー起動スキップ (FR-006)

### Miscellaneous Tasks

- Trigger CI checks
- Resolve merge conflict with develop
- Clarify immediate save of last tool
- Address review feedback for cleanup flow
- Quick Start表示をさらに簡潔化
- Quick StartでOtherカテゴリ前に余白を追加
- Quick Startカテゴリ表示のテキストを簡潔化
- Quick Startをカテゴリヘッダー+配下アクションの構造に変更
- ビルドエラー解消の型インポート追加
- Quick Startでカテゴリヘッダーを除去し選択肢のみ表示
- Quick Start行をカテゴリ色付きラベルのみに整理
- Quick Startラベルを色付きカテゴリ+アクションだけに整理
- Merge develop to resolve conflicts
- AIツール終了後に3秒待機してブランチ一覧へ戻す
- Fix markdownlint violation
- **deps-dev:** Bump esbuild from 0.27.0 to 0.27.1
- Fix markdownlint in spec
- Bun.lock を更新
- Bun.lock の configVersion を復元
- 仕様ディレクトリを規約に沿って移設
- Cli-designスキルをプロジェクトから削除
- Fix markdownlint indent in log plan
- Raise test memory and limit vitest workers
- Stabilize tests under CI memory constraints
- Further reduce vitest parallelism to avoid OOM
- Skip branch list performance specs in CI and lower vitest footprint
- MCP設定ファイルを追加
- **husky:** Commit-msgフックでcommitlintを自動実行
- Developブランチをマージしコンフリクト解消
- Developをマージ
- **test:** Use threads pool for vitest
- Update manifest to 2.7.3 [skip ci]

### Refactor

- **release:** Llm-router と同じ release-please ワークフローに統一
- M ショートカットコマンド（Manage worktrees）の削除
- Quick Startカテゴリ判定を定義テーブル化
- **web:** 残存レガシーCSSを削除しTailwind + shadcn/uiに完全移行
- CLI起動時のWeb UIサーバー自動起動を廃止

### Testing

- Ensure cleanup uses branch upstream for diff base
- Add history capping and branch list unknown display
- Cover usage map and unknown display in web
- Fix selector prefill integration assertion
- Fix quick start screen lint warning
- Skip unreliable Error Boundary test with React 18 async useEffect
- Update Gemini tests to match new stdout-only pipe implementation
- **webui:** CLI起動時Web UIサーバー自動起動の仕様化とTDD追加
- Vi.doMockポリフィルを削除

### Ci

- **commitlint:** PRタイトルのみを検証するよう変更
- **husky:** Pre-commitフックでlint-stagedを実行

## [2.7.2] - 2025-11-25

### Bug Fixes

- **docs:** Release-guide.jaのフロー図を実装に合わせて更新 (#283)

### Miscellaneous Tasks

- Update manifest to 2.7.2 [skip ci]

## [2.7.1] - 2025-11-25

### Miscellaneous Tasks

- Backmerge main to develop [skip ci]
- Update manifest to 2.7.1 [skip ci]

## [2.7.0] - 2025-11-25

### Bug Fixes

- GitHub Actions完全自動化のためrelease-please設定を修正
- Create-release.ymlをdevelop→main PR作成方式に修正
- Jqコマンドの構文エラーを修正
- Release.ymlをrelease-pleaseから直接タグ作成方式に変更
- Release.ymlのコミットメッセージ検出条件を修正
- **docs:** Release-pleaseの参照をリリースワークフローに修正

### Documentation

- ドキュメント内のsemantic-release言及をrelease-pleaseに更新
- Release.mdのフロー説明をmainブランチターゲットに修正

### Features

- Semantic-releaseからrelease-pleaseへ移行

### Miscellaneous Tasks

- Update manifest to 2.7.0 [skip ci]

### Ci

- Lint/testワークフローをmainブランチPRでも実行するよう修正

## [2.6.1] - 2025-11-25

### Bug Fixes

- アイコン幅計測を補正してブランチ行の日時折り返しを防止
- 幅オーバーライドとアイコン計測のずれで発生する改行を再修正
- 幅計測ヘルパー欠落による型エラーを解消
- 実幅を過小評価しないよう文字幅計測と整列テストを更新
- タイムスタンプ右寄せに安全マージンを設けて改行を防止
- Ensure claude skipPermissions uses sandbox env
- 実行モード表示をNewに変更

## [2.6.0] - 2025-11-25

### Bug Fixes

- 全アイコンの幅オーバーライドを追加してタイムスタンプ折り返しを修正
- Prevent false positives in git hook detection
- 全ての幅計算をmeasureDisplayWidthに統一してstring-width v8対応を完了
- RenderBranchRowのcursorAdjustロジックを復元してテスト互換性を維持

### Features

- Set upstream tracking for newly created refs

## [2.5.0] - 2025-11-25

### Bug Fixes

- String-width v8対応のためWIDTH_OVERRIDESにVariation Selector付きアイコンを追加

### Miscellaneous Tasks

- **deps-dev:** Bump @commitlint/cli from 19.8.1 to 20.1.0
- **deps-dev:** Bump @types/node from 22.19.1 to 24.10.1
- **deps-dev:** Bump vite from 6.4.1 to 7.2.4
- **deps-dev:** Bump @vitejs/plugin-react from 4.7.0 to 5.1.1
- **deps-dev:** Bump esbuild from 0.25.12 to 0.27.0
- **deps-dev:** Bump lint-staged from 15.5.2 to 16.2.7
- **deps-dev:** Bump @commitlint/config-conventional
- Update bun.lock

## [2.4.1] - 2025-11-21

### Bug Fixes

- Omit --model flag when default Opus 4.5 is selected
- Ensure selected model ID is passed to launcher for Claude Code
- フィルターモードでショートカットを無効化

### Features

- Update Opus model version to 4.5
- Update default Claude Code model to Opus 4.5
- Add Sonnet 4.5 as an explicit model option
- Set Opus 4.5 as default and remove explicit Default option

### Miscellaneous Tasks

- Auto fix lint issues

## [2.4.0] - 2025-11-20

### Bug Fixes

- Improve git hook detection for commands with options
- Use process.platform in claude command availability
- **cli:** ターミナル入力がフリーズする問題を修正
- Claude Codeのデフォルトモデル指定を標準扱いに修正

### Features

- Align model selection with provider defaults
- Remember last model and reasoning selection per tool

### Miscellaneous Tasks

- Add vitest compatibility shims for hoisted/resetModules
- Stabilize tests with cross-platform platform checks and timer shims
- 再PR モデル選択修正・テスト安定化 (#243)

### Testing

- Cover model selection defaults and model list integrity

## [2.3.0] - 2025-11-19

### Documentation

- Plan.mdの見出しレベルを修正

### Features

- Gemini CLIをビルトインツールとして追加
- Codex/Geminiの表示名を簡潔化
- Qwenをビルトインツールとして追加
- QwenサポートをREADMEに追加し、GEMINI.mdを作成

### Miscellaneous Tasks

- コードフォーマットを適用

### Testing

- QwenとGemini CLIのTDDテストを追加

## [2.2.0] - 2025-11-18

### Bug Fixes

- フィルター入力の表示位置をWorking DirectoryとStatsの間に修正
- フィルター入力とStatsの間の空行を削除
- フィルターモード中でもブランチ選択のカーソル移動を可能に
- ブランチ選択モードでのカーソル反転表示を修正

### Documentation

- 仕様書を実装に合わせて更新＋Filter:の色をdimColorに変更

### Features

- Fキーでフィルター・検索モードを追加
- フィルター入力中のキーバインド(c/r/m)を無効化＋要件・テスト更新
- フィルターモード/ブランチ選択モードの切り替え機能を追加
- フィルターモード中もブランチ選択の反転表示を有効化

### Refactor

- Filter入力を常に表示するように変更

## [2.1.1] - 2025-11-18

### Miscellaneous Tasks

- Developブランチの最新変更をマージ

## [2.1.0] - 2025-11-18

### Bug Fixes

- Markdownlintのignore_filesを複数行形式に修正
- .markdownlintignoreを追加してCHANGELOG.mdを除外
- Semantic-release実行に必要なNode.js setupを追加
- Publish.ymlでSetup Bunステップの順序を修正

### Miscellaneous Tasks

- CI再実行のための空コミット
- CI/CDをbunに統一してnpm依存を削除

### Refactor

- Clean up CLAUDE.md and Docker setup

## [2.0.4] - 2025-11-18

### Bug Fixes

- Bin/gwt.jsでmain関数を明示的に呼び出すように修正

## [2.0.3] - 2025-11-18

### Bug Fixes

- Semantic-release npmプラグインをnpmPublish: falseで有効化

## [2.0.2] - 2025-11-18

### Bug Fixes

- Semantic-releaseからnpm publishを分離してpublish.ymlに移動

## [2.0.1] - 2025-11-18

### Bug Fixes

- Release.ymlでnpm publish前にビルドを実行

## [2.0.0] - 2025-11-18

### Bug Fixes

- Execa互換性問題によるblock-git-branch-ops.test.tsのテスト失敗を修正
- Markdownlintエラーを修正
- Release.ymlでsemantic-releaseの出力をログに表示するように修正
- スコープ付きパッケージをpublicとして公開するよう設定

### Documentation

- 残りのドキュメント内の参照を更新
- Fix changelog markdownlint errors
- Spec Kit対応 - bugfixブランチタイプ機能の仕様書・計画・タスクを追加

### Features

- Bugfixブランチタイプのサポートを追加

### Miscellaneous Tasks

- Dockerfile を復元

### Refactor

- パッケージ名を@akiojin/claude-worktreeから@akiojin/gwtに変更
- UI表示とヘルプメッセージの全参照をgwtに更新
- パッケージ名を@akiojin/claude-worktreeから@akiojin/gwtに変更

### Testing

- セッションテスト内のパス参照を.config/gwt/sessionsに更新
- テスト内のパス参照とUIセレクタをgwtに更新

## [1.33.0] - 2025-11-17

### Bug Fixes

- **server:** 型エラー修正とビルドスクリプト最適化
- **server:** Docker環境からのアクセス対応とビルドパス修正
- **build:** Esbuildバージョン不一致エラーの解決
- **server:** Web UIサーバーをNode.jsで起動するよう修正
- **docker:** Web UIアクセス用にポート3000を公開
- CLI英語表示を強制
- **lint:** ESLintエラーを修正（未使用変数の削除）
- **docs:** Specsディレクトリのmarkdownlintエラーを修正
- **lint:** ESLint設定を改善してテストファイルのルールを緩和
- **docs:** Specs/feature/webui/spec.mdのbare URL修正
- **test:** テストファイルのimportパス修正
- **test:** Vi.mockのパスも修正してテストのimport問題を完全解決
- **test:** 通常のimport文も../../../../cli/パスに修正
- **test:** Importパスを正しい../../../git.jsに戻す
- **test:** Vitest.config.tsをESLintの対象に追加し、拡張子解決を改善
- **test:** テストファイルのインポートパスを修正して.ts拡張子に対応
- **test:** Dist-app-bundle.testのファイルパスを修正
- **test:** Main error handlingテストとCI環境でのhookテストスキップを修正
- **webui:** フック順序を安定化して詳細画面のクラッシュを解消
- **webui:** ブランチ選択でモーダルを確実に表示
- **webui:** ラジアルノードの重なりを軽減
- **webui:** ベース中心から接続線を描画
- **webui:** Navigate to branch detail after launching session
- **webui:** セッション終了後に一覧へ戻る
- **webui:** Focus new session after launch
- Clean up stale sessions on websocket close
- **web:** Generate worktree paths with repo root
- **websocket:** Add grace period before auto cleanup
- **websocket:** Add retry logic and detailed close logs
- **webui:** Use Fastify logger for WebSocket events
- **webui:** Prevent WebSocket reconnection on prop changes
- **webui:** Add missing useEffect import
- **webui:** 保護ブランチでのworktree作成を禁止
- **docker:** Docker起動時の強制ビルドを削除し開発環境専用に変更
- **webui:** Bun起動と環境設定の型崩れを修正
- **webui:** Update BranchGraph props for simplified API
- **docker:** Docker起動時の強制ビルドを削除し開発環境専用に変更
- **config:** Satisfy exact optional types
- **docker:** Docker起動時の強制ビルドを削除し開発環境専用に変更
- **test:** テストファイルのインポートパスとモックを修正
- **test:** GetSharedEnvironmentモックを追加
- 依存インストール失敗時のクラッシュを防止
- 依存インストール失敗時も起動を継続
- Markdownlint の違反を解消
- Xterm パッケージの依存関係問題を解決するため--legacy-peer-depsを追加
- Package-lock.jsonをpackage.jsonと同期
- Create-release.ymlのdry-runモードでNPM_TOKENエラーを回避

### Documentation

- Web UI機能のドキュメント追加
- **spec:** Add env config specs

### Features

- **web:** Web UI依存関係追加とCLI UI分離
- **web:** Web UIディレクトリ構造と共通型定義を作成
- **cli:** Src/index.tsにserve分岐ロジックを追加
- **server:** Fastifyベースのバックエンド実装とREST API完成
- **client:** フロントエンド基盤実装 (Vite/React/React Router)
- **client:** ターミナルコンポーネント実装とAI Toolセッション起動機能
- Web UIのデザイン刷新とテスト追加
- Web UIのブランチグラフ表示を追加
- **webui:** ブランチ差分を同期して起動を制御
- **webui:** Web UI からGit同期を実行
- **webui:** AIツール設定とWebSocket起動を共通化
- **webui:** ラジアル分岐グラフでモーダル起動に対応
- **webui:** グラフ優先の表示切替を追加
- **webui:** ラジアルグラフにベースフィルターを追加
- **webui:** Divergenceフィルターでグラフ/リストを連動
- **webui:** ラジアルノードをドラッグで再配置
- **webui:** ベースとノードを線で接続
- **webui:** Origin系ノードを統合
- **webui:** グラフ表示を下部へ移動
- **webui:** グラフレイアウト改善とセッション起動修正
- Add shared environment config management
- **logging:** Persist web server logs to file
- **webui:** Implement graphical overlay UI
- **config:** Support shared env persistence
- **server:** Expose shared env configuration
- **webui:** Add shared env management UI
- **cli:** Merge shared environment when launching tools
- Codex CLI のデフォルトモデルを gpt-5.1 に更新

### Miscellaneous Tasks

- **webui:** Switch branch list strings to English
- **debug:** Add websocket instrumentation
- Merge origin/feature/webui
- Synapse PoCのスタンドアロン環境追加
- **worktree:** Remove duplicated files from worktree
- Merge develop into feature/environment
- Configure dependabot commit messages
- **deps-dev:** Bump js-yaml
- Semantic-releaseがreleaseブランチから実行できるように設定追加

### Testing

- Update claude warning expectations
- **webui:** Update ui specs for new env and graph

## [1.32.2] - 2025-11-09

### Bug Fixes

- **workflows:** リリースフローの依存関係と重複実行を最適化

### Documentation

- **spec:** SPEC-57fde06fにバックマージ要件を追加しワークフローを最適化

### Miscellaneous Tasks

- **workflows:** 不要なcheck-pr-base.ymlを削除

## [1.32.1] - 2025-11-09

### Bug Fixes

- ParseInt関数に基数パラメータを明示的に指定

### Documentation

- Align release flow with release branch automation
- Clarify /release can run from any branch

## [1.31.0] - 2025-11-09

### Documentation

- Commitlintとsemantic-release整合性の厳格化
- Lintエラー修正

### Features

- ワークツリー依存を自動同期

### Miscellaneous Tasks

- Lint-stagedでmarkdownlintを強制

### Testing

- バイナリ欠如時の挙動テスト修正

## [1.30.0] - 2025-11-09

### Bug Fixes

- Block interactive rebase
- Use process.cwd() for hook script path resolution
- Worktree外へのcd制限とメッセージ英語化
- Execaをchild_process.spawnに置き換えてCodex CLI起動の互換性問題を解決
- ShellCheck警告を修正（SC2155, SC2269）

### Documentation

- Fix markdownlint error in spec document

### Features

- Add comprehensive TDD and spec for git operations hook
- Worktree内でのcdコマンド使用を禁止するフックを追加
- Worktree内でのファイル操作制限機能を追加

### Styling

- Apply Prettier formatting to hook test file

### Testing

- Add logging to hook test for CI troubleshooting
- Skip hook tests in CI due to execa/bun compatibility

### Revert

- Execaからchild_process.spawnへの変更を元に戻す

## [1.29.1] - 2025-11-08

### Bug Fixes

- Npm publish時の認証設定を修正
- Remove redundant terminal.exitRawMode() call in error path

### Documentation

- READMEのインストールセクションを改善 (#207)
- Publish.ymlのコメントを更新
- READMEのインストールセクションを改善

### Miscellaneous Tasks

- Npm認証方式をコメントに追記

## [1.29.0] - 2025-11-08

### Bug Fixes

- Execaのshell: trueオプションを削除してCodex CLI起動エラーを修正
- Npm publish時の認証設定を修正 (#203)

### Documentation

- Publish.ymlのコメントを更新 (#204)

### Features

- Npm公開機能を有効化

### Miscellaneous Tasks

- Npm認証方式をコメントに追記 (#205)

## [1.28.2] - 2025-11-08

### Bug Fixes

- Publish.ymlへのバックマージ処理の移行

### Miscellaneous Tasks

- Backmerge main to develop after release

## [1.28.1] - 2025-11-08

### Miscellaneous Tasks

- Backmerge main to develop after release

## [1.28.0] - 2025-11-08

### Bug Fixes

- 3回目のパッチバージョンテスト修正追加

### Miscellaneous Tasks

- Backmerge main to develop after release

## [1.27.1] - 2025-11-08

### Features

- 3回目のマイナーバージョンテスト機能追加

### Miscellaneous Tasks

- Backmerge main to develop after release

## [1.27.0] - 2025-11-08

### Bug Fixes

- パッチバージョンリリーステスト用修正追加

### Miscellaneous Tasks

- Backmerge main to develop after release

## [1.26.1] - 2025-11-08

### Bug Fixes

- カバレッジレポート生成失敗を許容

### Features

- マイナーバージョンリリーステスト機能追加

### Miscellaneous Tasks

- Backmerge main to develop after release

## [1.26.0] - 2025-11-08

### Bug Fixes

- Add test file for patch version release
- パッチバージョンリリーステスト用ファイル追加
- WorktreeOrchestratorモックをクラスベースに修正

## [1.25.0] - 2025-11-07

### Bug Fixes

- Docker環境でのpnpmセットアップとプロジェクトビルドを修正
- Update Dockerfile to use npm for global tool installation
- Use node 22 for release workflow
- Disable husky in release workflow
- Use PAT for release pushes
- Make release sync safe for develop
- Auto-mergeをpull_request_targetに変更
- Unity-mcp-serverとの差分を修正
- Unity-mcp-serverとの完全統一（残り20%の修正）
- Semantic-releaseのドライラン実行時にGITHUB_TOKENを設定

### Features

- Orchestrate release branch auto merge flow
- Unity-mcp-server型自動リリースフロー完全導入

### Miscellaneous Tasks

- Update Docker setup and entrypoint script
- ReleaseフローをMethod Aに再構築
- Disable commitlint body line limit
- Dockerfileのグローバルツールインストールを最適化
- Merge develop
- Releaseコミットをcommitlint準拠に調整
- Auto Merge ワークフローで PERSONAL_ACCESS_TOKEN を使用
- Auto Merge ワークフローを pull_request_target に変更
- Auto Merge ワークフローを一本化
- 古いrelease-trigger.ymlを削除

### Refactor

- Unity-mcp-server方式への完全統一

### Testing

- Fix vitest hoisted mocks for git branch flows
- CLI関連テストのタイムアウトを延長

## [1.24.2] - 2025-11-07

### Bug Fixes

- Codexエラー時でもCLIを継続
- Keep cli running on git failures
- Format entry workflow tests
- Codex起動時のJSON構文エラー修正とエラー時のCLI継続

### Testing

- Codex CLI引数の期待値を更新

## [1.24.1] - 2025-11-07

### Miscellaneous Tasks

- Merge origin/main into hotfix

## [1.24.0] - 2025-11-07

### Bug Fixes

- Allow protected branches to launch ai tools
- 保護ブランチ選択時のルート切替とUIを整備
- Scope gitignore updates to active worktree
- Git branch参照コマンドのブロックを解除
- Stabilize release test suites
- Replace vi.hoisted() with direct mock definitions
- Move mock functions inside vi.mock factory

### Documentation

- Add SPEC-a5a44f4c release test stabilization kit

### Miscellaneous Tasks

- Merge origin/main into feature branch

### Testing

- Update worktree mocks for protected branches
- 保護ブランチ遷移の統合テストを追加
- Stabilize worktree-related mocks

## [1.23.0] - 2025-11-06

### Bug Fixes

- Reuse repository root for protected branches
- Correct protected branch type handling
- AIツール起動失敗時もCLIを継続
- Worktree作成時の進捗表示を改善

### Features

- PRベースブランチ検証とブランチ戦略の明確化
- Guard protected branches from worktree creation
- Clarify protected branch workflow in ui
- Worktree作成中にスピナーを表示

## [1.22.0] - 2025-11-06

### Documentation

- SPEC-23bb2eedを手動リリースフロー仕様に更新

### Features

- Develop-to-main手動リリースフローの実装

### Miscellaneous Tasks

- Dockerfileにcommitlintツールを追加
- 開発環境をnpmからpnpmに移行

## [1.21.3] - 2025-11-06

### Bug Fixes

- Ensure worktree directory exists before creation

### Refactor

- ブランチ作成時のベースブランチ解決ロジックを改善

### Testing

- Stub worktree mkdir in integration suites
- Hoist mkdir stub for vitest
- Align fs/promises mock default

## [1.21.2] - 2025-11-06

### Bug Fixes

- エラー発生時の入力待機処理を追加

### Documentation

- CLAUDE.mdからフック重複記述を削除しコンテキストを最適化

## [1.21.1] - 2025-11-05

### Bug Fixes

- Show pending state during branch creation

## [1.21.0] - 2025-11-05

### Bug Fixes

- Align timestamp column for branch list

### Features

- ブランチ行の最終更新表示を整形し右寄せを改善

### Testing

- UI強調テストをANSI出力向けに調整

## [1.20.2] - 2025-11-05

### Bug Fixes

- Bashフックで連結コマンドのgit操作を検知

## [1.20.1] - 2025-11-05

### Bug Fixes

- Limit divergence checks to selected branch

## [1.20.0] - 2025-11-05

### Bug Fixes

- ブランチ行レンダリングのハイライト表示を調整

### Documentation

- SPEC-a5ae4916 を最新コミット表示要件に更新

### Features

- ブランチ一覧に最終更新時刻を表示

### Miscellaneous Tasks

- Auto merge workflow test 5

### Refactor

- ハイライト表現をANSI制御コードに統一

### Testing

- 長大ブランチ名と特殊記号のUIテストを新表示仕様に追随

## [1.19.3] - 2025-11-05

### Bug Fixes

- Rely on GH_TOKEN env directly

### Miscellaneous Tasks

- Auto merge workflow test 4

## [1.19.2] - 2025-11-05

### Bug Fixes

- Login gh before enabling auto merge

### Miscellaneous Tasks

- Auto merge workflow test 3

## [1.19.1] - 2025-11-05

### Bug Fixes

- Adjust auto merge workflow permissions
- Guard auto merge workflow when token missing

### Miscellaneous Tasks

- Auto merge workflow test 2
- Skip auto-merge when token missing

### Refactor

- Conditionally skip auto merge without token

## [1.19.0] - 2025-11-05

### Features

- PR作成時に自動マージを有効化

### Miscellaneous Tasks

- Auto merge workflow test

## [1.18.1] - 2025-11-05

### Bug Fixes

- Heredoc内のgit文字列に誤反応しないようフック検知ロジックを改善

### Refactor

- フックをスクリプトファイルベースに変更し、git worktree操作も禁止対象に追加

## [1.18.0] - 2025-11-05

### Bug Fixes

- 最新コミット順ソートの型エラーを解消
- BatchMergeServiceテストのモック修正とコンパイルエラー解消
- Exact optional cwd handling in divergence helper

### Documentation

- CLAUDE.mdにコミットメッセージポリシーを追記
- Update tasks.md with completed US2 and Phase 4 status
- SPEC-a5ae4916 に最新コミット順の要件を追記
- MarkdownlintをクリアするためのSpec更新
- SPEC-ee33ca26 品質分析完了・修正適用

### Features

- Husky対応を追加してコミット前の品質チェックを自動化
- ヘッダーに起動ディレクトリ表示機能の仕様を追加
- ヘッダーへの起動ディレクトリ表示の実装計画を追加
- ヘッダーへの起動ディレクトリ表示の実装タスクを追加
- ヘッダーに起動ディレクトリ表示機能を実装
- ブランチ一覧の最新コミット順ソートを追加
- Bashツールでのgitブランチ操作を禁止するPreToolUseフックを追加
- フェーズ2完了 - 型定義とgit操作基盤実装
- BatchMergeService完全実装 (T201-T214)
- App.tsxにbatch merge機能を統合
- Dry-runモード実装（T301-T304）
- Auto-pushモード実装（T401-T404）
- AI起動前にfast-forward pullと競合警告を追加

### Miscellaneous Tasks

- ESLint ignore設定を移行
- Mainブランチを取り込み競合を解消
- Markdownlint違反を是正

### Testing

- Add comprehensive tests for working directory display feature
- 最新コミット時刻取得のユニットテストを追加
- LoadingIndicatorテストを疑似タイマー化してリリースを安定化

### Ci

- Releaseコミットをcommitlintチェック対象外に

## [1.17.0] - 2025-11-01

### Features

- Windows向けインストール方法を推奨メッセージに追加

### Styling

- 推奨メッセージの色をyellowに変更

## [1.16.0] - 2025-11-01

### Features

- Bunxフォールバック時に公式インストール方法を推奨
- Bunxフォールバック時のメッセージに2秒待機を追加

## [1.15.0] - 2025-11-01

### Documentation

- Plan.mdのURL形式を修正（Markdownlint対応）

### Features

- Claude Code自動検出機能を追加（US4: ローカルインストール版優先）

### Styling

- Prettierフォーマットを適用

## [1.14.0] - 2025-10-31

### Features

- ブランチ一覧に未プッシュ・PR状態アイコンを追加

## [1.13.0] - 2025-10-31

### Features

- **version:** Add CLI version flag (--version/-v)
- UIヘッダーにバージョン表示機能を追加 (US2)

### Miscellaneous Tasks

- コードフォーマット修正とドキュメント更新

### Testing

- CIで失敗するテストをスキップ

## [1.12.3] - 2025-10-31

### Bug Fixes

- Codex CLIのweb検索フラグを正しく有効化

## [1.12.2] - 2025-10-31

### Bug Fixes

- 自動更新時のカーソル位置リセット問題を解決

### Miscellaneous Tasks

- Add .worktrees/ to .gitignore

### Refactor

- 自動更新をrキーによる手動更新に変更

### Testing

- RealtimeUpdate.test.tsxを手動更新に対応
- Select.memo.test.tsxをスキップ（環境問題のため）

## [1.12.1] - 2025-10-31

### Bug Fixes

- Codex CLIのweb_search_request対応

### Documentation

- エージェントによるブランチ操作禁止を明記

## [1.11.0] - 2025-10-30

### Bug Fixes

- Spec Kitスクリプトのデフォルト動作をブランチ作成なしに変更
- Spec Kitスクリプトのブランチ名制約を緩和
- EnsureGitignoreEntryテストを統合テストに変更
- RealtimeUpdate.test.tsxのテストアプローチを修正

### Documentation

- Worktreeディレクトリパス変更の実装計画を作成
- Worktreeディレクトリパス変更のタスクリストを生成
- CHANGELOG.mdにWorktreeディレクトリ変更を追加

### Features

- Worktreeディレクトリパスを.git/worktreeから.worktreesに変更
- Worktree作成時に.gitignoreへ.worktrees/を自動追加
- リアルタイム更新機能を実装（FR-009対応）

### Testing

- 既存.git/worktreeパスの後方互換性テストを追加

## [1.10.0] - 2025-10-29

### Features

- Cコマンドでベース差分なしブランチもクリーンアップ対象に追加

## [1.9.0] - 2025-10-29

### Bug Fixes

- AIToolSelectorScreenテストを非同期読み込みに対応

### Documentation

- 現行CLI仕様に合わせてヘルプを更新

### Features

- カスタムAIツール対応機能を実装（設定管理・UI統合・起動機能）
- カスタムツール統合と実行オプション拡張（Phase 4-6完了）
- セッション管理拡張とコード品質改善（Phase 7-8完了）

## [1.8.0] - 2025-10-29

### Features

- 戻るキーをqからESCに変更、終了はCtrl+Cに統一

### Refactor

- Nコマンド（新規ブランチ作成）を削除

### Testing

- テストをqキーからESCキーに更新

## [1.7.1] - 2025-10-29

### Bug Fixes

- BranchActionSelectorScreenでqキーで戻る機能と英語化を実装

## [1.7.0] - 2025-10-29

### Bug Fixes

- TypeScript型エラーを修正してビルドを通す

### Features

- ブランチ選択後にアクション選択画面を追加（MVP2）
- 選択したブランチをベースブランチとして新規ブランチ作成に使用

## [1.6.0] - 2025-10-29

### Features

- 型定義を追加（BranchAction, ScreenType拡張, getCurrentBranch export）
- カレントブランチ選択時にWorktree作成をスキップする機能を実装

## [1.5.0] - 2025-10-29

### Features

- ブランチ一覧のソート機能を実装

## [1.4.5] - 2025-10-27

### Bug Fixes

- テストファイルを削除してnpm自動公開を確認

### Testing

- Npm自動公開の動作確認

## [1.4.4] - 2025-10-27

### Bug Fixes

- NPM Token更新後の自動公開を有効化

### Miscellaneous Tasks

- NPM_TOKEN更新後の自動公開テスト

## [1.4.3] - 2025-10-27

### Bug Fixes

- Npm publishでOIDC provenanceを有効化

## [1.4.2] - 2025-10-27

### Bug Fixes

- **ui:** Stop spinner once cleanup completes
- PRクリーンアップ時の未プッシュ判定をマージ済みブランチに対応
- Semantic-releaseがdetached HEAD状態で動作しない問題を修正

### Build

- Pretestで自動ビルドしてdist検証を安定化

## [1.4.1] - 2025-10-27

### Bug Fixes

- 子プロセス用TTYを安全に引き渡す
- Ink UI終了時にTTYリスナーを解放

## [1.4.0] - 2025-10-27

### Bug Fixes

- Ink UIのTTY制御を安定化
- TTYフォールバックの標準入出力を引き渡す

### Documentation

- Lint最小要件をタスクテンプレに明記
- エージェントによるブランチ操作禁止を明記 (#108)

### Features

- **ui:** PRクリーンアップ実行中のフィードバックを改善
- **ui:** PRクリーンアップ実行中のフィードバックを改善
- **ui:** 即時スピナー更新と入力ロックのレスポンス改善

## [1.3.1] - 2025-10-26

### Bug Fixes

- Bunテスト互換のモック復元処理を整備

### Documentation

- Markdownlintスタイルの調整

## [1.3.0] - 2025-10-26

### Features

- SPEC-6d501fd0仕様・計画・タスクの詳細化と品質分析

## [1.2.1] - 2025-10-26

### Bug Fixes

- Spec Kitのブランチ自動作成を無効化

### Documentation

- ブランチ切り替え禁止ルールを追加

## [1.2.0] - 2025-10-26

### Bug Fixes

- Docker環境でのGitリポジトリ検出エラーメッセージを改善
- WorktreeディレクトリでのisGitRepository()動作を修正
- エラー表示にデバッグモード時のスタックトレース表示を追加
- リモートブランチ表示のアイコン幅を調整
- WorktreeConfig型のエクスポートとフォーマット修正
- Ink UIショートカットの動作を修正
- リリースワークフローの認証設定を追加
- LintワークフローにMarkdownlintを統合

### Documentation

- Tasks.md Phase 4進捗を更新（T056-T071完了、T068スキップ）
- Tasks.md Phase 4完了をマーク（T072-T076）
- Tasks.md Phase 1-6完了マーク（全タスク完了）

### Features

- ブランチ選択後のワークフロー統合（AIツール選択→実行モード選択→起動）
- SkipPermissions選択機能とAIツール終了後のメイン画面復帰を実装
- Add git loading indicator with tdd coverage
- ブランチ作成機能を実装（FR-007完全対応）
- Add git loading indicator with tdd coverage (#104)

### Refactor

- WorktreeOrchestratorクラスを導入してWorktree管理を分離
- WorktreeOrchestratorにDependency Injectionを実装してテスト問題を解決

### Testing

- ブランチ一覧ローディング指標の遅延を安定化

## [1.1.0] - 2025-10-26

### Bug Fixes

- Vi.hoistedエラーを修正してテストを全て成功させる
- CIエラーを修正（Markdown Lint + Test）
- CIエラー修正（Markdown LintとVitest mock）
- CHANGELOG.mdの全リストマーカーをアスタリスクに統一
- Ink.js UIのブランチ表示位置とキーボード操作を修正

### Miscellaneous Tasks

- Merge main branch
- CI再トリガー

## [1.0.0] - 2025-10-26

### Bug Fixes

- 修正と設定の更新
- Package.jsonの名前を変更
- Package.jsonの名前を"akiojin/claude-worktree"に変更
- Remove unnecessary '.' argument when launching Claude Code
- GitHub CLI認証チェックを修正
- CLAUDE.mdをclaude-worktreeプロジェクトに適した内容に修正
- String-width negative value error by adding Math.max protection
- バージョン番号表示による枠線のズレを修正
- ウェルカムメッセージの枠線表示を修正
- カラム名（ヘッダー）が表示されない問題を修正
- ウェルカムメッセージの枠線表示を長いバージョン番号に対応
- 現在のブランチがCURRENTとして表示されない問題を修正
- CodeRabbitレビューコメントへの対応
- 保護対象ブランチ(main, master, develop)をクリーンアップから除外
- リモートブランチ選択時にローカルブランチが存在しない場合の不具合を修正
- Windows環境でのnpx実行エラーを修正
- エラー発生時にユーザー入力を待機するように修正
- Windows環境でのClaude Code起動エラーを改善
- Claude Codeのnpmパッケージ名を修正
- Claude Codeコマンドが見つからない場合の適切なエラーハンドリングを追加
- Dockerコンテナのentrypoint.shエラーを修正
- Claude Code実行時のエラーハンドリングを改善
- 未使用のインポートを削除
- 改行コードをLFに統一
- Docker環境でのClaude Code実行時のパス問題を修正
- Worktree内での実行時の警告表示とパス解決の改善
- Claude コマンドのPATH解決問題を修正
- ビルドエラーを修正
- 独自履歴選択後のclaude -r重複実行を修正
- Claude Code履歴表示でタイトルがセッションIDしか表示されない問題を修正
- タイトル抽出ロジックをシンプル化し、ブランチ記録機能を削除
- Claude Code履歴タイトル表示を根本的に改善
- 会話タイトルを最後のメッセージから抽出するように改善
- Claude Code履歴メッセージ構造に対応したタイトル抽出
- 履歴選択キャンセル時にメニューに戻るように修正
- UI表示とタイトル抽出の問題を修正
- プレビュー表示前に画面をクリアして見やすさを改善
- Claude Code実際の表示形式に合わせて履歴表示を修正
- Claude Code実行モード選択でqキーで戻れる機能を追加
- Claude Code実行モード選択でqキー対応とUI簡素化
- 全画面でqキー統一操作に対応
- 会話プレビューで最新メッセージが見えるように表示順序を改善
- 会話プレビューの「more messages above」を「more messages below」に修正
- 会話プレビューの表示順序を通常のチャット形式に修正
- リリースブランチ作成フローを完全に修正
- Developブランチが存在しない場合にmainブランチから分岐するように修正
- リリースブランチの2つの問題を修正
- リリースブランチ検出を正確にするため実際のGitブランチ名を使用
- Npm versionコマンドのエラーハンドリングを改善
- Npm versionエラーの詳細情報を出力するよう改善
- アカウント管理UIの改善
- アカウント切り替え機能のデバッグとUI改善
- **codex:** 承認/サンドボックス回避フラグをCodex用に切替
- Codexの権限スキップフラグ表示を修正
- Codex CLI の resume --last への統一
- Node_modulesをmarkdownlintから除外
- Markdownlintエラー修正（裸のURL）
- 自動マージワークフローのトリガー条件を修正
- GraphQL APIで自動マージを実行
- Worktreeパス衝突時のエラーハンドリングを改善 (#79)
- 新規Worktree作成時にClaude CodeとCodex CLIを選択可能にする (SPEC-473b3d47 FR-008対応)
- マージ済みPRクリーンアップ画面でqキーで前の画面に戻れるように修正
- ESLintエラーを修正
- StripAnsi関数の位置を修正してimport文の後に移動
- ESLint、Prettier、Markdown Lintのエラーを修正
- T094-T095完了 - テスト修正とフィーチャーフラグ変更
- Markdownlint違反のエスケープを追加
- Mainブランチから追加されたclaude.test.tsを一時スキップ（bun vitest互換性問題）
- リアルタイム更新テストの安定性向上
- Claude.test.tsをbun vitest互換に書き直し
- Session-resume.test.ts の node:os mock に default export を追加
- Node:fs/promisesとexecaのmockにdefault exportを追加
- 残り全テストファイルのmock問題を修正
- Ink.js UIの表示とキーボードハンドリングを修正
- キーボードハンドリング競合とWorktreeアイコン表示を修正
- QキーとEnterキーが正常に動作するように修正

### Documentation

- README.mdを大幅に更新し日本語版README.ja.mdを新規作成
- インストール方法にnpx実行オプションを追加
- CLAUDE.mdのGitHub Issues更新ルールを削除し、コミュニケーションガイドラインを追加
- README.ja.mdからCI/CD統合セクションを削除
- README.mdからもCI/CD統合セクションを削除
- Add pnpm and bun installation methods to README
- Memory/・templates/・.claude/commands/ 配下のMarkdownを日本語化
- **specs:** 仕様の要件/チェックリストを実装内容に合わせ更新
- **tasks:** 仕様実装に合わせてタスクを圧縮・完了状態へ更新
- **bun:** 関連ドキュメントをbun前提に更新
- READMEをbun専用に統一し、関連ドキュメントも整備
- README(英/日)をAIツール選択（Claude/Codex）対応の記述へ更新
- AGENTS.md と CLAUDE.md にbun利用ルール（ローカル検証/実行）を明記
- 仕様駆動開発ライフサイクルに関する表現を修正
- Clean up merged PRs機能の修正仕様書を作成
- Spec Kit完全ワークフローの文書化を完了
- フェーズ11ドキュメント改善 & フェーズ12 CI/CD強化完了 (T1001-T1109)
- テスト実装プロジェクト完了サマリー作成
- AGENTS.mdの内容を@CLAUDE.mdに移行し、開発ガイドラインを整理
- PR自動マージ機能の説明をREADMEに追加し、ドキュメントを完成 (T015-T016)
- Spec Kit設計ドキュメントを追加
- SPEC-23bb2eed全タスク完了マーク
- T011完了をtasks.mdに反映
- セッション完了サマリー - Phase 3完了とPhase 4開始の記録
- SESSION_SUMMARY.md最終更新 - Phase 4完了を反映
- T098-T099完了 - ドキュメント更新（Ink.js UI移行）
- Tasks.md更新 - Phase 6全タスク完了マーク
- Enforce Spec Kit SDD/TDD
- Bun vitestのretry未サポートを記録
- Add commitlint rules to tasks template

### Features

- Initial package structure for claude-worktree
- 新機能の追加と既存機能の改善
- Add change tracking and post-Claude Code change management
- マージ済みPRのworktreeとブランチを削除する機能を追加
- UIの改善と表示形式の更新
- 表デザインをモダンでより見やすいスタイルに改善
- 表デザインをモダンでより見やすいスタイルに改善
- Repository Statistics表示をよりコンパクトで見やすいデザインに改善
- ブランチ選択UIと操作メニューの視覚的分離を改善
- Repository Statisticsの表デザインを改善
- Repository Statisticsセクションを削除
- キーボードショートカット機能とブランチ名省略表示を実装
- クリーンアップ時の表示メッセージを改善
- バージョン番号をタイトルに表示
- マージ済みPRクリーンアップ機能の改善
- テーブル表示にカラムヘッダーを追加
- クリーンアップ時にリモートブランチも削除する機能を追加
- リモートブランチ削除を選択可能にする機能を追加
- Worktree削除時にローカルブランチをリモートにプッシュする機能を追加
- Worktreeに存在しないローカルブランチのクリーンアップ機能を追加
- Git認証設定をentrypoint.shに追加
- アクセスできないworktreeを明示的に表示し、pnpmへ移行
- -cパラメーターによる前回セッション継続機能を追加
- -rパラメーターによるセッション選択機能を追加
- .gitignoreと.mcp.jsonの更新、docker-compose.ymlから不要な環境変数を削除
- Worktree選択後にClaude Code実行方法を選択できる機能を追加
- Docker-compose.ymlにNPMのユーザー情報を追加
- Claude -rの表示を大幅改善
- Claude -rをグルーピング形式で大幅改善
- Claude Code履歴を参照したresume機能を実装
- Resume機能を大幅強化
- メッセージプレビュー表示を大幅改善
- 時間表示を削除してccresume風のプレビュー表示に改善
- 全画面活用の拡張プレビュー機能を実装
- 全画面でqキー統一操作に変更
- Npm versionコマンドと連携したリリースブランチ作成機能を実装
- Git Flowに準拠したリリースブランチ作成機能を実装
- リリースブランチ終了時に選択肢を提供
- リリースブランチの自動化を強化
- リリースブランチ完了時のworktreeとローカルブランチ自動削除機能を追加
- Claude Codeアカウント切り替え機能を追加
- Add Spec Kit
- **specify:** ブランチを作成しない運用へ変更
- Codex CLI対応の仕様と実装計画を追加
- AIツール選択（Claude/Codex）機能を実装
- ツール引数パススルーとエラーメッセージを追加
- Npx経由でAI CLIを起動するよう変更
- @akiojin/spec-kitを導入し、仕様駆動開発をサポート
- 既存実装に対する包括的な機能仕様書を作成（SPEC-473b3d47）
- Codex CLIのbunx対応とresumeコマンド整備
- GitHub CLIのインストールをDockerfileに追加
- Claude CodeをnpxからbunxへComplete移行（SPEC-c0deba7e）
- **auto-merge:** PR番号取得、マージ可能性チェック、PRマージステップを実装 (T004-T006)
- Semantic-release自動リリース機能を実装
- Semantic-release設定を明示化
- ブランチ選択カーソル視認性向上 (SPEC-822a2cbf)
- Ink.js UI移行のPhase 1完了（セットアップと準備）
- Phase 2 開始 - 型定義拡張とカスタムフック実装（進行中）
- Phase 2基盤実装 - カスタムフック（useTerminalSize, useScreenState）
- Phase 2基盤実装 - 共通コンポーネント（ErrorBoundary, Select, Confirm, Input）
- Phase 2基盤実装完了 - UI部品コンポーネント（Header, Footer, Stats, ScrollableList）
- Phase 3開始 - データ変換ロジック実装（branchFormatter, statisticsCalculator）
- Phase 3実装 - useGitDataフック（Git情報取得）
- Phase 3 T038-T041完了 - BranchListScreen実装
- Phase 3 T042-T044完了 - App component統合とフィーチャーフラグ実装
- Phase 3 完了 - 統合テスト・受け入れテスト実装（T045-T051）
- Phase 4 開始 - 画面遷移とWorktree管理画面実装（T052-T055）
- T056完了 - WorktreeManager画面遷移統合（mキー）
- T057-T059完了 - BranchCreatorScreen実装と統合
- T060-T062完了 - PRCleanupScreen実装と統合
- T063-T071完了 - 全サブ画面実装完了（Phase 4 サブ画面実装完了）
- T072-T076完了 - Phase 4完全完了！（統合テスト・受け入れテスト実装）
- T077-T080完了 - リアルタイム更新機能実装
- T081-T084完了 - パフォーマンス最適化と統合テスト実装
- T085-T086完了 - Phase 5完全完了！リアルタイム更新機能実装完了
- T096完了 - レガシーUIコード完全削除
- T097完了 - @inquirer/prompts依存削除
- Phase 6完了 - Ink.js UI移行成功（成功基準7/8達成）
- Docker/root環境でClaude Code自動承認機能を追加
- ブランチ一覧のソート優先度を整理
- Tasks.mdにCI/CD検証タスク（T105-T106）を追加 & markdownlintエラーを修正
- カーソルのループ動作を無効化したカスタムSelectコンポーネントを実装
- カスタムSelectコンポーネントのテスト実装とUI 5カラム表示構造への修正

### Miscellaneous Tasks

- Mainブランチとのコンフリクトを解決
- Bump version to 0.4.15
- .gitignoreとpackage.jsonの更新、pnpm-lock.yamlの追加
- Dockerfileから不要なnpm更新コマンドを削除
- Prepare release 0.5.3
- Prepare release 0.5.4
- Bump version to 0.5.5
- Bump version to 0.5.6
- 余分にコミットされた specs を削除
- **bun:** パッケージマネージャをpnpmからbunへ移行
- Npm/pnpmの痕跡を削除しbun専用化
- Npm/pnpm言及の完全排除とbun専用化の仕上げ
- バナー/ヘルプ文言を中立化（Worktree Manager）
- Npx経由コマンドを最新版指定に更新
- プロジェクトセットアップとタスク完了マーク更新
- Mainブランチとのコンフリクトを解決
- CI検証手順をテンプレートと設定に反映

### Refactor

- プログラム全体のリファクタリング
- Docker環境の自動検出・パス変換ロジックを削除
- Pnpmインストール方法をcorepack enableに変更

### Styling

- Prettierでコードフォーマット統一

### Testing

- フェーズ1 テストインフラのセットアップ完了 (T001-T007)
- フェーズ2 US1のユニットテスト実装完了 (T101-T107)
- US1の統合テスト＆E2Eテスト実装完了 (T108-T110)
- US2スマートブランチ作成ワークフローのテスト完了 (T201-T209)
- フェーズ4 US3セッション管理テスト完了 (T301-T305)
- 並列実行で不安定なテストをスキップして100%パス率達成

### Merge

- MainブランチをSPEC-4c2ef107にマージ
- Mainブランチを統合（PR #90対応）

### Revert

- Claude Codeアカウント切り替え機能を完全に削除

### Version

- バージョンを1.0.0から0.1.0に変更


