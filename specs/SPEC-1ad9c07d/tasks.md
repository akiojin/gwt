<!-- markdownlint-disable MD013 -->
# タスク: エージェント起動ウィザード統合 - AIBranchSuggest

**仕様ID**: `SPEC-1ad9c07d`
**入力**: `/specs/SPEC-1ad9c07d/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、data-model.md、contracts/ai-branch-suggest-api.md、research.md

## ストーリー間の依存関係

```text
US6 (AI無効時スキップ) ← 基盤（US5/US7の前提）
US5 (AI候補生成・選択) ← US6完了後
US7 (エラーハンドリング) ← US5完了後
```

US1-US4, US8-US10 は既存実装済みのため、タスク対象外。

## フェーズ1: 基盤（型定義・enumバリアント追加）

**目的**: AIBranchSuggestに必要な型・フィールド・enumバリアントを定義し、既存コードをコンパイル可能に保つ

### テスト

- [ ] **T001** [US6] `crates/gwt-cli/src/tui/screens/wizard.rs` に `BranchType::from_prefix()` のユニットテストを追加: `from_prefix("feature/add-login")` → `Some((Feature, "add-login"))`、`from_prefix("bugfix/fix-crash")` → `Some((Bugfix, "fix-crash"))`、`from_prefix("unknown/name")` → `None`、`from_prefix("no-prefix")` → `None`

### 型定義

- [ ] **T002** [P] [US6] `crates/gwt-cli/src/tui/screens/wizard.rs` に `AIBranchSuggestPhase` enumを定義（Input, Loading, Select, Error）、`Default` は `Input`
- [ ] **T003** [P] [US6] `crates/gwt-cli/src/tui/screens/wizard.rs` の `BranchType` に `from_prefix(name: &str) -> Option<(BranchType, &str)>` メソッドを追加

### WizardStep追加

- [ ] **T004** [US6] `crates/gwt-cli/src/tui/screens/wizard.rs` の `WizardStep` enumに `AIBranchSuggest` バリアントを `IssueSelect` と `BranchNameInput` の間に追加

### WizardState拡張

- [ ] **T005** [US6] `crates/gwt-cli/src/tui/screens/wizard.rs` の `WizardState` structに以下のフィールドを追加: `ai_enabled: bool`, `ai_branch_phase: AIBranchSuggestPhase`, `ai_branch_input: String`, `ai_branch_cursor: usize`, `ai_branch_suggestions: Vec<String>`, `ai_branch_selected: usize`, `ai_branch_error: Option<String>`

### exhaustive match更新（コンパイル通過）

- [ ] **T006** [US6] T004の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `wizard_title()` 関数に `WizardStep::AIBranchSuggest` ケースを追加（タイトル: "AI Branch Suggest"）
- [ ] **T007** [US6] T004の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `wizard_required_content_width()` 関数に `WizardStep::AIBranchSuggest` ケースを追加
- [ ] **T008** [US6] T004の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `current_step_item_count()` に `WizardStep::AIBranchSuggest` ケースを追加（フェーズにより0またはsuggestions.len()）
- [ ] **T009** [US6] T004の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `current_selection_index()` に `WizardStep::AIBranchSuggest` ケースを追加（ai_branch_selected）
- [ ] **T010** [US6] T004の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `set_selection_index()` に `WizardStep::AIBranchSuggest` ケースを追加
- [ ] **T011** [US6] T004の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `select_next()` に `WizardStep::AIBranchSuggest` ケースを追加（Selectフェーズ時のみ候補ナビゲーション）
- [ ] **T012** [US6] T004の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `select_prev()` に `WizardStep::AIBranchSuggest` ケースを追加
- [ ] **T013** [US6] T004の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の render dispatch（`render_wizard()` 内のmatch）に `WizardStep::AIBranchSuggest` ケースを追加（暫定的に空のrender関数を呼ぶ）

### コンパイル検証

- [ ] **T014** [US6] T006-T013の後に `cargo build --release` と `cargo test` が成功することを検証。既存テストが全て通過すること

## フェーズ2: US6 - AI無効時のAIBranchSuggestスキップ (優先度: P1)

**ストーリー**: AI設定が無効のユーザーは、従来通りAIBranchSuggestステップなしでブランチ作成ウィザードを進める

**価値**: 後方互換性の維持。既存ユーザーの体験を損なわない

### テスト

- [ ] **T015** [US6] T014の後に `crates/gwt-cli/src/tui/screens/wizard.rs` にAI無効時のnext_stepテストを追加: `ai_enabled=false` の状態で `IssueSelect` → `next_step()` → `BranchNameInput` に遷移すること
- [ ] **T016** [US6] T014の後に `crates/gwt-cli/src/tui/screens/wizard.rs` にAI有効時のnext_stepテストを追加: `ai_enabled=true` の状態で `IssueSelect` → `next_step()` → `AIBranchSuggest` に遷移すること

### ステップ遷移実装

- [ ] **T017** [US6] T014の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `next_step()` を更新: `WizardStep::IssueSelect` ケースで、Issue処理後に `self.ai_enabled` をチェックし、有効なら `AIBranchSuggest`（フェーズをInputにリセット）、無効なら `BranchNameInput` に遷移
- [ ] **T018** [US6] T017の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `next_step()` に `WizardStep::AIBranchSuggest` ケースを追加: 選択された候補からbranch_type/new_branch_nameを設定し `BranchNameInput` に遷移
- [ ] **T019** [US6] T017の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `prev_step()` に `WizardStep::AIBranchSuggest` ケースを追加: gh CLI有効なら `IssueSelect`、無効なら `BranchTypeSelect` に戻る
- [ ] **T020** [US6] T017の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `prev_step()` の `WizardStep::BranchNameInput` ケースを更新: `ai_enabled` が有効なら `AIBranchSuggest` に戻る

### ai_enabledフラグ設定

- [ ] **T021** [US6] T005の後に `crates/gwt-cli/src/tui/app.rs` の wizard open処理（`open_for_branch()` / `open_for_new_branch()` 呼び出し箇所）で `self.wizard.ai_enabled = self.active_ai_enabled()` を設定

### テスト実行

- [ ] **T022** [US6] T015-T021の後に `cargo test` と `cargo clippy --all-targets --all-features -- -D warnings` が成功することを検証

**✅ MVP1チェックポイント**: US6完了後、AI無効ユーザーの既存フローが維持され、AI有効時はAIBranchSuggestステップに遷移する（中身は未実装）

## フェーズ3: US5 - AIによるブランチ名候補の生成と選択 (優先度: P1)

**ストーリー**: AI設定が有効なユーザーがブランチの目的を入力すると、AIが3つの候補を提案し、選択した名前がBranchNameInputに事前入力される

**価値**: ブランチ命名規則に沿った一貫性のある名前を簡単に生成

### テスト

- [ ] **T023** [US5] T022の後に `crates/gwt-cli/src/tui/screens/wizard.rs` にAIレスポンスパースのユニットテストを追加: 正常JSON、不正JSON、空配列、3件未満のケース
- [ ] **T024** [US5] T022の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に候補選択後のbranch_type/new_branch_name設定テストを追加: `from_prefix` 連携で正しく分離されること

### 入力ハンドラ実装

- [ ] **T025** [US5] T022の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `insert_char()` に `WizardStep::AIBranchSuggest` ケースを追加: `AIBranchSuggestPhase::Input` 時に `ai_branch_input` に文字追加、`ai_branch_cursor` を更新
- [ ] **T026** [US5] T025の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `delete_char()` に `WizardStep::AIBranchSuggest` ケースを追加: `AIBranchSuggestPhase::Input` 時に `ai_branch_input` から文字削除

### AIリクエスト・レスポンス処理

- [ ] **T027** [US5] T022の後に `crates/gwt-cli/src/tui/screens/wizard.rs` にAIレスポンスパース関数 `parse_branch_suggestions(response: &str) -> Result<Vec<String>, String>` を追加: JSON抽出、`{"suggestions": [...]}` パース、各候補をsanitize
- [ ] **T028** [US5] T027の後に `crates/gwt-cli/src/tui/app.rs` に `AiBranchSuggestUpdate` structと `ai_branch_suggest_rx: Option<mpsc::Receiver<AiBranchSuggestUpdate>>` フィールドを追加
- [ ] **T029** [US5] T028の後に `crates/gwt-cli/src/tui/app.rs` にAIBranchSuggest用のEnterハンドラを追加: `AIBranchSuggestPhase::Input` 時にAI設定を取得し、`thread::spawn` + `mpsc::channel` でAIリクエストを送信、`ai_branch_phase` を `Loading` に遷移
- [ ] **T030** [US5] T029の後に `crates/gwt-cli/src/tui/app.rs` に `apply_ai_branch_suggest_updates()` メソッドを追加: `try_recv()` で結果を受信し、成功時は `ai_branch_suggestions` を設定して `Select` フェーズに遷移、エラー時は `Error` フェーズに遷移
- [ ] **T031** [US5] T030の後に `crates/gwt-cli/src/tui/app.rs` のメインイベントループで `apply_ai_branch_suggest_updates()` を呼び出す（`apply_ai_wizard_updates()` と同様の位置）

### 候補選択・確定処理

- [ ] **T032** [US5] T018の後に `crates/gwt-cli/src/tui/screens/wizard.rs` の `next_step()` の `WizardStep::AIBranchSuggest` ケースを実装: `Select` フェーズ時に選択候補を `from_prefix()` で分離し、`branch_type`/`new_branch_name`/`cursor` を設定して `BranchNameInput` に遷移
- [ ] **T033** [US5] T029の後に `crates/gwt-cli/src/tui/app.rs` のAIBranchSuggest Enter ハンドラに `Select` フェーズのケースを追加: `wizard.next_step()` を呼び出す

### レンダリング実装

- [ ] **T034** [US5] T025の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に `render_ai_branch_suggest_step()` 関数を追加: Input/Loading/Select/Error 4フェーズのUI描画。Inputフェーズ: "What is this branch for?" ラベル + テキスト入力 + カーソル。Loadingフェーズ: "Generating branch name suggestions..." テキスト。Selectフェーズ: 候補リスト（選択状態ハイライト）。Errorフェーズ: エラーメッセージ表示
- [ ] **T035** [US5] T034の後に `crates/gwt-cli/src/tui/screens/wizard.rs` のrender dispatch（T013で暫定追加したもの）を `render_ai_branch_suggest_step()` の呼び出しに置き換え

### フッター表示

- [ ] **T036** [US5] T034の後に `crates/gwt-cli/src/tui/screens/wizard.rs` のウィザードフッター表示を更新: AIBranchSuggestステップのフェーズに応じたフッター（Input: "[Enter] Generate  [Esc] Skip"、Loading: "[Esc] Cancel"、Select: "[Enter] Select  [Esc] Back  [Up/Down] Navigate"、Error: "[Enter] Manual input  [Esc] Retry"）

### テスト実行

- [ ] **T037** [US5] T023-T036の後に `cargo test` と `cargo clippy --all-targets --all-features -- -D warnings` が成功することを検証

**✅ MVP2チェックポイント**: US5完了後、AI有効時にブランチ名候補の生成・選択・事前入力が機能する

## フェーズ4: US7 - AIBranchSuggestのスキップとフォールバック (優先度: P2)

**ストーリー**: AI設定が有効でもEscで手動入力にフォールバックでき、APIエラー時もフローが中断されない

**価値**: AIに依存したくない場合や障害時の代替手段

### テスト

- [ ] **T038** [US7] T037の後に `crates/gwt-cli/src/tui/screens/wizard.rs` にEscスキップのテストを追加: Input フェーズでEsc → BranchNameInput遷移（空のまま）、Select フェーズでEsc → Inputフェーズ戻り
- [ ] **T039** [US7] T037の後に `crates/gwt-cli/src/tui/screens/wizard.rs` に空入力でEnterのテストを追加: 空テキストでEnter → リクエスト送信なし

### Escハンドリング

- [ ] **T040** [US7] T037の後に `crates/gwt-cli/src/tui/app.rs` のAIBranchSuggest用Escハンドラを追加: `Input` フェーズ → `BranchNameInput` にスキップ（`new_branch_name` 空のまま）、`Loading` フェーズ → `ai_branch_suggest_rx = None` でキャンセルし `Input` フェーズに戻る、`Select` フェーズ → `Input` フェーズに戻る、`Error` フェーズ → `Input` フェーズに戻る

### エラーフォールバック

- [ ] **T041** [US7] T030の後に `crates/gwt-cli/src/tui/app.rs` の `apply_ai_branch_suggest_updates()` のエラーハンドリングを確認: `ai_branch_error` にエラーメッセージを設定し、`ai_branch_phase` を `Error` に遷移
- [ ] **T042** [US7] T040の後に `crates/gwt-cli/src/tui/app.rs` のAIBranchSuggest Enter ハンドラに `Error` フェーズのケースを追加: `BranchNameInput` にスキップ（`new_branch_name` 空のまま）

### 空入力バリデーション

- [ ] **T043** [US7] T029の後に `crates/gwt-cli/src/tui/app.rs` のAIBranchSuggest Enter ハンドラの `Input` フェーズで空テキストチェックを追加: `ai_branch_input.trim().is_empty()` の場合はリクエスト送信せず何もしない

### テスト実行

- [ ] **T044** [US7] T038-T043の後に `cargo test` と `cargo clippy --all-targets --all-features -- -D warnings` が成功することを検証

**✅ 完全な機能**: US7完了後、スキップ・キャンセル・エラーフォールバックが全て機能する

## フェーズ5: 統合と仕上げ

**目的**: 全体の品質検証とコミット

### 品質検証

- [ ] **T045** [統合] T044の後に `cargo build --release` が成功することを検証
- [ ] **T046** [統合] T045の後に `cargo test` の全テスト通過を検証
- [ ] **T047** [統合] T046の後に `cargo clippy --all-targets --all-features -- -D warnings` がクリーンであることを検証
- [ ] **T048** [統合] T047の後に `cargo fmt -- --check` がクリーンであることを検証

## タスク凡例

**優先度**:

- **P1**: 最も重要 - MVP1/MVP2に必要（US5, US6）
- **P2**: 重要 - 完全な機能に必要（US7）

**依存関係**:

- **[P]**: 並列実行可能（異なるファイル/セクション、依存関係なし）
- 依存なし: 前のタスクの完了後に実行

**ストーリータグ**:

- **[US5]**: AIによるブランチ名候補の生成と選択
- **[US6]**: AI無効時のAIBranchSuggestスキップ
- **[US7]**: AIBranchSuggestのスキップとフォールバック
- **[統合]**: 複数ストーリーにまたがる品質検証

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化
