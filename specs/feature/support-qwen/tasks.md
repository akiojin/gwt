# Tasks: Qwen CLIビルトインツール統合

**仕様ID**: `SPEC-afd20ca6` | **日付**: 2025-11-19 | **仕様書**: [spec.md](./spec.md) | **計画書**: [plan.md](./plan.md)

**生成元**: `/speckit.tasks` コマンド
**TDD原則**: Red（テスト失敗）→ Green（実装）→ Refactor（品質向上）の順序を厳守

## タスク概要

- **合計タスク数**: 21
- **ユーザーストーリー**: 3件（P1-P3）
- **主要成果物**: 4ファイル（新規2、変更2）
- **テストケース**: 16件（T001-T015）
- **推定作業時間**: 2-3時間（TDDフロー含む）

## Phase 1: Setup

### 環境準備

- [ ] **afd20ca6-T001** Bun環境の確認と依存関係インストール
  - **優先度**: P0（必須）
  - **説明**: `bun --version`で動作確認、`bun install`で依存インストール
  - **完了条件**: bunが動作し、node_modulesが最新
  - **推定時間**: 5分

- [ ] **afd20ca6-T002** 既存のビルトインツール実装の調査
  - **優先度**: P0（必須）
  - **説明**: `src/gemini.ts`、`src/config/builtin-tools.ts`、`src/index.ts`を読み、パターンを理解
  - **参照**: src/gemini.ts（最も類似）、src/claude.ts（詳細なエラーハンドリング）
  - **完了条件**: Geminiパターンの理解、カスタマイズ箇所の特定
  - **推定時間**: 15分

## Phase 2: Foundational（P1ブロッカー）

*このフェーズのタスクは並列実行不可（依存関係あり）*

- [ ] **afd20ca6-T003** [P1] [US1] テストファイルの作成（Red - 失敗確認）
  - **優先度**: P1
  - **説明**: `tests/unit/qwen.test.ts`を作成し、16件のテストケース（T001-T015）を実装
  - **ファイル**: tests/unit/qwen.test.ts（新規）
  - **参照**: tests/unit/claude.test.ts（テストパターン）、plan.md 1.4節（テスト設計）
  - **テストケース**:
    - T001-T003: 基本起動テスト
    - T004-T006: モード別起動テスト
    - T007-T008: 権限スキップテスト
    - T009-T011: エラーハンドリングテスト
    - T012-T013: 環境変数テスト
    - T014-T015: ターミナル管理テスト
  - **モック設定**: execa、fs、utils/terminal
  - **完了条件**: テストファイルが存在し、すべてのテストが失敗する（qwen.ts未実装のため）
  - **推定時間**: 45分
  - **TDDフェーズ**: Red

- [ ] **afd20ca6-T004** [P1] [US1] テストの実行とRed確認
  - **優先度**: P1
  - **説明**: `bun run test tests/unit/qwen.test.ts`でテスト実行、すべて失敗を確認
  - **完了条件**: 16件すべてが失敗（launchQwenCLI未定義エラー）
  - **推定時間**: 5分
  - **TDDフェーズ**: Red
  - **依存**: afd20ca6-T003

- [ ] **afd20ca6-T005** [P1] [US1] ✋ ユーザー承認待ち：テストコードレビュー
  - **優先度**: P1
  - **説明**: ユーザーにテストコードをレビュー依頼
  - **レビューポイント**:
    - テストケースが仕様（spec.md）を満たしているか？
    - テストケースが実装を強制していないか？（実装の詳細ではなく動作をテスト）
    - モックの使い方は適切か？
  - **完了条件**: ユーザーの承認
  - **推定時間**: ユーザー依存
  - **TDDフェーズ**: Red
  - **依存**: afd20ca6-T004

## Phase 3: User Story 1（P1）- AIツール選択とQwen起動

*afd20ca6-T005承認後に開始*

### 実装（Green - テスト合格）

