# タスク: ブランチアクション画面のWSL入力互換性改善

**仕様ID**: `SPEC-d2f4762a`
**ポリシー**: CLAUDE.md の TDD ルールに基づき、必ず RED→GREEN→リグレッションチェックの順に進める。

## 追加作業: Rust版キーバインド整合 (2026-01-13)

- [x] **T9901** [P] [US0] `specs/SPEC-d2f4762a/spec.md` から `n` キー記述を削除し、ウィザード手順を整理
- [x] **T9902** [US0] `crates/gwt-cli/src/tui/app.rs` などで BranchList の `n` ショートカットを除外し、フッター/ヘルプ表示を更新

## 追加作業: ブランチ一覧の統計/最終更新表示削除 (2026-01-13)

- [x] **T9903** [P] [共通] `specs/SPEC-d2f4762a/spec.md` から統計情報/最終更新時刻の表示要件とシナリオを削除し、Mode(tab)行/ツール表示の記述を更新
- [x] **T9904** [P] [共通] `specs/SPEC-d2f4762a/plan.md` にMode(tab)行の表示方針（統計/Updated非表示）を追記
- [x] **T9905** [P] [共通] `specs/SPEC-f47db390/spec.md` のツール表示フォーマットを`ToolName@X.Y.Z`形式へ更新
- [x] **T9906** [Test] `crates/gwt-core/src/config/ts_session.rs` のツール表示フォーマットテストを更新（時刻表示の削除）
- [x] **T9907** [実装] `crates/gwt-cli/src/tui/app.rs` と `crates/gwt-cli/src/tui/screens/branch_list.rs` から統計/Updated表示を削除
- [x] **T9908** [実装] `crates/gwt-core/src/config/ts_session.rs` のツール表示フォーマットから時刻を削除し、関連コメントを更新

## フェーズ0: ブランチ一覧アイコンのASCII再整理 (優先度: P1)

**ストーリー**: ブランチ一覧の選択/Worktree/安全アイコンをASCII表記へ整理し、アイコン間にスペースを入れてカーソル記号は表示しない。

**価値**: 端末幅のズレを防ぎつつ、直感的な記号で一覧表示の視認性を維持する。

### 仕様更新

- [x] **T901** [P] [共通] `specs/SPEC-d2f4762a/spec.md` のアイコン仕様とカーソル非表示要件を更新
- [x] **T902** [P] [共通] `specs/SPEC-d2f4762a/plan.md` のアイコン方針をASCIIに更新
- [x] **T903** [P] [共通] `specs/SPEC-d27be71b/spec.md` の意思決定ログを更新（ASCII整理を反映）
- [x] **T904** [P] [共通] `CLAUDE.md` のアイコン方針をASCIIに戻す

### テスト（TDD）

- [x] **T911** [US4] `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx` にASCIIアイコンとカーソル非表示の表示テストを追加

### 実装

- [x] **T921** [US4] `src/cli/ui/screens/solid/BranchListScreen.tsx` の選択/Worktree/安全アイコンをASCIIに更新
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

## 追加作業ToDo (2026-01-09)
- [x] **T503** [US9] `src/cli/ui/__tests__/solid/components/QuickStartStep.test.tsx` に「Resume with previous settings」選択時の動作テストを追加
- [x] **T504** [US9] `src/cli/ui/__tests__/solid/components/QuickStartStep.test.tsx` に「Start new with previous settings」選択時の動作テストを追加
- [x] **T505** [US9] `src/cli/ui/__tests__/solid/components/QuickStartStep.test.tsx` に「Choose different settings...」選択時の動作テストを追加
- [x] **T506** [US9] `src/cli/ui/__tests__/solid/components/QuickStartStep.test.tsx` に履歴がない場合のスキップテストを追加

### 実装

- [x] **T507** [US9] `src/cli/ui/components/solid/QuickStartStep.tsx` にクイック選択ステップコンポーネントを作成
- [x] **T508** [US9] `src/cli/ui/components/solid/WizardController.tsx` にクイック選択ステップの統合（履歴有無による分岐）

