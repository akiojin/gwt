# 機能仕様: GUI Worktree Summary 6タブ + Quick Launchヘッダー再編（Issue #1097）

**仕様ID**: `SPEC-7c0444a8`
**作成日**: 2026-02-17
**更新日**: 2026-02-17
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**:

- `SPEC-3a1b7c2d`（Worktree Summary の AI Summary 振る舞い）
- `SPEC-735cbc5d`（Worktree Summary の Git セクション）
- `SPEC-d6949f99`（Session Summary の PR/Workflow 表示責務）

**入力**: ユーザー説明: "Issue #1097: Quick Startタブを廃止し、Launch Agent左に Quick Launch（Continue/New）を配置。Summary/Git/Issue/PR/Workflow/Docker の責務を維持する。"

## 背景

- 現在の Worktree Summary は、Quick Start・Summary・PR/Workflow・Docker が混在し、目的別アクセスが分かりにくい。
- ブランチに関連しない Issue 一覧へのフォールバック表示は、作業対象 Issue の追跡性を下げる。
- タブごとの失敗分離が不十分な場合、1つの取得失敗で全体の利用性が低下する。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - 固定6タブとヘッダー起動導線で情報へ到達できる (優先度: P0)

開発者として、Worktree Summary で必要な情報に最短でアクセスしたいので、表示構成を固定6タブに統一し、Quick起動はヘッダー導線で即時実行したい。

**独立したテスト**: Worktree Summary 表示時に 6 タブが固定順で表示され、ヘッダーに `Continue/New/Launch Agent...` が並び、各タブが独立して描画されることを UI テストで確認できる。

**受け入れシナリオ**:

1. **前提条件** Worktree Summary パネルを表示、**操作** タブ列とヘッダーを確認、**期待結果** `Summary / Git / Issue / PR / Workflow / Docker` の順で常時表示され、ヘッダー右側に `Continue / New / Launch Agent...` が表示される。
2. **前提条件** いずれかのタブでデータ取得に失敗、**操作** 別タブへ切り替え、**期待結果** 失敗タブは空状態またはエラー表示に留まり、他タブは継続して利用できる。

---

### ユーザーストーリー 2 - ブランチ関連 Issue と PR/Workflow を正しく把握できる (優先度: P0)

開発者として、現在ブランチの作業コンテキストだけを見たいので、Issue/PR/Workflow をブランチ関連情報に限定して表示したい。

**独立したテスト**: ブランチ名に `issue-<number>` が含まれる場合は該当 Issue のみ表示され、PR は優先順位に従って 1 件表示され、Workflow は PR 有無で表示が切り替わることを確認できる。

**受け入れシナリオ**:

1. **前提条件** ブランチ名が `feature/issue-1097`、**操作** Issue タブを開く、**期待結果** `#1097` のみ表示され、open issues 一覧へのフォールバックは行われない。
2. **前提条件** ブランチ名に `issue-<number>` が含まれない、**操作** Issue タブを開く、**期待結果** 対応 Issue なしの空状態メッセージが表示される。
3. **前提条件** ブランチに open PR が存在する、**操作** PR タブを開く、**期待結果** open PR が表示される。
4. **前提条件** open PR がないが closed/merged PR は存在する、**操作** PR タブを開く、**期待結果** 最新の closed/merged PR が表示される。
5. **前提条件** PR が存在しない、**操作** Workflow タブを開く、**期待結果** PR が必要である旨の空状態メッセージが表示される。

---

### ユーザーストーリー 3 - Launch導線/Summary/Docker の責務分離を保ったまま利用できる (優先度: P1)

開発者として、起動導線・AI 要約・Docker 状態を混同せずに確認したいので、ヘッダー導線と各タブの表示責務を明確に分離したい。

**独立したテスト**: ヘッダーの Continue/New 挙動が維持され、Summary には AI 要約のみ表示され、Docker タブに現在状態と履歴が併記されることを確認できる。

**受け入れシナリオ**:

1. **前提条件** Quick Start 履歴が存在する、**操作** ヘッダーの Continue/New を実行、**期待結果** 既存と同じ起動挙動が維持される。
2. **前提条件** AI 要約が存在する、**操作** Summary タブを開く、**期待結果** AI 要約 Markdown のみ表示され、Quick Start 要素は表示されない。
3. **前提条件** Docker 検出結果と Quick Start 履歴が存在する、**操作** Docker タブを開く、**期待結果** 現在の `detect_docker_context` 結果と履歴上の Docker 設定が同一画面で確認できる。

## エッジケース