- [ ] **afd20ca6-T006** [P1] [US1] src/qwen.ts の作成
  - **優先度**: P1
  - **説明**: Qwen CLI起動ロジックを実装
  - **ファイル**: src/qwen.ts（新規）
  - **参照**: src/gemini.ts（95%コピー＆カスタマイズ）
  - **実装内容**:
    - QwenError クラス定義（name = "QwenError"）
    - launchQwenCLI 関数（async, 戻り値 Promise<void>）
    - isQwenCommandAvailable 内部関数（which/where）
    - isQwenCLIAvailable エクスポート関数（将来用）
  - **カスタマイズ箇所**:
    - パッケージ名: `@google/gemini-cli` → `@qwen-code/qwen-code`
    - コマンド名: `gemini` → `qwen`
    - デフォルト引数: `[]` → `["--checkpointing"]`
    - 権限スキップ: `"-y"` → `"--yolo"`
    - モード引数: すべて `[]`（Qwenに継続・再開オプションなし）
  - **完了条件**: src/qwen.ts が存在し、型エラーなし
  - **推定時間**: 30分
  - **TDDフェーズ**: Green
  - **依存**: afd20ca6-T005

- [ ] **afd20ca6-T007** [P1] [US1] src/config/builtin-tools.ts に QWEN_CLI_TOOL 追加
  - **優先度**: P1
  - **説明**: QWEN_CLI_TOOL定義を作成し、BUILTIN_TOOLS配列に追加
  - **ファイル**: src/config/builtin-tools.ts（変更）
  - **実装内容**:
    ```typescript
    export const QWEN_CLI_TOOL: CustomAITool = {
      id: "qwen-cli",
      displayName: "Qwen",
      type: "bunx",
      command: "@qwen-code/qwen-code@latest",
      defaultArgs: ["--checkpointing"],
      modeArgs: { normal: [], continue: [], resume: [] },
      permissionSkipArgs: ["--yolo"],
    };
    ```
  - **完了条件**: QWEN_CLI_TOOL が BUILTIN_TOOLS 配列の最後に追加
  - **推定時間**: 10分
  - **TDDフェーズ**: Green
  - **並列実行**: afd20ca6-T006と並列可能

- [ ] **afd20ca6-T008** [P1] [US1] src/index.ts に QwenError 処理と分岐追加
  - **優先度**: P1
  - **説明**: QwenErrorのインポートとエラーハンドリング、起動分岐を追加
  - **ファイル**: src/index.ts（変更）
  - **実装内容**:
    - インポート: `import { launchQwenCLI, QwenError } from "./qwen.js";`
    - isRecoverableError関数（3箇所）: `error instanceof QwenError ||`, `error.name === "QwenError" ||`, `name === "QwenError" ||`
    - handleAIToolWorkflow関数: `else if (tool === "qwen-cli") { await launchQwenCLI(...) }`
  - **完了条件**: 3箇所の変更が正しく適用され、型エラーなし
  - **推定時間**: 15分
  - **TDDフェーズ**: Green
  - **依存**: T1006

- [ ] **afd20ca6-T009** [P1] [US1] テストの実行とGreen確認
  - **優先度**: P1
  - **説明**: `bun run test tests/unit/qwen.test.ts`でテスト実行、すべて合格を確認
  - **完了条件**: 16件すべてが合格
  - **推定時間**: 5分
  - **TDDフェーズ**: Green
  - **依存**: afd20ca6-T006, T1008

- [ ] **afd20ca6-T010** [P1] [US1] ビルド確認
  - **優先度**: P1
  - **説明**: `bun run build`でビルド実行、エラーなしを確認
  - **完了条件**: ビルド成功、dist/qwen.js 生成
  - **推定時間**: 5分
  - **TDDフェーズ**: Green
  - **並列実行**: afd20ca6-T009と並列可能

## Phase 4: User Story 2（P2）- セッション管理機能の利用

*注: --checkpointingフラグはafd20ca6-T006で実装済み（FR-002対応）*

