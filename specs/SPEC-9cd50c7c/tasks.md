# タスクリスト: AI自動ブランチ命名モード

## ストーリー間の依存関係

- US1（AI自動命名）, US2（Direct入力）: 並列不可（同一ファイル AgentLaunchForm.svelte を変更）
- US3（永続化）: US1/US2 完了後（セグメンテッドボタンが存在する前提）
- US4（フォールバック）: US1 完了後（AI Suggestモードの存在が前提）
- US5（fromIssue分離）: US1 完了後（セグメンテッドボタンがmanualタブ限定であることの確認）

## Phase 1: セットアップ・基盤

- [x] T001 [P] [US1] テスト: parse_branch_suggestion() 正常系（1つのJSON→ブランチ名返却）のテスト作成（RED確認） `crates/gwt-core/src/ai/branch_suggest.rs`
- [x] T002 [P] [US1] テスト: parse_branch_suggestion() 異常系（不正prefix、空suffix、パース不能）のテスト作成（RED確認） `crates/gwt-core/src/ai/branch_suggest.rs`
- [x] T003 [US1] 実装: BRANCH_SUGGEST_SYSTEM_PROMPT を1つ生成に変更 `crates/gwt-core/src/ai/branch_suggest.rs`
- [x] T004 [US1] 実装: BranchSuggestionsResponse を suggestion: String に変更 `crates/gwt-core/src/ai/branch_suggest.rs`
- [x] T005 [US1] 実装: parse_branch_suggestions() → parse_branch_suggestion() にリネーム・改修（1つ検証、戻り値 Result<String, AIError>） `crates/gwt-core/src/ai/branch_suggest.rs`
- [x] T006 [US1] 実装: suggest_branch_names() → suggest_branch_name() にリネーム（戻り値 Result<String, AIError>） `crates/gwt-core/src/ai/branch_suggest.rs`
- [x] T007 [US1] 検証: T001, T002 のテストがGREENになることを確認 `cargo test -p gwt-core`
- [x] T008 [US1] 実装: BranchSuggestResult の suggestions → suggestion に変更 `crates/gwt-tauri/src/commands/branch_suggest.rs`
- [x] T009 [US1] 実装: suggest_branch_names コマンド → suggest_branch_name にリネーム、内部呼び出し更新 `crates/gwt-tauri/src/commands/branch_suggest.rs`
- [x] T010 [US1] 実装: Tauriコマンド登録を suggest_branch_name に更新 `crates/gwt-tauri/src/lib.rs`

## Phase 2: US1 - AI自動ブランチ命名でLaunch (P0)

- [x] T011 [US1] 実装: LaunchAgentRequest に ai_branch_description: Option<String> フィールド追加 `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T012 [US1] 実装: "create" ステップ内に AI生成フロー追加（ai_branch_description が Some の場合: AI呼び出し → ブランチ名生成 → worktree作成） `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T013 [US1] 実装: AI失敗時に [E2001] エラーコード付き StructuredError を返却 `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T014 [US1] 実装: "create" ステップの detail に "Generating branch name..." を report_launch_progress で送信 `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T015 [US1] 検証: cargo test -p gwt-tauri パス確認 `cargo test -p gwt-tauri`
- [x] T016 [P] [US1] 実装: BranchSuggestResult 型を suggestion: string に変更 `gwt-gui/src/lib/types.ts`
- [x] T017 [P] [US1] 実装: LaunchAgentRequest 型に aiBranchDescription?: string 追加 `gwt-gui/src/lib/types.ts`
- [x] T018 [US1] テスト: セグメンテッドボタン切替（Direct→Prefix+Suffix表示、AI Suggest→Description表示）のvitestテスト作成（RED確認） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T019 [US1] テスト: AI Suggest + Description空→Launch disabled、Description入力→Launch enabled のvitestテスト作成（RED確認） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T020 [US1] 実装: Suggestモーダル関連の状態変数削除（suggestOpen, suggestDescription, suggestLoading, suggestError, suggestSuggestions） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T021 [US1] 実装: Suggestモーダル関連の関数削除（openSuggestModal, closeSuggestModal, generateBranchSuggestions） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T022 [US1] 実装: SuggestモーダルHTML全体 + "Suggest..." ボタンを削除 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T023 [US1] 実装: 状態変数追加（branchNamingMode, aiDescription, aiConfigured, aiFallbackError） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T024 [US1] 実装: manualタブ内にセグメンテッドボタン（Direct / AI Suggest）を追加 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T025 [US1] 実装: AI Suggestモード表示（「Description」ラベル + 単行テキストフィールド、placeholder: "e.g. Add user authentication feature"） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T026 [US1] 実装: Directモード表示（従来のPrefix選択+Suffix入力、Suggest...ボタンなし） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T027 [US1] 実装: handleLaunch() 改修 — AI Suggestモード時に aiBranchDescription をセット、createBranch.name は空文字列 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T028 [US1] 実装: Launchボタンのdisabled条件更新（AI Suggest + aiDescription空 → disabled） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T029 [US1] 検証: T018, T019 のテストがGREENになることを確認 `cd gwt-gui && pnpm test`