- ブランチ名に複数の数字を含む場合でも、`issue-<number>` パターンに一致する番号のみを関連 Issue として扱う。
- Issue/PR/Workflow/Docker の一部取得に失敗しても、タブ列と他タブ表示は維持する。
- PR は存在するが workflow run が 0 件の場合、Workflow タブは空状態を表示する。
- Quick Start 履歴が空の場合、ヘッダーの Continue/New は無効化され、`Launch Agent...` は継続利用できる。
- リモートブランチ名が `origin/*` 以外（例: `upstream/*`）でも、PR検索時は head ref 名（`feature/*`）へ正規化して照合する。
- ブランチ切替中に旧ブランチの非同期取得が失敗しても、現在ブランチの PR/Workflow 状態を上書きしない。

## 要件 *(必須)*

### 機能要件

- **FR-001**: Worktree Summary は 6 タブを固定順 (`Summary`, `Git`, `Issue`, `PR`, `Workflow`, `Docker`) で常時表示しなければならない。
- **FR-002**: ヘッダー右側に `Continue` / `New` / `Launch Agent...` を表示し、`Continue/New` は既存 Quick Start 履歴に基づく起動挙動を維持しなければならない。
- **FR-003**: `Summary` タブは既存の AI 要約 Markdown のみを表示し、Quick Start 要素を含めてはならない。
- **FR-004**: `Git` タブは既存 `GitSection`（Changes/Commits/Stash）を表示しなければならない。
- **FR-005**: `Issue` タブはブランチ名の `issue-<number>` から解釈した関連 Issue のみ表示しなければならない。
- **FR-006**: `Issue` タブは関連 Issue が存在しない場合、空状態メッセージを表示しなければならない。
- **FR-007**: `Issue` タブは open issues 一覧へのフォールバック表示をしてはならない。
- **FR-008**: `PR` タブは現在ブランチに紐づく PR を 1 件表示し、open を優先し、なければ最新の closed/merged を表示しなければならない。
- **FR-009**: `PR` タブは候補 PR が存在しない場合、空状態メッセージを表示しなければならない。
- **FR-010**: `Workflow` タブは `PR` タブで選定された PR に紐づく checks/workflow 状態を表示しなければならない。
- **FR-011**: `Workflow` タブは PR が存在しない場合、空状態メッセージを表示しなければならない。
- **FR-012**: `Docker` タブは現在の `detect_docker_context` 結果を表示しなければならない。
- **FR-013**: `Docker` タブは Quick Start 履歴由来の Docker 設定（runtime/service/build/recreate/keep 等）を併記表示しなければならない。
- **FR-014**: 各タブはデータ取得失敗時にタブ単位のエラー/空状態を表示し、他タブの描画・操作に影響を与えてはならない。
- **FR-015**: Worktree Summary のタブ名称は UI 上で英語表示しなければならない。
- **FR-016**: `fetch_latest_branch_pr` は `origin/*` を含む既知remote接頭辞付きブランチ名を正規化し、PR head ref 解決に利用しなければならない。
- **FR-017**: Worktree Summary の PR取得失敗ハンドリングは、要求時の branch key と現在 key が一致する場合のみエラー状態を反映しなければならない。

### 非機能要件

- **NFR-001**: タブ切り替え時の UI 応答は既存 Session Summary 体感を劣化させない（不要な全体再描画を避ける）。
- **NFR-002**: 取得失敗時のメッセージはユーザーが復旧条件（例: PR が未作成）を判断できる情報量を持つ。
- **NFR-003**: 既存の Summary 生成・Quick Launch 起動・Git 表示の回帰をユニットテストで検出可能であること。

## 制約と仮定

- Worktree Summary の実装は既存の Tauri command と GUI 型定義を再利用し、互換を壊さない。
- ブランチ関連 Issue 判定はブランチ名規約（`issue-<number>`）に依存する。
- PR/Workflow 情報は GitHub CLI 連携の取得可能性に依存し、取得不能時は空状態/エラー表示で扱う。

## 成功基準 *(必須)*

- **SC-001**: Worktree Summary 表示時に 6 タブが固定順で常時表示され、ヘッダーに `Continue/New/Launch Agent...` が表示されることを UI テストで確認できる。
- **SC-002**: `Summary` タブに Quick Start 要素が表示されず、AI 要約のみ表示されることを UI テストで確認できる。
- **SC-003**: `Issue` タブが `issue-<number>` に一致する関連 Issue のみ表示し、非一致時は空状態となることをテストで確認できる。
- **SC-004**: `PR` / `Workflow` / `Docker` タブがデータ有無に応じた表示（実データまたは空状態）を行い、全体 UI が継続動作することをテストで確認できる。
- **SC-005**: ブランチ切替後に旧ブランチの `fetch_latest_branch_pr` エラーが返っても、現在ブランチで `No PR` 表示を維持し誤エラー表示しないことをテストで確認できる。