- [ ] [afd20ca6-T011] [P2] [US2] セッション管理機能のテスト追加（任意）
  - **優先度**: P2
  - **説明**: /chat save, /chat resumeの動作を手動テストで確認
  - **ファイル**: なし（手動テスト）
  - **テスト手順**:
    1. Qwen CLI起動（--checkpointingフラグ付き）
    2. `/chat save test-session` を実行
    3. 終了後、再起動して `/chat resume test-session` を実行
    4. セッション状態が復元されることを確認
  - **完了条件**: セッション保存・復元が正常に動作
  - **推定時間**: 10分（任意）
  - **依存**: afd20ca6-T010

## Phase 5: User Story 3（P3）- 権限スキップモードでの起動

*注: --yoloフラグはafd20ca6-T006で実装済み（FR-006対応）*

- [ ] [afd20ca6-T012] [P3] [US3] 権限スキップモードの動作確認（任意）
  - **優先度**: P3
  - **説明**: skipPermissions=trueで起動し、--yoloフラグが付与されることを確認
  - **ファイル**: なし（手動テスト）
  - **テスト手順**:
    1. gwtで権限スキップモードを有効化
    2. Qwen CLI起動
    3. 起動ログに「Auto-approving all actions (YOLO mode)」が表示されることを確認
  - **完了条件**: --yoloフラグが正しく機能
  - **推定時間**: 5分（任意）
  - **依存**: afd20ca6-T010

## Phase 6: Polish & Cross-cutting Concerns

### コード品質（Refactor）

- [ ] [afd20ca6-T013] 型チェック
  - **優先度**: P0（必須）
  - **説明**: `bun run type-check`で型エラーがないことを確認
  - **完了条件**: 型エラー0件
  - **推定時間**: 5分
  - **TDDフェーズ**: Refactor
  - **並列実行**: afd20ca6-T014, afd20ca6-T015と並列可能

- [ ] [afd20ca6-T014] リント
  - **優先度**: P0（必須）
  - **説明**: `bun run lint`でリントエラーがないことを確認
  - **完了条件**: リントエラー0件
  - **推定時間**: 5分
  - **TDDフェーズ**: Refactor
  - **並列実行**: afd20ca6-T013, afd20ca6-T015と並列可能

- [ ] [afd20ca6-T015] フォーマット
  - **優先度**: P0（必須）
  - **説明**: `bun run format src/qwen.ts tests/unit/qwen.test.ts`でフォーマット適用
  - **完了条件**: フォーマット適用済み
  - **推定時間**: 5分
  - **TDDフェーズ**: Refactor
  - **並列実行**: afd20ca6-T013, afd20ca6-T014と並列可能

### 全体テスト

- [ ] [afd20ca6-T016] 全テスト実行
  - **優先度**: P0（必須）
  - **説明**: `bun run test`で既存テストが壊れていないことを確認
  - **完了条件**: すべてのテスト（既存+新規）が合格
  - **推定時間**: 10分
  - **依存**: afd20ca6-T013, afd20ca6-T014, afd20ca6-T015

- [ ] [afd20ca6-T017] カバレッジ確認（任意）
  - **優先度**: P2
  - **説明**: `bun run test:coverage`でカバレッジ確認（qwen.ts 80%以上目標）
  - **完了条件**: qwen.ts のカバレッジ80%以上
  - **推定時間**: 5分（任意）
  - **並列実行**: afd20ca6-T016と並列可能

### Git操作

- [ ] [afd20ca6-T018] Git コミット（feat: Qwen追加）
  - **優先度**: P0（必須）
  - **説明**: 変更をコミット（Conventional Commits形式）
  - **コミットメッセージ例**:
    ```
    feat: Qwenをビルトインツールとして追加

    Qwen CLIをビルトインAIツールとして統合。
    - src/qwen.ts を新規作成（起動ロジック、エラーハンドリング）
    - src/config/builtin-tools.ts に QWEN_CLI_TOOL 追加
    - src/index.ts にQwenError処理と分岐ロジックを追加
    - tests/unit/qwen.test.ts を追加（16テストケース）

    主な特徴:
    - --checkpointing フラグでセッション管理を有効化
    - /chat コマンドで対話中にセッション保存・再開可能
    - --yolo フラグで権限スキップモード対応

    SPEC-afd20ca6
    ```
  - **完了条件**: コミット作成完了
  - **推定時間**: 5分
  - **依存**: afd20ca6-T016