## フェーズ11: ユーザーストーリー4 - unsafeブランチ選択の確認 (優先度: P1)

**ストーリー**: 安全ではないブランチを`space`でチェックしようとした場合、警告OK/Cancelを表示し、OKでチェック、Cancelで未選択を維持する。

**価値**: 誤削除リスクの高いブランチを選択する際に、意図確認を必須化できる。

### テスト（TDD）

- [x] **T961** [US4] `src/cli/ui/__tests__/solid/AppSolid.cleanup.test.tsx` にunsafeブランチ選択時の警告表示とOK/Cancel動作のテストを追加

### 実装

- [x] **T962** [US4] `src/cli/ui/App.solid.tsx` にunsafe選択時の警告OK/Cancel表示と選択確定/維持ロジックを追加
- [x] **T963** [US4] `src/cli/ui/screens/solid/BranchListScreen.tsx` に警告表示中の入力ロックを追加

## フェーズ12: ユーザーストーリー4 - 選択済みブランチの優先実行 (優先度: P1)

**ストーリー**: チェック済みブランチは安全判定に関係なくクリーンアップ/修復の対象に含める（リモートは除外、クリーンアップ時の現在ブランチは除外）。

**価値**: 意図的に選択したブランチを確実に処理でき、操作の予測性が向上する。

### テスト（TDD）

- [x] **T971** [US4] `src/cli/ui/__tests__/solid/AppSolid.cleanup.test.tsx` にunsafe/保護ブランチが選択されている場合でもクリーンアップ対象になるテストを追加
- [x] **T972** [US4] `src/cli/ui/__tests__/solid/AppSolid.cleanup.test.tsx` に現在ブランチが選択されている場合はクリーンアップ対象から除外されるテストを追加
- [x] **T973** [US4] `src/cli/ui/__tests__/solid/AppSolid.cleanup.test.tsx` に修復でアクセス可能なWorktreeも対象になるテストを追加

### 実装

- [x] **T981** [US4] `src/cli/ui/App.solid.tsx` のクリーンアップ選択ロジックを更新し、安全判定・保護ブランチによる除外を廃止する
- [x] **T982** [US4] `src/cli/ui/App.solid.tsx` の修復対象判定から `worktreeStatus === "inaccessible"` 条件を除外する

## フェーズ13: ユーザーストーリー4 - 安全アイコン凡例の表示 (優先度: P2)

**ストーリー**: Mode(tab)行の直下に安全アイコンの凡例行を表示し、未コミット/未プッシュ/未マージの意味を説明する。

**価値**: 安全アイコンの意味を即座に理解でき、誤操作の防止につながる。

### 仕様更新

- [x] **T991** [P] [共通] `specs/SPEC-d2f4762a/spec.md` に安全アイコン凡例行の要件とシナリオを追記
- [x] **T992** [P] [共通] `specs/SPEC-d2f4762a/plan.md` に凡例行の方針とToDoを追記

### テスト（TDD）

- [x] **T993** [US4] `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx` に凡例行の表示テストを追加

### 実装

- [x] **T994** [US4] `src/cli/ui/screens/solid/BranchListScreen.tsx` に凡例行を表示

## フェーズ14: ユーザーストーリー4 - unsafe警告ダイアログの反転範囲修正 (優先度: P2)

**ストーリー**: unsafe選択の警告ダイアログで OK/Cancel の反転表示が枠外に広がらないようにする。

**価値**: 画面の視認性を保ち、ダイアログの境界が明確になる。

### 仕様更新

- [x] **T995** [P] [共通] `specs/SPEC-d2f4762a/spec.md` に反転範囲の要件を追記
- [x] **T996** [P] [共通] `specs/SPEC-d2f4762a/plan.md` に反転範囲の方針を追記

### テスト（TDD）

- [x] **T997** [US4] `src/cli/ui/__tests__/solid/ConfirmScreen.test.tsx` に反転範囲がダイアログ幅に収まるテストを追加