## Phase 3: US2 - Direct入力でLaunch (P0)

- [x] T030 [US2] テスト: Directモードで従来のPrefix+Suffix UIが表示されるテスト作成（RED確認） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T031 [US2] 検証: T030 のテストがGREENになることを確認（US1実装でカバー済みの可能性あり） `cd gwt-gui && pnpm test`

## Phase 4: US3 - モード選択の永続化 (P1)

- [x] T032 [US3] テスト: branchNamingMode のlocalStorage保存・復元のvitestテスト作成（RED確認） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T033 [US3] 実装: LaunchDefaults 型に branchNamingMode: "direct" | "ai-suggest" 追加（デフォルト "ai-suggest"） `gwt-gui/src/lib/agentLaunchDefaults.ts`
- [x] T034 [US3] 実装: フォーム初期化時に loadLaunchDefaults() から branchNamingMode を復元 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T035 [US3] 実装: Launch時/モード切替時に saveLaunchDefaults() で branchNamingMode を保存 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T036 [US3] 検証: T032 のテストがGREENになることを確認 `cd gwt-gui && pnpm test`

## Phase 5: US4 - AI提案失敗時のフォールバック (P1) + US3 AI未設定 (P1)

- [x] T037 [US4] テスト: [E2001]エラー時にDirectモード切替+エラーバナー表示のvitestテスト作成（RED確認） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T038 [US4] テスト: エラーバナーがモード切替時に自動消去されるvitestテスト作成（RED確認） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T039 [US4] 実装: launch-finished イベントで [E2001] を検知し、branchNamingMode を "direct" に切替 + aiFallbackError をセット `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T040 [US4] 実装: フォーム上部にエラーバナー表示（aiFallbackError が非null時） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T041 [US4] 実装: branchNamingMode 変更時に aiFallbackError を null にクリア `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T042 [US3] テスト: AI未設定時にAI Suggestセグメントがdisabledになるvitestテスト作成（RED確認） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T043 [US3] 実装: フォーム呈示時にバックエンドへAI設定チェック（invoke("suggest_branch_name") の ai-not-configured パターンを利用） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T044 [US3] 実装: AI未設定時に AI Suggest セグメントを disabled + Directモード強制 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T045 [US4] 実装: Description入力値はモード切替時もクリアせず内部保持（FR-014） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T046 [US3,US4] 検証: T037, T038, T042 のテストがGREENになることを確認 `cd gwt-gui && pnpm test`

## Phase 6: US5 - fromIssueタブとの分離 (P2)

- [x] T047 [US5] テスト: fromIssueタブでセグメンテッドボタンが表示されないvitestテスト作成（RED確認） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T048 [US5] 検証: T047 のテストがGREENになることを確認（Phase 2でmanualタブ限定にしているためカバー済みの可能性あり） `cd gwt-gui && pnpm test`

## Phase 7: 仕上げ・横断

- [x] T049 [P] [共通] デッドコード確認: Suggestモーダル関連の未使用インポート・型定義・CSS削除 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T050 [P] [共通] cargo clippy --all-targets --all-features -- -D warnings パス確認 `cargo clippy --all-targets --all-features -- -D warnings`
- [x] T051 [P] [共通] cargo fmt パス確認 `cargo fmt --check`
- [x] T052 [P] [共通] svelte-check パス確認 `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`
- [x] T053 [共通] 全テスト最終確認: cargo test + cd gwt-gui && pnpm test