- [ ] [afd20ca6-T019] commitlint 検証
  - **優先度**: P0（必須）
  - **説明**: `bunx commitlint --from HEAD~1 --to HEAD`でコミットメッセージ検証
  - **完了条件**: `✔ No problems found`
  - **推定時間**: 2分
  - **依存**: afd20ca6-T018

- [ ] [afd20ca6-T020] Git プッシュ
  - **優先度**: P0（必須）
  - **説明**: `git push`でリモートにプッシュ
  - **完了条件**: プッシュ成功
  - **推定時間**: 2分
  - **依存**: afd20ca6-T019

### ドキュメント

- [ ] [afd20ca6-T021] 実装完了報告
  - **優先度**: P0（必須）
  - **説明**: ユーザーに実装完了を報告、変更ファイル一覧とテスト結果を提示
  - **完了条件**: ユーザーへの報告完了
  - **推定時間**: 5分
  - **依存**: afd20ca6-T020

## 依存関係グラフ

```
afd20ca6-T001, afd20ca6-T002（並列可能）
    ↓
afd20ca6-T003（テスト作成）
    ↓
afd20ca6-T004（Red確認）
    ↓
afd20ca6-T005（ユーザー承認）← ✋ ゲート
    ↓
afd20ca6-T006, afd20ca6-T007（並列可能）
    ↓ afd20ca6-T006
afd20ca6-T008
    ↓
afd20ca6-T009, afd20ca6-T010（並列可能）
    ↓
afd20ca6-T011, afd20ca6-T012（任意、並列可能）
    ↓
afd20ca6-T013, afd20ca6-T014, afd20ca6-T015（並列可能）
    ↓
afd20ca6-T016, afd20ca6-T017（並列可能）
    ↓ afd20ca6-T016
afd20ca6-T018
    ↓
afd20ca6-T019
    ↓
afd20ca6-T020
    ↓
afd20ca6-T021
```

## 実行ガイダンス

### 並列実行可能なタスク

- **フェーズ1**: afd20ca6-T001, afd20ca6-T002
- **フェーズ3**: afd20ca6-T006, afd20ca6-T007
- **フェーズ3**: afd20ca6-T009, afd20ca6-T010
- **フェーズ4**: afd20ca6-T011, afd20ca6-T012（任意）
- **フェーズ6**: afd20ca6-T013, afd20ca6-T014, afd20ca6-T015
- **フェーズ6**: afd20ca6-T016, afd20ca6-T017

### ゲートポイント

- **afd20ca6-T005**: ユーザー承認が必要。承認前にafd20ca6-T006以降を開始しないこと

### TDD原則の厳守

1. **Red（失敗）**: afd20ca6-T003, afd20ca6-T004（テスト先行）
2. **Green（成功）**: afd20ca6-T006, afd20ca6-T007, afd20ca6-T008, afd20ca6-T009, afd20ca6-T010（実装）
3. **Refactor（品質）**: afd20ca6-T013, afd20ca6-T014, afd20ca6-T015（リファクタリング）

### トラブルシューティング

- **テストが失敗する場合**: モックのリセット確認（beforeEach で vi.clearAllMocks()）
- **型エラーが出る場合**: execa のモック型確認（`const mockExeca = execa as ReturnType<typeof vi.fn>`）
- **ビルドは成功するがテストが失敗**: `bun run clean && bun run build && bun run test`
- **commitlint エラー**: コミットメッセージ形式確認（`type: subject`、subject 100文字以内）

## 参考リソース

- **仕様書**: [spec.md](./spec.md)
- **実装計画**: [plan.md](./plan.md)
- **開発者ガイド**: [quickstart.md](./quickstart.md)
- **Gemini実装**: src/gemini.ts（最も類似）
- **Claude実装**: src/claude.ts（詳細なエラーハンドリング例）
- **Claudeテスト**: tests/unit/claude.test.ts（テストパターン参考）

---

**ドキュメント作成日**: 2025-11-19
**最終更新日**: 2025-11-19
**ステータス**: タスク分解完了、実装待ち
