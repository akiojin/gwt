# タスク: ブランチアクション画面のWSL入力互換性改善

**仕様ID**: `SPEC-d2f4762a`
**ポリシー**: CLAUDE.md の TDD ルールに基づき、必ず RED→GREEN→リグレッションチェックの順に進める。

## フェーズ0: ブランチ一覧アイコンの絵文字復帰 (優先度: P1)

**ストーリー**: ブランチ一覧の選択/Worktree/安全アイコンを絵文字に戻し、カーソル記号を表示しない。

**価値**: 以前の視認性とユーザーの記憶に合わせ、分かりやすい一覧表示を復元する。

### 仕様更新

- [x] **T901** [P] [共通] `specs/SPEC-d2f4762a/spec.md` のアイコン仕様とカーソル非表示要件を更新
- [x] **T902** [P] [共通] `specs/SPEC-d2f4762a/plan.md` のアイコン方針を絵文字に更新
- [x] **T903** [P] [共通] `specs/SPEC-d27be71b/spec.md` の意思決定ログを更新（絵文字復帰を反映）
- [x] **T904** [P] [共通] `CLAUDE.md` のアイコン方針を更新（ブランチ一覧は絵文字を許容）

### テスト（TDD）

- [x] **T911** [US4] `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx` に絵文字アイコンとカーソル非表示の表示テストを追加

### 実装

- [x] **T921** [US4] `src/cli/ui/screens/solid/BranchListScreen.tsx` の選択/Worktree/安全アイコンを絵文字に戻す
- [x] **T922** [US4] `src/cli/ui/screens/solid/BranchListScreen.tsx` のカーソル記号を非表示のまま維持する

## フェーズ1: セットアップ（共有インフラストラクチャ）

### セットアップタスク

- [x] **T001** [P] [共通] `specs/SPEC-d2f4762a/spec.md` にWSL/Windowsの分割エスケープ対応要件と受け入れシナリオを追記
- [x] **T002** [P] [共通] `specs/SPEC-d2f4762a/plan.md` のハイレベルToDoへ入力互換性の追記

## フェーズ2: ユーザーストーリー0 - ブランチ選択の基本操作（既存機能） (優先度: P0)

**依存関係**: US0 のみ（他ストーリー依存なし）

**ストーリー**: ブランチ選択後のアクション画面でも、矢印キーで選択移動でき、`Esc`は戻る操作として機能する。

**価値**: WSL/Windows環境でも誤って前画面に戻らずにアクション選択ができる。

### テスト（TDD）

- [x] **T101** [US0] `src/cli/ui/screens/__tests__/BranchActionSelectorScreen.test.tsx` に遅延した分割矢印シーケンスでも`Esc`扱いにならないテストを追加

### 実装

- [x] **T102** [US0] `src/cli/ui/hooks/useAppInput.ts` のエスケープシーケンス待機時間を調整し、WSL/Windowsの遅延を許容する
- [x] **T103** [US0] `src/cli/ui/screens/__tests__/BranchActionSelectorScreen.test.tsx` の`Esc`タイムアウト期待値を新しい待機時間に合わせる
- [x] **T104** [US0] `tests/unit/index.entrypoint.test.ts` に相対パス実行でもエントリ判定が通ることを確認するテストを追加
- [x] **T105** [US0] `src/index.ts` のエントリポイント判定を `fileURLToPath` + `path.resolve` で正規化する
- [x] **T106** [P] [共通] `specs/SPEC-d2f4762a/spec.md` にリモート取得の停止時もUIが継続する要件とシナリオを追記
- [x] **T107** [P] [共通] `specs/SPEC-d2f4762a/plan.md` に非ブロッキングフェッチの実装方針を追記
- [x] **T108** [US0] `src/cli/ui/__tests__/hooks/useGitData.nonblocking.test.tsx` にフェッチが解決しなくてもローディングが解除されるテストを追加
- [x] **T109** [US0] `tests/unit/git.fetchAllRemotes.test.ts` に`fetchAllRemotes`がタイムアウト/非対話設定を渡すテストを追加
- [x] **T110** [US0] `src/git.ts` と `src/cli/ui/hooks/useGitData.ts` を更新し、フェッチのタイムアウトと非ブロッキングを実装
- [x] **T111** [US0] `src/cli/ui/__tests__/components/screens/BranchListScreen.test.tsx` に選択中ブランチのフルパス表示（`Branch: ...`）と空表示時の`Branch: (none)`のテストを追加
- [x] **T112** [US0] `src/cli/ui/components/screens/BranchListScreen.tsx` にフッターヘルプ直上のフルパス表示と固定行数調整を実装

## フェーズ3: 統合とポリッシュ

- [x] **T201** [統合] `package.json` に従い `bun run format:check` / `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` / `bun run lint` を実行し、失敗があれば修正

## フェーズ4: ユーザーストーリー6 - Worktree作成時のstaleディレクトリ自動回復

**ストーリー**: Worktree作成対象パスがstale状態で残っていても、自動回復して作成を完了する。

**価値**: 手動削除の手戻りをなくし、ブランチ選択から作業開始までを途切れさせない。