## 追加作業ToDo (2026-01-10)
- [x] **T1100** [共通] `specs/SPEC-d2f4762a/spec.md` と `specs/SPEC-d2f4762a/plan.md` に安全判定中の選択確認とバージョン選択の自動遷移防止要件を追記
- [x] **T1101** [US4] 安全判定中のブランチ選択で警告が表示される実装（Rust版: `crates/gwt-cli/src/tui/app.rs` - `safe_to_cleanup.is_none()` で判定中を検出）
- [x] **T1102** [US4] 安全判定中の`space`選択時に警告を出し、OK/Cancelで選択を制御する実装（Rust版: `crates/gwt-cli/src/tui/app.rs` - `ConfirmState::unsafe_selection_warning`）
- [x] **T1103** [US10] バージョン選択ステップが自動遷移しない実装（Rust版: `crates/gwt-cli/src/tui/screens/wizard.rs` - 各ステップで明示的Enter必須）
- [x] **T1104** [US10] エージェント選択後のEnter伝播を抑止し、バージョン選択が必ず表示される実装（Rust版: `WizardConfirm` → `next_step()` の明示的呼び出し）

> **Note**: T1101-T1104はTypeScript版のタスクでしたが、Rust移行（2026-01-11）により同等機能がRustで実装されています。

### 実装

- [x] **T998** [US4] `src/cli/ui/screens/solid/ConfirmScreen.tsx` と `src/cli/ui/App.solid.tsx` を更新し、ダイアログ内幅で反転表示する

## フェーズ15: ユーザーストーリー4 - 凡例にSafe表示を追加 (優先度: P2)

**ストーリー**: 安全アイコンの凡例に `o Safe` を追加し、安全状態が即時に理解できるようにする。

**価値**: 警告アイコンだけでなく安全状態も明示され、一覧の理解が早くなる。

### 仕様更新

- [x] **T999** [P] [共通] `specs/SPEC-d2f4762a/spec.md` の凡例行を `o Safe` を含む内容に更新
- [x] **T1000** [P] [共通] `specs/SPEC-d2f4762a/plan.md` の凡例説明に Safe を追加

### テスト（TDD）

- [x] **T1001** [US4] `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx` の凡例表示テストを `o Safe` に更新

### 実装

- [x] **T1002** [US4] `src/cli/ui/screens/solid/BranchListScreen.tsx` の凡例行に `o Safe` を追加

## フェーズ16: ユーザーストーリー4 - unsafe確認Enterの伝搬抑止 (優先度: P2)

**ストーリー**: unsafe選択の警告ダイアログで Enter により OK/Cancel を確定した際、ブランチ一覧の Enter 選択が発火しないようにする。

**価値**: 意図しないブランチ選択/ウィザード起動を防止する。

### 仕様更新

- [x] **T1003** [P] [共通] `specs/SPEC-d2f4762a/spec.md` に Enter 伝搬抑止の受け入れ条件と要件を追記
- [x] **T1004** [P] [共通] `specs/SPEC-d2f4762a/plan.md` に伝搬抑止の方針を追記

### テスト（TDD）

- [x] **T1005** [US4] `src/cli/ui/__tests__/solid/AppSolid.cleanup.test.tsx` に Enter 確定時のブランチ選択が起きないテストを追加

### 実装

- [x] **T1006** [US4] `src/cli/ui/App.solid.tsx` の unsafe確認確定時にブランチ一覧入力を抑止

## フェーズ7: ユーザーストーリー7 - 選択中Worktreeフルパス表示 (優先度: P2)

**ストーリー**: ブランチ一覧のフッター直上に、選択中ブランチのWorktreeフルパスを表示する。Worktreeが存在しないが現在ブランチの場合は起動時の作業ディレクトリを表示し、ブランチ一覧が空の場合は`Worktree: (none)`を表示する。

**価値**: Worktreeの実体パスを正確に確認でき、誤操作や環境移行時の混乱を防げる。

### テスト（TDD）

