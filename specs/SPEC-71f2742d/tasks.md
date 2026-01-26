# タスク: カスタムコーディングエージェント登録機能

**仕様ID**: `SPEC-71f2742d`
**入力**: `/specs/SPEC-71f2742d/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、research.md、data-model.md、quickstart.md

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（US1〜US6）
- 説明に正確なファイルパスを含める

## ストーリー間の依存関係

```text
US1 (P1: tools.json読み込み)
  │
  ├──► US2 (P1: エージェント起動) ─► US5 (P2: モデル/バージョン)
  │                                    │
  └──► US3 (P1: TUI登録)              └──► US6 (P2: 履歴統合)
         │
         └──► US4 (P2: タブ統合)
```

- US1 は全ストーリーの前提
- US2, US3 は US1 完了後に並列実行可能
- US4 は US3 に依存
- US5, US6 は US2 に依存

## フェーズ1: セットアップ（共有インフラストラクチャ）

**目的**: tools.rs モジュール作成と基本構造体定義

### セットアップタスク

- [x] **T001** [共通] `crates/gwt-core/src/config/tools.rs` を新規作成し、モジュールヘッダとインポートを追加
- [x] **T002** [共通] T001の後に `crates/gwt-core/src/config/mod.rs` に `pub mod tools;` を追加
- [x] **T003** [P] [共通] `crates/gwt-core/src/lib.rs` で tools モジュールを公開エクスポート

## フェーズ2: ユーザーストーリー1 - tools.jsonからのカスタムエージェント読み込み (P1)

**ストーリー**: 開発者がグローバル/ローカルにカスタムエージェントを定義すると、Wizardで表示される

**価値**: カスタムエージェント機能の基盤

**FR対応**: FR-001, FR-002, FR-003, FR-004, FR-013, FR-016, FR-017, FR-018, FR-019

### テスト（TDD）

- [x] **T101** [P] [US1] `crates/gwt-core/src/config/tools.rs` に ToolsConfig パーステストを追加
- [x] **T102** [P] [US1] `crates/gwt-core/src/config/tools.rs` に CustomCodingAgent バリデーションテストを追加
- [x] **T103** [P] [US1] `crates/gwt-core/src/config/tools.rs` にグローバル/ローカルマージテストを追加
- [x] **T104** [P] [US1] `crates/gwt-core/src/config/tools.rs` に version 未定義時エラーテストを追加

### データ層

- [x] **T105** [US1] T101-104の後に `crates/gwt-core/src/config/tools.rs` に AgentType enum を定義
- [x] **T106** [US1] T105の後に `crates/gwt-core/src/config/tools.rs` に ModeArgs 構造体を定義
- [x] **T107** [US1] T106の後に `crates/gwt-core/src/config/tools.rs` に ModelDef 構造体を定義
- [x] **T108** [US1] T107の後に `crates/gwt-core/src/config/tools.rs` に CustomCodingAgent 構造体を定義
- [x] **T109** [US1] T108の後に `crates/gwt-core/src/config/tools.rs` に ToolsConfig 構造体を定義

### 読み込みロジック

- [x] **T110** [US1] T109の後に `crates/gwt-core/src/config/tools.rs` に load_global() 関数を実装
- [x] **T111** [US1] T110の後に `crates/gwt-core/src/config/tools.rs` に load_local() 関数を実装
- [x] **T112** [US1] T111の後に `crates/gwt-core/src/config/tools.rs` に merge() 関数を実装（ローカル優先）
- [x] **T113** [US1] T112の後に `crates/gwt-core/src/config/tools.rs` に validate() 関数を実装

### Wizard統合

- [x] **T114** [US1] T113の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に AgentEntry 構造体を追加
- [x] **T115** [US1] T114の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に get_all_agents() 関数を追加（ビルトイン+カスタム）
- [x] **T116** [US1] T115の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に色自動割り当てロジックを追加
- [x] **T117** [US1] T116の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の render_agent_select() でセパレータ表示を追加
- [x] **T118** [US1] T117の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に未インストールエージェントのグレーアウト表示を追加

**✅ MVP1チェックポイント**: US1完了後、tools.jsonからカスタムエージェントがWizardに表示される

## フェーズ3: ユーザーストーリー2 - カスタムエージェントの起動 (P1)

**ストーリー**: Wizardでカスタムエージェントを選択すると、定義されたcommandとargsで起動する

**価値**: カスタムエージェントの実際の利用が可能になる

**FR対応**: FR-005, FR-006, FR-007, FR-008, FR-009, FR-010

### テスト（TDD）

- [ ] **T201** [P] [US2] `crates/gwt-cli/src/main.rs` に type:command 起動テストを追加
- [ ] **T202** [P] [US2] `crates/gwt-cli/src/main.rs` に type:path 起動テストを追加
- [ ] **T203** [P] [US2] `crates/gwt-cli/src/main.rs` に type:bunx 起動テストを追加
- [ ] **T204** [P] [US2] `crates/gwt-cli/src/main.rs` に modeArgs 適用テストを追加

### 起動ロジック

- [x] **T205** [US2] T201-204の後に `crates/gwt-cli/src/main.rs` に CustomAgentLaunchConfig 構造体を追加
- [x] **T206** [US2] T205の後に `crates/gwt-cli/src/main.rs` に build_custom_agent_args() 関数を追加
- [x] **T207** [US2] T206の後に `crates/gwt-cli/src/main.rs` に type 別実行分岐を追加（command/path/bunx）
- [x] **T208** [US2] T207の後に `crates/gwt-cli/src/main.rs` に modeArgs 適用ロジックを追加
- [x] **T209** [US2] T208の後に `crates/gwt-cli/src/main.rs` に env 環境変数設定ロジックを追加
- [x] **T210** [US2] T209の後に `crates/gwt-cli/src/main.rs` に permissionSkipArgs 適用ロジックを追加

### Wizard連携

- [x] **T211** [US2] T210の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の WizardState にカスタムエージェント選択状態を追加
- [x] **T212** [US2] T211の後に `crates/gwt-cli/src/tui/screens/wizard.rs` にサポートモード表示制御を追加（modeArgs定義のみ表示）
- [x] **T213** [US2] T212の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に permissionSkipArgs 非表示制御を追加

### tmux連携

- [x] **T214** [US2] T213の後に `crates/gwt-cli/src/tui/app.rs` の build_agent_args_for_tmux() にカスタムエージェント対応を追加

**✅ MVP2チェックポイント**: US1+US2完了後、カスタムエージェントの選択・起動が可能

## フェーズ4: ユーザーストーリー3 - TUIからのカスタムエージェント登録・編集 (P1)

**ストーリー**: TUIの設定画面からカスタムエージェントを新規登録、編集、削除できる

**価値**: JSON手動編集なしでカスタムエージェントを管理可能

**FR対応**: FR-021

### テスト（TDD）

- [x] **T301** [P] [US3] `crates/gwt-core/src/config/tools.rs` に ToolsConfig 保存テストを追加
- [x] **T302** [P] [US3] `crates/gwt-core/src/config/tools.rs` に CustomCodingAgent 追加・更新・削除テストを追加

### 保存ロジック

- [x] **T303** [US3] T301-302の後に `crates/gwt-core/src/config/tools.rs` に save() 関数を実装
- [x] **T304** [US3] T303の後に `crates/gwt-core/src/config/tools.rs` に add_agent() 関数を実装
- [x] **T305** [US3] T304の後に `crates/gwt-core/src/config/tools.rs` に update_agent() 関数を実装
- [x] **T306** [US3] T305の後に `crates/gwt-core/src/config/tools.rs` に remove_agent() 関数を実装

### 設定画面UI

- [x] **T307** [US3] T306の後に `crates/gwt-cli/src/tui/screens/settings.rs` を新規作成（既存ファイル拡張）
- [x] **T308** [US3] T307の後に `crates/gwt-cli/src/tui/screens/settings.rs` に SettingsState 構造体を定義
- [x] **T309** [US3] T308の後に `crates/gwt-cli/src/tui/screens/settings.rs` にカスタムエージェント一覧表示を実装
- [x] **T310** [US3] T309の後に `crates/gwt-cli/src/tui/screens/settings.rs` に新規エージェント追加フォームを実装
- [x] **T311** [US3] T310の後に `crates/gwt-cli/src/tui/screens/settings.rs` にエージェント編集フォームを実装
- [x] **T312** [US3] T311の後に `crates/gwt-cli/src/tui/screens/settings.rs` にエージェント削除確認を実装
- [x] **T313** [US3] T312の後に `crates/gwt-cli/src/tui/screens/mod.rs` に settings モジュールを追加（既存）

### App統合

- [x] **T314** [US3] T313の後に `crates/gwt-cli/src/tui/app.rs` に Screen::Settings を追加（既存）
- [x] **T315** [US3] T314の後に `crates/gwt-cli/src/tui/app.rs` に設定画面への遷移ロジックを追加（既存）

**✅ MVP3チェックポイント**: US1+US2+US3完了後、TUIからカスタムエージェントのCRUD操作が可能

## フェーズ5: ユーザーストーリー4 - 設定画面のタブ統合 (P2)

**ストーリー**: Tabキーでブランチモード→エージェントモード→設定画面を切り替えできる

**価値**: シームレスな画面遷移によるUX向上

**FR対応**: FR-020, FR-022

### テスト（TDD）

- [x] **T401** [P] [US4] `crates/gwt-cli/src/tui/app.rs` にタブ切り替え順序テストを追加（test_tab_cycles_three_screens）

### タブ切り替え実装

- [x] **T402** [US4] T401の後に `crates/gwt-cli/src/tui/app.rs` の handle_key_event() にタブ切り替えロジックを追加（3画面循環）
- [x] **T403** [US4] T402の後に `crates/gwt-cli/src/tui/screens/settings.rs` に Profile設定セクションを追加（既存のGeneral/Worktree/Web/Agentカテゴリで実現）
- [x] **T404** [US4] T403の後に `crates/gwt-cli/src/tui/screens/settings.rs` にカスタムエージェントセクションとProfile セクションの統合表示を実装（CustomAgentsカテゴリとして統合済み）

**✅ フェーズ5チェックポイント**: Tab キーで 3 画面をシームレスに切り替え可能

## フェーズ6: ユーザーストーリー5 - モデル選択とバージョン取得 (P2)

**ストーリー**: カスタムエージェントでもモデル選択とバージョン取得ができる

**価値**: ビルトインと同等の詳細設定機能

**FR対応**: FR-011, FR-012

### テスト（TDD）

- [ ] **T501** [P] [US5] `crates/gwt-cli/src/tui/screens/wizard.rs` にカスタムエージェントモデル選択テストを追加
- [ ] **T502** [P] [US5] `crates/gwt-cli/src/tui/screens/wizard.rs` に versionCommand 実行テストを追加

### モデル選択

- [x] **T503** [US5] T501-502の後に `crates/gwt-cli/src/tui/screens/wizard.rs` にカスタムエージェント用モデル一覧取得を実装
- [x] **T504** [US5] T503の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に models 未定義時のモデル選択スキップを実装

### バージョン取得

- [x] **T505** [US5] T504の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に versionCommand 実行ロジックを追加
- [x] **T506** [US5] T505の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に versionCommand 未定義時のバージョン選択スキップを実装

**✅ フェーズ6チェックポイント**: カスタムエージェントでモデル/バージョン選択が可能

## フェーズ7: ユーザーストーリー6 - セッション履歴とQuick Start (P2)

**ストーリー**: カスタムエージェントもセッション履歴が保存され、Quick Start機能で再利用できる

**価値**: 前回設定の再利用による効率化

**FR対応**: FR-014, FR-015

### テスト（TDD）

- [x] **T601** [P] [US6] `crates/gwt-core/src/ai/agent_history.rs` にカスタムエージェント履歴保存テストを追加（既存テストでカバー）
- [x] **T602** [P] [US6] `crates/gwt-cli/src/tui/screens/wizard.rs` に Quick Start カスタムエージェント復元テストを追加

### 履歴保存

- [x] **T603** [US6] T601-602の後に `crates/gwt-cli/src/main.rs` にカスタムエージェント使用時の履歴保存を追加
- [x] **T604** [US6] T603の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に Quick Start でのカスタムエージェント設定復元を実装

### ビルトインID上書き

- [x] **T605** [US6] T604の後に `crates/gwt-cli/src/tui/screens/wizard.rs` にビルトインIDと同一カスタムIDの上書きロジックを実装

**✅ フェーズ7チェックポイント**: カスタムエージェントの履歴保存と Quick Start 復元が可能

## フェーズ8: 統合とポリッシュ

**目的**: 全ストーリーの統合、品質確認、ドキュメント更新

### 統合テスト

- [ ] **T701** [統合] 全エージェントタイプ（command/path/bunx）のエンドツーエンドテスト実行
- [ ] **T702** [統合] エッジケーステスト実行（JSON パースエラー、未インストールコマンド、ID 重複）

### 品質チェック

- [x] **T703** [統合] `cargo clippy --all-targets --all-features -- -D warnings` をローカルで実行し、警告を修正
- [x] **T704** [統合] `cargo fmt` を実行してフォーマット統一
- [x] **T705** [統合] `cargo test` で全テストパスを確認

### ドキュメント

- [x] **T706** [P] [ドキュメント] `README.md` のカスタムエージェント説明を更新（tools.json スキーマ、使用方法）
- [x] **T707** [P] [ドキュメント] `README.ja.md` に同内容を追加

### コミット

- [ ] **T708** [デプロイ] 全変更を Conventional Commits 形式でコミット（`feat(custom-agent): ...`）
- [ ] **T709** [デプロイ] `bunx commitlint --from HEAD~1 --to HEAD` でコミットメッセージを検証

## タスク凡例

**優先度**:

- **P1**: 最も重要 - US1, US2, US3（基本機能）
- **P2**: 重要 - US4, US5, US6（拡張機能）

**ストーリータグ**:

- **[US1]**: tools.json 読み込み
- **[US2]**: エージェント起動
- **[US3]**: TUI 登録・編集
- **[US4]**: タブ統合
- **[US5]**: モデル/バージョン
- **[US6]**: 履歴統合
- **[共通]**: セットアップ
- **[統合]**: 全体統合
- **[ドキュメント]**: ドキュメント
- **[デプロイ]**: コミット・検証

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化

## 並列実行候補

以下のタスクグループは並列実行可能：

1. **フェーズ1**: T001 と T003
2. **フェーズ2テスト**: T101, T102, T103, T104
3. **フェーズ3テスト**: T201, T202, T203, T204
4. **フェーズ4テスト**: T301, T302
5. **フェーズ6テスト**: T501, T502
6. **フェーズ7テスト**: T601, T602
7. **ドキュメント**: T706, T707
