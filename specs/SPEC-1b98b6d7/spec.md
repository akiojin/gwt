# 機能仕様: Claude Code Hooks 経由の gwt-tauri hook 実行で GUI を起動しない

**仕様ID**: `SPEC-1b98b6d7`
**作成日**: 2026-02-10
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**: `SPEC-861d8cdf`（エージェント状態の可視化）
**入力**: ユーザー説明: "gwt を終了させると新しい gwt が立ち上がり、終了のたびに増殖する。CLI では問題なかったが GUI 化後に発生している。Hook が影響している可能性が高い。"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Hook 実行で gwt GUI が増殖起動しない (優先度: P0)

開発者として、Claude Code の Hook（PreToolUse / PostToolUse / Stop 等）が発火しても gwt GUI が新規起動してしまわないようにしたい。
Hook の目的はセッション状態更新であり、Hook 実行を契機に GUI を立ち上げる必要はない。

**この優先度の理由**: Hook が頻繁に発火するため、GUI 起動が混入するとプロセスが増殖し、作業継続が困難になる。

**独立したテスト**: Claude Code hooks を有効にした状態でツール実行/停止を繰り返し、gwt GUI が意図せず起動・増殖しないことを確認できる。

**受け入れシナリオ**:

1. **前提条件** Claude Code hooks が `gwt-tauri hook PreToolUse` を呼び出す設定になっている、**操作** Claude Code で任意のツールを実行する、**期待結果** gwt GUI が新規起動しない
2. **前提条件** gwt GUI が起動している、**操作** Claude Code が Stop イベントを発火する操作を行う（例: セッション停止）、**期待結果** gwt GUI が増殖起動しない
3. **前提条件** Claude Code hooks が有効、**操作** Hook が短時間に複数回発火する状況を作る（PreToolUse/PostToolUse 等）、**期待結果** `gwt-tauri hook <Event>` の各プロセスが短時間で終了し常駐しない

---

### ユーザーストーリー 2 - Hook によりセッション状態が更新される (優先度: P1)

開発者として、Hook のイベントに応じて gwt のセッション状態（running / waiting_input / stopped）が更新され、UI の状態表示が正しくなるようにしたい。

**独立したテスト**: Hook payload（stdin JSON）を与えて `~/.gwt/sessions/*.toml` の状態が更新されることを確認できる。

**受け入れシナリオ**:

1. **前提条件** 対象 worktree に既存セッションがある、**操作** `PreToolUse` を処理する、**期待結果** status が `running` になる
2. **前提条件** 対象 worktree に既存セッションがある、**操作** `Notification(permission_prompt)` を処理する、**期待結果** status が `waiting_input` になる
3. **前提条件** 対象 worktree に既存セッションがある、**操作** `Stop` を処理する、**期待結果** status が `stopped` になる
4. **前提条件** 対象 worktree に既存セッションがない、**操作** Hook を処理する、**期待結果** 何も作成せずエラーなく終了する（no-op）

## 要件 *(必須)*

### 機能要件

- **FR-700**: `gwt-tauri hook <EventName>` は Hook 実行時に GUI を起動**してはならない**
- **FR-701**: `gwt-tauri hook <EventName>` は stdin の JSON payload を処理し、必要なセッション更新を行った後に短時間で終了**しなければならない**
- **FR-702**: Hook payload が空、または worktree パスが解決できない場合、処理は no-op で終了**しなければならない**
- **FR-703**: 対象 worktree のセッションが存在しない場合、セッションを新規作成せず no-op で終了**しなければならない**
- **FR-704**: `PreToolUse` / `PostToolUse` / `UserPromptSubmit` は status を `running` に更新**しなければならない**
- **FR-705**: `Notification(permission_prompt)` は status を `waiting_input` に更新**しなければならない**
- **FR-706**: `Stop` は status を `stopped` に更新**しなければならない**

## 成功基準 *(必須)*

- **SC-700**: Claude Code hooks 有効状態でも gwt GUI が増殖起動しない（常に）
- **SC-701**: Hook 実行プロセス（`gwt-tauri hook ...`）が常駐せず短時間で終了する
- **SC-702**: 既存セッションがある場合のみ status が更新され、セッションが無い場合は no-op で終わる

## 制約と仮定 *(該当する場合)*

### 制約

- Hook は Claude Code 側の設定（`~/.claude/settings.json`）により `gwt-tauri` が直接呼ばれる
- Hook 実行の失敗により Claude Code の操作が阻害されるべきではない（no-op で継続する）

## 範囲外 *(必須)*

- Hook でセッションが存在しない場合のセッション新規作成
- Hook 実行結果を Claude Code へフィードバックして動作を変更する（additionalContext/updatedInput 等）
