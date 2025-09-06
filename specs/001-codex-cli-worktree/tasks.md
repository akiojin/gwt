# タスク: Codex CLI対応（claude-worktree起動時のツール選択）

**入力**: `/specs/001-codex-cli-worktree/` の設計ドキュメント
**前提**: plan.md（必須）、research.md、data-model.md、contracts/cli-interface.yaml

## 実行フロー（main）
```
1. 機能ディレクトリの plan.md を読み込む
   → ない場合: ERROR "実装計画が見つかりません"
   → 抽出: 技術スタック、ライブラリ、構成
2. 任意の設計ドキュメントを読み込む:
   → data-model.md: エンティティを抽出 → モデルタスク
   → contracts/: 各ファイル → コントラクトテストタスク
   → research.md: 決定事項を抽出 → セットアップタスク
3. カテゴリ別にタスクを生成:
   → Setup: プロジェクト初期化、依存、Lint 設定
   → Tests: コントラクトテスト、統合テスト
   → Core: モデル、サービス、CLI コマンド
   → Integration: DB、ミドルウェア、ログ
   → Polish: ユニットテスト、性能、ドキュメント
4. タスクルールを適用:
   → 異なるファイル = 並列可として [P]
   → 同一ファイル = 逐次（[P] なし）
   → 実装前にテスト（TDD）
5. タスクに連番を付与（T001, T002...）
6. 依存グラフを生成
7. 並列実行例を作成
8. タスクの完全性を検証:
   → すべての契約にテストがあるか？
   → すべてのエンティティにモデルがあるか？
   → すべてのエンドポイントが実装されるか？
9. 戻り値: SUCCESS（実行可能なタスクが整備）
```

## 形式: `[ID] [P?] 説明`
- **[P]**: 並列実行可能（異なるファイル・依存なし）
- 説明には正確なファイルパスを含める

## パス規約
- **単一プロジェクト（選択）**: リポジトリ直下に `src/`、`tests/`
- 技術スタック: Node.js 18+、TypeScript 5.0+、inquirer、chalk

## フェーズ 3.1: セットアップ
- [ ] T001 実装計画に従いプロジェクト構成を作成（src/models/, src/services/, src/cli/, src/lib/）
- [ ] T002 TypeScript プロジェクトを inquirer/chalk 依存付きで初期化（package.json、tsconfig.json）
- [ ] T003 [P] ESLint とPrettier の設定（.eslintrc.json、.prettierrc）
- [ ] T004 [P] Jest/Vitest のテスト環境設定（jest.config.js または vitest.config.ts）

## フェーズ 3.2: まずテスト（TDD） ⚠️ 3.3 の前に必須
**重要: これらのテストは実装前に必ず作成し、必ず失敗していなければならない**
- [ ] T005 [P] CLI引数解析のコントラクトテスト（tests/contract/test_cli_args.ts）
- [ ] T006 [P] 設定ファイル読み書きのコントラクトテスト（tests/contract/test_config.ts）
- [ ] T007 [P] ツール利用可能性チェックのコントラクトテスト（tests/contract/test_tool_availability.ts）
- [ ] T008 [P] 統合テスト: 対話型選択フロー（tests/integration/test_interactive_selection.ts）
- [ ] T009 [P] 統合テスト: 直接指定での起動（tests/integration/test_direct_launch.ts）
- [ ] T010 [P] 統合テスト: qキーでのキャンセル（tests/integration/test_cancel_operation.ts）
- [ ] T011 [P] 統合テスト: ツール固有オプションのパススルー（tests/integration/test_passthrough_args.ts）
- [ ] T012 [P] 統合テスト: エラーハンドリング（tests/integration/test_error_handling.ts）