- [x] **T601** [US7] `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx` に Worktree 行の表示（worktree path / (none) / workingDirectory フォールバック）テストを追加

### 実装

- [x] **T602** [US7] `src/cli/ui/screens/solid/BranchListScreen.tsx` のフッター表示を `Worktree:` に置き換え、表示ロジックを追加

## フェーズ8: フッターヘルプ整理とショートカット併記 (優先度: P2)

**ストーリー**: ブランチ一覧で画面内に表示される要素（Filter行/Mode/Profiles）にショートカットを併記し、フッターヘルプからは削除する。

**価値**: 重複した案内を減らし、視認性と理解度を高める。

### 仕様更新

- [x] **T701** [P] [共通] `specs/SPEC-d2f4762a/spec.md` にフッターヘルプ整理と`Filter(f)`/`Mode(tab)`/`Profile(p)`表記を反映
- [x] **T702** [P] [共通] `specs/SPEC-d2f4762a/plan.md` のハイレベルToDoにフッターヘルプ更新を追記

### テスト（TDD）

- [x] **T711** [US0] `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx` に`Filter(f)`/`Mode(tab)`/`Profile(p)`の表示とフッターヘルプからの除外を確認するテストを追加

### 実装

- [x] **T721** [US0] `src/cli/ui/screens/solid/BranchListScreen.tsx` のFilter/Mode表示ラベルとフッターアクションを更新
- [x] **T722** [US0] `src/cli/ui/components/solid/Header.tsx` のProfile表示ラベルにショートカットを併記

## フェーズ9: ユーザーストーリー8 - ウィザードポップアップのスクロール対応 (優先度: P1)

**ストーリー**: ウィザードポップアップの内容が表示領域を超える場合でも、ポップアップ内でスクロールできるようにして内容のはみ出しを防ぐ。

**価値**: 端末サイズが小さい環境でも、ウィザード内の全項目に到達できる。

### テスト（TDD）

- [x] **T731** [US8] `src/cli/ui/__tests__/solid/components/WizardPopup.test.tsx` にポップアップ内スクロールの表示テストを追加
- [x] **T733** [US8] `src/cli/ui/__tests__/solid/components/WizardPopup.test.tsx` に上下キーでのスクロールテストを追加

### 実装

- [x] **T732** [US8] `src/cli/ui/components/solid/WizardPopup.tsx` にスクロールコンテナを追加し、内容のはみ出しを防止
- [x] **T734** [US8] `src/cli/ui/components/solid/WizardPopup.tsx` と `src/cli/ui/components/solid/WizardSteps.tsx` で上下キーによるスクロールを実装

## フェーズ10: ユーザーストーリー4 - 安全判定と表示の更新 (優先度: P1)

**ストーリー**: upstream の有無とマージ状態を安全判定に反映し、未コミットは赤色の安全アイコン`!`、未プッシュは黄色`!`、未マージは黄色`*`で警告し、判定中はスピナーを表示する。

**価値**: 安全条件の誤認を防ぎ、削除判断の誤りを減らす。

### 仕様更新

- [x] **T801** [P] [共通] `specs/SPEC-d2f4762a/spec.md` の安全判定ルールと色指定を更新
- [x] **T802** [P] [共通] `specs/SPEC-d2f4762a/plan.md` の安全判定方針を更新
- [x] **T803** [P] [共通] `specs/SPEC-d2f4762a/spec.md` に安全判定スピナー/リモートブランチ空白の要件を追記
- [x] **T804** [P] [共通] `specs/SPEC-d2f4762a/plan.md` に安全判定スピナー/リモートブランチ空白の方針を追記
- [x] **T805** [P] [共通] `specs/SPEC-d2f4762a/spec.md` の安全時アイコンを緑色`o`に更新
- [x] **T806** [P] [共通] `specs/SPEC-d2f4762a/plan.md` の安全時アイコン方針に緑色`o`を追記
- [x] **T807** [P] [共通] `specs/SPEC-d27be71b/spec.md` の意思決定ログを安全時`o`に更新
- [x] **T816** [P] [共通] `specs/SPEC-d2f4762a/spec.md` の安全判定スピナーをブランチ単位で順次更新する要件を追記
- [x] **T817** [P] [共通] `specs/SPEC-d2f4762a/plan.md` の安全判定スピナー方針にブランチ単位更新を追記
- [x] **T826** [P] [共通] `specs/SPEC-d2f4762a/spec.md` の安全/Worktreeアイコンの明るい緑表示を追記
- [x] **T827** [P] [共通] `specs/SPEC-d2f4762a/plan.md` の明るい緑表示方針を更新

