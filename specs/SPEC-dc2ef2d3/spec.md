# 機能仕様: Worktree詳細ビューでCLAUDE.md/AGENTS.md/GEMINI.mdを確認・修正し編集起動

**仕様ID**: `SPEC-dc2ef2d3`  
**作成日**: 2026-02-27  
**更新日**: 2026-02-27  
**ステータス**: ドラフト  
**カテゴリ**: GUI  
**依存仕様**: `SPEC-7c0444a8`（Worktree Summary）  
**入力**: ユーザー説明: "CLAUDE.mdやAGENTS.mdの内容を確認・修正する機能を実装したい。Windows(PowerShell/cmd)でも編集起動したい。"

## 背景

- Worktree詳細ビューから、エージェント向け指示ファイル（`CLAUDE.md`/`AGENTS.md`/`GEMINI.md`）を即時に整備したい。
- `vi` 前提の編集起動は WSL では有効だが、Windows の PowerShell/cmd では環境差で失敗しやすい。
- ドキュメント整合（`@CLAUDE.md` 参照）を手作業に依存すると、起動時の規約適用漏れが起きる。

## ユーザーシナリオとテスト

### ユーザーストーリー 1 - Worktree詳細で一括整備して編集開始したい (優先度: P0)

ユーザーとして、選択中ブランチの Worktree で `CLAUDE.md` / `AGENTS.md` / `GEMINI.md` を一度に検査・修正し、すぐ編集を開始したい。

**独立したテスト**: Worktree詳細ビューのボタン押下だけで3ファイル整備と編集起動が完了すること。

**受け入れシナリオ**:

1. **前提条件** 選択ブランチの worktree に3ファイルが無い、**操作** ボタン押下、**期待結果** 3ファイルが作成され、編集用 terminal タブが開く。
2. **前提条件** `AGENTS.md` と `GEMINI.md` が既存だが `@CLAUDE.md` を含まない、**操作** ボタン押下、**期待結果** 既存内容を保持したまま `@CLAUDE.md` が補完される。

### ユーザーストーリー 2 - Windows でも編集起動を失敗させたくない (優先度: P0)

ユーザーとして、PowerShell/cmd 環境でも編集起動を成功させたい。

**独立したテスト**: Windows想定のコマンド分岐で `code` 優先・`notepad` フォールバックが選択されること。

**受け入れシナリオ**:

1. **前提条件** Windows + PowerShell/cmd + `code` 利用可能、**操作** ボタン押下、**期待結果** `code -g` で3ファイルが開く。
2. **前提条件** Windows + PowerShell/cmd + `code` 未導入、**操作** ボタン押下、**期待結果** `notepad` で3ファイル編集へフォールバックする。
3. **前提条件** Windows + WSL shell、**操作** ボタン押下、**期待結果** `vi CLAUDE.md AGENTS.md GEMINI.md` が起動する。

### ユーザーストーリー 3 - 不正な対象を誤って修正したくない (優先度: P1)

ユーザーとして、選択ブランチに対応する worktree が存在しない場合は処理を止めてほしい。

**独立したテスト**: worktree 未存在 branch 指定時に修正・編集起動が行われないこと。

## エッジケース

- `branch` が空文字の場合はバリデーションエラーを返す。
- `worktree` が Missing/Prunable/Locked で編集不能な場合はエラーで中断する。
- `CLAUDE.md` が存在しても空白のみの場合は初期テンプレートで再生成する。

## 要件

### 機能要件

- **FR-001**: Worktree詳細ビューのヘッダーに「Check/Fix Docs + Edit」ボタンを追加しなければならない。
- **FR-002**: ボタン押下時に backend command `check_and_fix_agent_instruction_docs(projectPath, branch)` を呼び出さなければならない。
- **FR-003**: backend は `projectPath` から repo を解決し、選択 `branch` の worktree を解決できない場合はエラーを返さなければならない。
- **FR-004**: `CLAUDE.md` は「存在・非空」であることを保証し、未存在または空の場合は初期テンプレートで作成/更新しなければならない。
- **FR-005**: `AGENTS.md` と `GEMINI.md` は `@CLAUDE.md` を含むことを保証し、未存在時は作成、既存時は他内容を保持したまま補完しなければならない。
- **FR-006**: backend command は `worktreePath`, `checkedFiles`, `updatedFiles` を返却しなければならない。
- **FR-007**: frontend は command 成功後に新規 terminal タブを開き、編集コマンドを自動投入しなければならない。
- **FR-008**: 編集コマンドは環境別に分岐し、WSL は `vi`、Windows PowerShell/cmd は `code` 優先かつ `notepad` フォールバックを使用しなければならない。
- **FR-009**: 実行中はボタンを disabled にし、多重実行を防止しなければならない。
- **FR-010**: `CLAUDE.md` 初期テンプレートは指定された Qiita 記事の構成（ワークフロー設計/タスク管理/コア原則）を反映しなければならない。

### 非機能要件

- **NFR-001**: 既存の Worktree Summary（Quick Launch/PR/Docker など）の既存挙動を変更しない。
- **NFR-002**: 自動修正は atomic write で行い、部分書き込みを避ける。
- **NFR-003**: Rust unit tests と Vitest を追加し、回帰防止を行う。

## 制約と仮定

- UI文言は英語。
- 修正対象は選択中ブランチの worktree のみ（projectPath直下へのフォールバックなし）。
- 編集起動のコマンド分岐は frontend で実施する。

## 成功基準

- **SC-001**: ボタン押下1回で3ファイル検査が完了し、必要なファイルのみ `updatedFiles` に含まれる。
- **SC-002**: `AGENTS.md` と `GEMINI.md` の既存記述を保持したまま `@CLAUDE.md` が補完される。
- **SC-003**: Windows (PowerShell/cmd) で `vi` 依存なく編集開始できる。
- **SC-004**: `cargo test -p gwt-tauri clause_docs` と `pnpm --dir gwt-gui test -- WorktreeSummaryPanel.test.ts` が成功する。