### テスト（TDD）

- [x] **T301** [US6] `tests/integration/branch-creation.test.ts` にstaleディレクトリを検出して削除→再作成できるテストを追加
- [x] **T302** [US6] `tests/integration/branch-creation.test.ts` にstale判定できない既存ディレクトリは削除せずエラーになるテストを追加

### 実装

- [x] **T303** [US6] `src/worktree.ts` にstale判定・削除処理を追加し、`createWorktree`の前処理として実行
- [x] **T304** [US6] `src/worktree.ts` に判定不能な既存ディレクトリ向けの明確なエラーメッセージを追加

## フェーズ5: ユーザーストーリー8 - ブランチ選択後のウィザードポップアップ (優先度: P0)

**依存関係**: なし

**ストーリー**: ブランチ選択後、ブランチ一覧画面の上にウィザードポップアップが表示され、7ステップの設定選択フローを経てAIツールを起動する。

**価値**: 画面遷移ではなくレイヤー表示により、コンテキストを維持しながら設定を進められる。

### テスト（TDD）

- [x] **T401** [US8] `src/cli/ui/__tests__/solid/components/WizardPopup.test.tsx` にウィザードポップアップの表示/非表示テストを追加
- [x] **T402** [US8] `src/cli/ui/__tests__/solid/components/WizardPopup.test.tsx` に背景オーバーレイ（半透過）の表示テストを追加
- [x] **T403** [US8] `src/cli/ui/__tests__/solid/components/WizardPopup.test.tsx` にステップ表示テストを追加（Escapeキーテストはhook制約により視覚テストに変更）
- [x] **T404** [US8] `src/cli/ui/__tests__/solid/components/WizardPopup.test.tsx` にステップ表示・枠線テストを追加
- [x] **T405** [US8] `src/cli/ui/__tests__/solid/components/WizardSteps.test.tsx` にブランチタイプ選択ステップのテストを追加
- [x] **T406** [US8] `src/cli/ui/__tests__/solid/components/WizardSteps.test.tsx` にブランチ名入力ステップのテストを追加
- [x] **T407** [US8] `src/cli/ui/__tests__/solid/components/WizardSteps.test.tsx` にコーディングエージェント選択ステップのテストを追加
- [x] **T408** [US8] `src/cli/ui/__tests__/solid/components/WizardSteps.test.tsx` にモデル選択ステップのテストを追加
- [x] **T409** [US8] `src/cli/ui/__tests__/solid/components/WizardSteps.test.tsx` に推論レベル選択ステップ（Codexのみ）のテストを追加
- [x] **T410** [US8] `src/cli/ui/__tests__/solid/components/WizardSteps.test.tsx` に実行モード選択ステップのテストを追加
- [x] **T411** [US8] `src/cli/ui/__tests__/solid/components/WizardSteps.test.tsx` に権限スキップ確認ステップのテストを追加

### 実装

- [x] **T412** [US8] `src/cli/ui/components/solid/WizardPopup.tsx` にウィザードポップアップコンポーネントを作成（z-index、オーバーレイ）
- [x] **T413** [US8] `src/cli/ui/components/solid/WizardSteps.tsx` に各ステップコンポーネントを作成
- [x] **T414** [US8] `src/cli/ui/App.solid.tsx` にウィザードポップアップの統合（BranchListScreenからの起動）

## フェーズ6: ユーザーストーリー9 - 前回履歴からのクイック選択ポップアップ (優先度: P1)

**依存関係**: US8（ウィザードポップアップUI）

**ストーリー**: ブランチ選択時に履歴がある場合、クイック選択画面を表示し、エージェントごとの「Resume/Start new」を選べる。

**価値**: 毎回ツールとモデルを選択する手間を削減し、高速に再開できる。

### テスト（TDD）

- [x] **T501** [US9] `src/cli/ui/__tests__/solid/components/QuickStartStep.test.tsx` にクイック選択画面の表示テストを追加
- [x] **T502** [US9] `src/cli/ui/__tests__/solid/components/QuickStartStep.test.tsx` にヘルプテキスト表示テストを追加
- [x] **T503** [US9] `src/cli/ui/__tests__/solid/components/QuickStartStep.test.tsx` に「Resume with previous settings」選択時の動作テストを追加
- [x] **T504** [US9] `src/cli/ui/__tests__/solid/components/QuickStartStep.test.tsx` に「Start new with previous settings」選択時の動作テストを追加
- [x] **T505** [US9] `src/cli/ui/__tests__/solid/components/QuickStartStep.test.tsx` に「Choose different settings...」選択時の動作テストを追加
- [x] **T506** [US9] `src/cli/ui/__tests__/solid/components/QuickStartStep.test.tsx` に履歴がない場合のスキップテストを追加

### 実装

- [x] **T507** [US9] `src/cli/ui/components/solid/QuickStartStep.tsx` にクイック選択ステップコンポーネントを作成
- [x] **T508** [US9] `src/cli/ui/components/solid/WizardController.tsx` にクイック選択ステップの統合（履歴有無による分岐）