### テスト（TDD）

- [x] **T811** [US4] `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx` に未コミット/未プッシュの`!`と未マージの`*`を確認するテストを追加
- [x] **T812** [US4] `src/cli/ui/__tests__/solid/AppSolid.cleanup.test.tsx` にupstream未設定時の安全判定除外を確認するテストを追加
- [x] **T813** [US4] `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx` に安全判定中のスピナーとリモートブランチ空白を確認するテストを追加
- [x] **T814** [US4] `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx` の安全時`o`表示テストを更新
- [x] **T815** [US4] `src/cli/ui/__tests__/solid/AppSolid.cleanup.test.tsx` の安全時`o`表示テストを更新
- [x] **T818** [US4] `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx` に安全判定スピナーのブランチ単位更新テストを追加
- [x] **T819** [US4] `src/cli/ui/__tests__/solid/AppSolid.cleanup.test.tsx` に安全判定の逐次更新を確認するテストを追加
- [x] **T820** [US4] 色変更に伴う表示確認（自動テスト追加不要の確認）

### 実装

- [x] **T821** [US4] `src/cli/ui/App.solid.tsx` にupstream/マージ/未コミット・未プッシュの安全判定反映ロジックを追加
- [x] **T822** [US4] `src/cli/ui/screens/solid/BranchListScreen.tsx` の安全アイコン色分けを更新
- [x] **T823** [US4] `src/cli/ui/screens/solid/BranchListScreen.tsx` に安全判定スピナー/リモートブランチ空白の表示を追加
- [x] **T824** [US4] `src/cli/ui/screens/solid/BranchListScreen.tsx` の安全時`o`表示を追加
- [x] **T825** [US4] `src/cli/ui/App.solid.tsx` と `src/worktree.ts` に安全判定スピナーのブランチ単位更新を実装
- [x] **T828** [US4] `src/cli/ui/screens/solid/BranchListScreen.tsx` の安全/Worktree明るい緑表示を更新

## フェーズ11: ユーザーストーリー11 - 大量ブランチでも滑らかな操作 (優先度: P1)

**ストーリー**: 1000件以上のブランチでも、表示・スクロール・入力が滑らかに行えるようにし、再計算はフィルター/モード変更時のみに限定する。

**価値**: 大規模リポジトリでもUIが止まらず、日常運用のストレスを解消する。

### 仕様更新

- [x] **T1101** [P] [共通] `specs/SPEC-d2f4762a/spec.md` に大量ブランチ対応の要件とシナリオを追記
- [x] **T1102** [P] [共通] `specs/SPEC-d2f4762a/plan.md` に大量ブランチ対応の方針を追記

### テスト（TDD）

- [x] **T1103** [US11] `crates/gwt-cli/src/tui/screens/branch_list.rs` にフィルター/モード変更時のみ再計算されることを確認するテストを追加
- [x] **T1104** [US11] `crates/gwt-cli/src/tui/screens/branch_list.rs` に可視範囲描画が維持されることを確認するテストを追加

### 実装

- [x] **T1105** [US11] `crates/gwt-cli/src/tui/screens/branch_list.rs` にフィルター/モード変更時のみの再計算キャッシュを実装
- [x] **T1106** [US11] `crates/gwt-cli/src/tui/screens/branch_list.rs` の描画経路で可視範囲のみ描画する処理を明示化し、ローディング表示の維持を実装