## フェーズ 3.3: コア実装（テストが失敗状態になってからのみ）
- [ ] T013 [P] ToolSelection モデル（src/models/tool-selection.ts）
- [ ] T014 [P] ToolConfig モデル（src/models/tool-config.ts）
- [ ] T015 [P] UserPreferences モデル（src/models/user-preferences.ts）
- [ ] T016 [P] ToolAvailability モデル（src/models/tool-availability.ts）
- [ ] T017 [P] ToolArgSpec モデル（src/models/tool-arg-spec.ts）
- [ ] T018 [P] ConfigService（設定ファイル管理）（src/services/config-service.ts）
- [ ] T019 [P] ToolDetectionService（ツール検出）（src/services/tool-detection-service.ts）
- [ ] T020 [P] ToolSelectorService（選択ロジック）（src/services/tool-selector-service.ts）
- [ ] T021 [P] ArgParserService（引数解析）（src/services/arg-parser-service.ts）
- [ ] T022 claude-worktree CLI メインエントリーポイント（src/cli/claude-worktree.ts）
- [ ] T023 対話型プロンプトUI（src/cli/prompts/tool-selection-prompt.ts）
- [ ] T024 コマンドライン引数処理（src/cli/handlers/cli-handler.ts）
- [ ] T025 ツール起動処理（src/cli/handlers/tool-launcher.ts）
- [ ] T026 エラーハンドリングとメッセージ（src/cli/handlers/error-handler.ts）

## フェーズ 3.4: 連携
- [ ] T027 設定ファイルパス解決（~/.claude-worktree/config.json）
- [ ] T028 環境変数サポート（CLAUDE_WORKTREE_DEFAULT_TOOL）
- [ ] T029 ログファイル出力（~/.claude-worktree/logs/selection.log）
- [ ] T030 キャッシュ実装（~/.claude-worktree/cache/tool-availability.json）
- [ ] T031 プロセス終了コード処理（0: 成功、1: エラー、130: キャンセル）
- [ ] T032 カラー出力とプレーンテキストモード切り替え

## フェーズ 3.5: 仕上げ
- [ ] T033 [P] モデルのユニットテスト（tests/unit/models/）
- [ ] T034 [P] サービスのユニットテスト（tests/unit/services/）
- [ ] T035 [P] CLIハンドラのユニットテスト（tests/unit/cli/）
- [ ] T036 起動性能テスト（< 100ms）
- [ ] T037 選択UI応答性能テスト（< 50ms）
- [ ] T038 [P] README.md の更新（使用方法、オプション説明）
- [ ] T039 [P] CHANGELOG.md の更新
- [ ] T040 quickstart.md のすべてのテストシナリオを手動実行
- [ ] T041 npm パッケージのビルドとリンクテスト

## 依存関係
- セットアップ（T001-T004）→ すべてのタスクの前提
- テスト（T005-T012）→ 実装（T013-T026）より先に必須
- モデル（T013-T017）→ サービス（T018-T021）の前に
- サービス（T018-T021）→ CLI（T022-T026）の前に
- コア実装（T013-T026）→ 連携（T027-T032）の前に
- すべての実装 → 仕上げ（T033-T041）の前に

## 並列実行例
```bash
# フェーズ 3.2: テストは並列実行可能
Task agent T005 T006 T007 T008 T009 T010 T011 T012

# フェーズ 3.3: モデルは並列実行可能
Task agent T013 T014 T015 T016 T017

# フェーズ 3.3: サービスは並列実行可能
Task agent T018 T019 T020 T021

# フェーズ 3.5: ユニットテストは並列実行可能
Task agent T033 T034 T035
```

## 完了条件
- [ ] すべてのコントラクトテストが緑（PASS）
- [ ] すべての統合テストが緑（PASS）
- [ ] quickstart.md のすべてのシナリオが動作確認済み
- [ ] 性能目標（起動 < 100ms、UI応答 < 50ms）達成
- [ ] ドキュメント更新完了

## 注意事項
1. **TDD厳守**: T005-T012のテストを作成し、失敗することを確認してから実装に進む
2. **並列実行**: [P]マークのタスクは独立したファイルなので並列実行可能
3. **設定パス**: ~/.claude-worktree/ を使用（既存の ~/.worktree/ ではない）
4. **コマンド名**: claude-worktree を維持（worktree への変更は行わない）
5. **ツール固有引数**: -- 以降の引数は透過的にツールへ渡す

---
*Constitution v2.1.1 準拠 - TDD必須、並列実行推奨*