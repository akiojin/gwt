# タスク: カスタムAIツール対応機能

**入力**: `/specs/SPEC-30f6d724/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、research.md、data-model.md、quickstart.md、contracts/types.ts

**テスト**: CLAUDE.mdの指針に従い、TDD絶対遵守のため、すべてのタスクにテストを含めます。

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
- 説明に正確なファイルパスを含める

## Commitlintルール

- コミットメッセージは件名のみを使用し、空にしてはいけません（`commitlint.config.cjs`の`subject-empty`ルール）
- 件名は100文字以内に収めてください（`subject-max-length`ルール）
- タスク生成時は、これらのルールを満たすコミットメッセージが書けるよう変更内容を整理してください

## Lint最小要件

- `.github/workflows/lint.yml` に対応するため、以下のチェックがローカルで成功することをタスク完了条件に含めてください
  - `bun run format:check`
  - `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`
  - `bun run lint`

## フェーズ1: セットアップ（共有インフラストラクチャ）

**目的**: 型定義とビルトインツール定義の準備

### セットアップタスク

- [x] **T001** [P] [共通] contracts/types.tsから型定義をsrc/config/tools.tsにインポート準備
- [x] **T002** [P] [共通] ビルトインツール（Claude Code, Codex CLI）のCustomAITool形式定義を作成

## フェーズ2: ユーザーストーリー1 - カスタムAIツールの設定 (優先度: P1)

**ストーリー**: 開発者が`~/.claude-worktree/tools.json`ファイルを作成・編集して、独自のAIツールを登録できる。設定ファイルには、ツールの実行方法、デフォルト引数、モード別引数、権限スキップ引数、環境変数を定義できる。

**価値**: カスタムツール設定の読み込みと検証機能を提供し、独立してテスト可能。

### テスト作成（TDD: Red）

- [x] **T101** [P] [US1] tests/unit/config/tools.test.ts にloadToolsConfig()のテストを作成
  - ファイル存在/不存在、JSON構文エラー、検証エラーのテストケース
- [x] **T102** [P] [US1] tests/unit/config/tools.test.ts にvalidateToolConfig()のテストを作成
  - 必須フィールド、type値、id重複のテストケース
- [x] **T103** [P] [US1] tests/unit/config/tools.test.ts にgetToolById()とgetAllTools()のテストを作成
  - 存在/不存在ケース、ビルトイン+カスタム統合のテストケース

### 実装（TDD: Green）

- [x] **T104** [US1] T101の後、src/config/tools.ts にloadToolsConfig()を実装
  - ~/.claude-worktree/tools.jsonから設定読み込み
  - JSONパースエラー時のエラーメッセージ表示
  - ファイル不在時は空配列を返す
- [x] **T105** [US1] T102の後、src/config/tools.ts にvalidateToolConfig()を実装
  - 必須フィールド（id, displayName, type, command, modeArgs）の存在チェック
  - typeフィールドの値検証（'path' | 'bunx' | 'command'）
  - id重複チェック
- [x] **T106** [US1] T103の後、src/config/tools.ts にgetToolById()とgetAllTools()を実装
  - getToolById(): IDでツール検索
  - getAllTools(): ビルトイン+カスタムツールの統合

### リファクタリング（TDD: Refactor）

- [x] **T107** [US1] T106の後、src/config/tools.ts のコードをリファクタリング
  - 重複コードの削減
  - エラーメッセージの改善
  - 型安全性の向上

**✅ MVP1チェックポイント**: US1完了後、設定ファイルの読み込みと検証が独立して動作可能

## フェーズ3: ユーザーストーリー2 - カスタムツールの選択と起動 (優先度: P1)

**ストーリー**: ユーザーがclaude-worktreeを起動し、AIツール選択画面でカスタムツールを選択して起動できる。選択したツールは、設定ファイルで定義された実行方式（path/bunx/command）に従って起動され、デフォルト引数とモード別引数が適切に渡される。

**価値**: カスタムツールの実行機能を提供し、3つの実行タイプすべてが独立してテスト可能。

### テスト作成（TDD: Red）

- [x] **T201** [P] [US2] tests/unit/launcher.test.ts にlaunchCustomAITool()のテストを作成（type='path'）
  - 絶対パス実行、引数結合（defaultArgs + modeArgs.normal + extraArgs）のテストケース
- [x] **T202** [P] [US2] tests/unit/launcher.test.ts にlaunchCustomAITool()のテストを作成（type='bunx'）
  - bunx経由実行、引数結合のテストケース
- [x] **T203** [P] [US2] tests/unit/launcher.test.ts にlaunchCustomAITool()のテストを作成（type='command'）
  - PATH解決→実行、引数結合のテストケース
- [x] **T204** [P] [US2] tests/unit/launcher.test.ts にresolveCommand()のテストを作成
  - which/whereコマンド実行、コマンド不在時のエラーのテストケース

### 実装（TDD: Green）

- [x] **T205** [US2] T201の後、src/launcher.ts にtype='path'の実装
  - 絶対パスで直接実行
  - execaでプロセス起動
  - stdio: "inherit"で標準入出力継承
- [x] **T206** [US2] T202の後、src/launcher.ts にtype='bunx'の実装
  - bunx経由でパッケージ実行
  - 引数配列の構築: ["bunx", command, ...args]
- [x] **T207** [US2] T203の後、src/launcher.ts にtype='command'の実装
  - resolveCommand()でPATH解決
  - 解決後のパスで実行
- [x] **T208** [US2] T204の後、src/launcher.ts にresolveCommand()を実装
  - which（Unix/Linux）またはwhere（Windows）コマンド実行
  - コマンド不在時は明確なエラーメッセージ
- [x] **T209** [US2] T205-T208の後、src/launcher.ts に引数結合ロジックを実装
  - buildArgs(): defaultArgs + modeArgs[mode] + extraArgs の順で結合

### UI統合（TDD: Green）

- [x] **T210** [P] [US2] tests/unit/ui/components/screens/AIToolSelectorScreen.test.tsx にカスタムツール表示のテストを作成
  - getAllTools()から取得したツール一覧の表示テスト
- [x] **T211** [US2] T210の後、src/ui/components/screens/AIToolSelectorScreen.tsx を修正
  - toolItemsのハードコードを削除
  - getAllTools()からツールリストを動的取得
  - AITool型をstringに変更（カスタムIDに対応）

### リファクタリング（TDD: Refactor）

- [x] **T212** [US2] T209とT211の後、src/launcher.ts とAIToolSelectorScreen.tsxをリファクタリング
  - コードの可読性向上
  - エラーハンドリングの統一

**✅ MVP2チェックポイント**: US2完了後、カスタムツールの選択と起動が完全に動作可能

## フェーズ4: 既存起動ロジックのリファクタリング（基盤）

**目的**: Claude Code/Codex CLIの既存起動ロジックをlaunchCustomAITool()を使用するようにリファクタリング

### テスト作成

- [ ] **T301** [P] [基盤] tests/unit/claude.test.ts にlaunchClaudeCode()のリグレッションテストを作成
  - 既存の動作（モード別引数、権限スキップ）が維持されることを確認
- [ ] **T302** [P] [基盤] tests/unit/codex.test.ts にlaunchCodexCLI()のリグレッションテストを作成
  - 既存の動作（デフォルト引数、モード別引数）が維持されることを確認

### リファクタリング

- [ ] **T303** [基盤] T301の後、src/claude.ts をリファクタリング
  - launchCustomAITool()を内部で使用
  - ビルトインツール定義（claude-code）を参照
  - 既存のインターフェースを100%維持
- [ ] **T304** [基盤] T302の後、src/codex.ts をリファクタリング
  - launchCustomAITool()を内部で使用
  - ビルトインツール定義（codex-cli）を参照
  - 既存のインターフェースを100%維持

### 後方互換性テスト

- [ ] **T305** [基盤] T303とT304の後、既存の全テストを実行
  - すべての既存テストが100%パスすることを確認

**✅ 基盤チェックポイント**: 既存のClaude Code/Codex CLI起動ロジックが完全に動作

## フェーズ5: ユーザーストーリー3 - 実行モードと権限スキップ (優先度: P2)

**ストーリー**: ユーザーがカスタムツール起動時に実行モード（normal/continue/resume）を選択し、必要に応じて権限スキップオプションを有効化できる。

**価値**: 柔軟な起動オプション機能を提供し、独立してテスト可能。

### テスト作成（TDD: Red）

- [ ] **T401** [P] [US3] tests/unit/launcher.test.ts にモード別引数結合のテストを追加
  - modeArgs.normal, modeArgs.continue, modeArgs.resumeの各テストケース
- [ ] **T402** [P] [US3] tests/unit/launcher.test.ts に権限スキップ引数追加のテストを作成
  - permissionSkipArgsが定義されている場合、未定義の場合のテストケース

### 実装（TDD: Green）

- [ ] **T403** [US3] T401の後、src/launcher.ts のbuildArgs()にモード別引数ロジックを追加
  - options.modeに応じてmodeArgs[mode]を選択
  - 未定義の場合は空配列
- [ ] **T404** [US3] T402の後、src/launcher.ts のbuildArgs()に権限スキップロジックを追加
  - options.skipPermissions=trueの場合、permissionSkipArgsを追加
  - 未定義の場合はスキップ

### リファクタリング（TDD: Refactor）

- [ ] **T405** [US3] T404の後、src/launcher.ts のbuildArgs()をリファクタリング
  - 引数結合ロジックの可読性向上

**✅ MVP3チェックポイント**: US3完了後、実行モードと権限スキップが完全に動作

## フェーズ6: ユーザーストーリー4 - 環境変数の設定 (優先度: P3)

**ストーリー**: カスタムツール起動時に、設定ファイルの`env`フィールドで定義された環境変数が設定される。

**価値**: ツール固有の環境設定機能を提供し、独立してテスト可能。

### テスト作成（TDD: Red）

- [ ] **T501** [P] [US4] tests/unit/launcher.test.ts に環境変数設定のテストを作成
  - envフィールドが定義されている場合、未定義の場合のテストケース

### 実装（TDD: Green）

- [ ] **T502** [US4] T501の後、src/launcher.ts に環境変数設定ロジックを追加
  - CustomAITool.envをexecaのenv optionに渡す
  - 未定義の場合は親プロセスの環境変数を継承

### リファクタリング（TDD: Refactor）

- [ ] **T503** [US4] T502の後、環境変数設定ロジックをリファクタリング
  - エラーハンドリング追加

**✅ MVP4チェックポイント**: US4完了後、環境変数設定が完全に動作

## フェーズ7: ユーザーストーリー5 - セッション管理とツール情報の保存 (優先度: P3)

**ストーリー**: 最後に使用したカスタムツールのIDがセッションに保存され、次回起動時に復元される。

**価値**: 作業継続性の向上を提供し、独立してテスト可能。

### テスト作成（TDD: Red）

- [ ] **T601** [P] [US5] tests/unit/config/index.test.ts にSessionData拡張のテストを作成
  - lastUsedToolフィールドの保存・読み込みテスト
  - 後方互換性テスト（lastUsedToolが存在しない古いセッション）

### 実装（TDD: Green）

- [ ] **T602** [US5] T601の後、src/config/index.ts のSessionDataインターフェースを拡張
  - lastUsedTool?: string フィールドを追加
- [ ] **T603** [US5] T602の後、src/index.ts のhandleAIToolWorkflow()を修正
  - ツール使用後にsaveSession()でlastUsedToolを保存
  - loadSession()でlastUsedToolを読み込み

### リファクタリング（TDD: Refactor）

- [ ] **T604** [US5] T603の後、セッション管理ロジックをリファクタリング
  - エラーハンドリング追加
  - コードの可読性向上

**✅ 完全な機能**: US5完了後、すべての要件が満たされます

## フェーズ8: 統合とポリッシュ

**目的**: すべてのストーリーを統合し、プロダクション準備を整える

### エッジケース対応

- [ ] **T701** [P] [統合] tests/integration/edge-cases.test.ts にエッジケーステストを作成
  - コマンドパス不在、PATH解決失敗、id重複、長いdefaultArgsのテスト
- [ ] **T702** [統合] T701の後、エッジケースのエラーハンドリングを実装
  - 明確なエラーメッセージ
  - ユーザーフレンドリーな表示

### E2Eテスト

- [ ] **T703** [P] [統合] tests/integration/custom-tool-launch.test.ts にE2Eテストを作成
  - カスタムツール登録 → 起動（normalモード）
  - カスタムツール登録 → 起動（continueモード、権限スキップあり）
  - 設定ファイル不在 → ビルトインツールのみ表示
  - JSON構文エラー → エラーメッセージ表示
- [ ] **T704** [統合] T703の後、E2Eテストをすべてパス

### Lint & Build

- [ ] **T705** [統合] すべての実装完了後、`bun run type-check`を実行し、型エラーを修正
- [ ] **T706** [統合] T705の後、`bun run lint`を実行し、lintエラーを修正
- [ ] **T707** [統合] T706の後、`bun run format:check`を実行し、フォーマットエラーを修正
- [ ] **T708** [統合] T707の後、`bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`を実行し、markdownlintエラーを修正
- [ ] **T709** [統合] T708の後、`bun run test`を実行し、すべてのテストがパス
- [ ] **T710** [統合] T709の後、`bun run test:coverage`を実行し、カバレッジが95%以上
- [ ] **T711** [統合] T710の後、`bun run build`を実行し、ビルドが成功

### ドキュメント

- [ ] **T712** [P] [ドキュメント] README.mdにカスタムツール対応機能のセクションを追加
  - quickstart.mdへのリンク
  - 簡単な使用例
- [ ] **T713** [P] [ドキュメント] CHANGELOG.mdに変更内容を追加
  - カスタムAIツール対応機能の追加
  - 後方互換性の維持

## タスク凡例

**優先度**:
- **P1**: 最も重要 - MVP1に必要
- **P2**: 重要 - MVP2に必要
- **P3**: 補完的 - 完全な機能に必要

**依存関係**:
- **[P]**: 並列実行可能
- **[依存なし]**: 他のタスクの後に実行

**ストーリータグ**:
- **[US1]**: ユーザーストーリー1（カスタムAIツールの設定）
- **[US2]**: ユーザーストーリー2（カスタムツールの選択と起動）
- **[US3]**: ユーザーストーリー3（実行モードと権限スキップ）
- **[US4]**: ユーザーストーリー4（環境変数の設定）
- **[US5]**: ユーザーストーリー5（セッション管理とツール情報の保存）
- **[共通]**: すべてのストーリーで共有
- **[基盤]**: 既存コードのリファクタリング
- **[統合]**: 複数ストーリーにまたがる
- **[ドキュメント]**: ドキュメント専用

## 実装戦略

### MVP定義

**MVP1（US1）**: カスタムツール設定の読み込みと検証
- 設定ファイル読み込み
- 検証機能
- エラーメッセージ表示

**MVP2（US1+US2）**: カスタムツールの選択と起動
- UI統合（ツール選択画面）
- 3つの実行タイプ対応
- 引数結合ロジック

**MVP3（US1+US2+US3）**: 実行モードと権限スキップ
- モード別引数
- 権限スキップオプション

**完全版（US1-US5）**: すべての機能
- 環境変数設定
- セッション管理

### 並列実行の機会

**フェーズ1（セットアップ）**:
- T001とT002は並列実行可能

**フェーズ2（US1）**:
- T101、T102、T103は並列実行可能（すべてテスト作成）

**フェーズ3（US2）**:
- T201、T202、T203、T204は並列実行可能（すべてテスト作成）
- T210は並列実行可能（UI統合テスト）

**フェーズ4（基盤）**:
- T301とT302は並列実行可能（リグレッションテスト）

**フェーズ5-7（US3-US5）**:
- 各フェーズのテスト作成タスクは並列実行可能

**フェーズ8（統合）**:
- T701とT703は並列実行可能（テスト作成）
- T712とT713は並列実行可能（ドキュメント）

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## タスクサマリー

- **総タスク数**: 66
- **フェーズ1（セットアップ）**: 2タスク
- **フェーズ2（US1）**: 7タスク
- **フェーズ3（US2）**: 12タスク
- **フェーズ4（基盤）**: 5タスク
- **フェーズ5（US3）**: 5タスク
- **フェーズ6（US4）**: 3タスク
- **フェーズ7（US5）**: 4タスク
- **フェーズ8（統合）**: 13タスク
- **フェーズ8（ドキュメント）**: 2タスク

- **並列実行可能タスク**: 25タスク
- **独立したMVPチェックポイント**: 5箇所

## 注記

- 各タスクは1時間から1日で完了可能であるべき
- より大きなタスクはより小さなサブタスクに分割
- ファイルパスは正確で、プロジェクト構造と一致させる
- TDD絶対遵守: Red（テスト作成）→ Green（実装）→ Refactor（リファクタリング）
- 各ストーリーは独立してテスト・デプロイ可能
