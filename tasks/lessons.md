# Lessons Learned

## L001: 実装前ワークフロー（仕様策定 + TDD）の省略

- **事象**: プランの実装指示を受けた際、CLAUDE.md の実装前ワークフロー（GitHub Issue gwt-spec 作成 → TDD RED 確認）を省略し、直接実装に着手した
- **原因**: プランが詳細だったため、そのまま実装に入れると判断した。CLAUDE.md の必須ワークフローを確認しなかった
- **再発防止策**: feat / fix / refactor の実装開始前に必ず以下を確認する
  1. GitHub Issue (`gwt-spec` ラベル) が存在するか → なければ作成
  2. テストを先に書いて RED を確認 → その後に実装コードを書く
  3. `tasks/todo.md` に Plan・ステップを記録してから着手する

## 2026-03-03 — refactor: ChromaDB コレクション名リネーム

### 事象

`refactor:` タイプのコミット（Python スクリプト内の文字列定数リネーム）を実装した後、
ユーザー確認でGitHub Issue 未作成・TDD テスト未追加の漏れが発覚した。

### 原因

- 変更規模が軽微（1ファイル・8行）だったため、CLAUDE.md の「`refactor:` 対象は
  GitHub Issue を作成してから実装」ルールを省略してしまった。
- Python スクリプト内の定数は Rust ユニットテストから直接検証しにくいと思い込み、
  `include_str!` で埋め込まれた文字列として検証できる手段を見落とした。

### 再発防止策

1. **コミットタイプに関わらず** `feat` / `fix` / `refactor` は必ず実装前に
   `gwt-spec` ラベル付き Issue を作成する。適用除外は `docs:` / `chore:` のみ。
2. Python スクリプトが `include_str!` で埋め込まれている場合、
   定数文字列の存在チェックは Rust の `assert!(SCRIPT.contains("..."))` で書ける。
   「Python だからテストしにくい」は誤り。
3. 計画（Plan）実行時に「仕様策定・TDD は済んでいるか？」を必ずチェックリストで確認してから実装に着手する。

## 2026-07-25: intake worktree が stale main で誤検出を報告

- 事象: intake セッション（.intake-32, main PR #1725 相当・Tauri gwt-gui 構成）
  の checked-out tree だけを調査し、「Tauri GUI の Codex リストが FR-121 と
  乖離」とユーザーに報告した。実際は origin/develop（PR #3336 相当、
  crates/gwt + launch_wizard + embedded web 構成、gwt-gui は廃止済み）では
  Phase 70 / PR #3270 で準拠済みで、乖離は存在しなかった。
- 原因: intake worktree は branchless 使い捨てで base が大きく遅れることが
  あるのに、手元 tree = 現行実装と思い込んだ。
- 再発防止策: intake セッションで実装状態・乖離・表示文言を報告する前に、
  `git show origin/develop:<path>` / `git ls-tree origin/develop` で develop の
  実体を必ず確認する。

## 2026-07-25: stale intake worktree で gwtd 全操作拒否の catch-22

- 事象: gwtd PreToolUse workflow-policy が全 gwtd JSON 操作（`board.show` /
  `plan.start` / `issue.spec.section` 等の読み取り含む）を "Owner Issue/SPEC
  is required" で拒否。`gh issue ...` は別ルールで「gwtd を使え」と禁止。
  owner リンク手段として案内される `gwt-execute` は stale worktree の
  スキルセットに不在で、SPEC 更新登録が構造的に不可能だった。read-only Bash
  や scratchpad / ~/.claude 配下への Write まで誤ブロックされた。
- 原因: 古い worktree のスキル/フック契約と新しい gwtd バイナリのポリシーの
  不整合（と思われる）。加えてポリシー分類器の過剰ブロック。
- 再発防止策: "Owner Issue/SPEC is required" が gwtd 読み取りにまで出続けたら
  リトライで消耗しない。許可領域（`tasks/`・`.gwt/`・ドキュメント）へ成果物を
  退避し（例: tasks/todo.md に spec/tasks 追記案を保全）、Execute レーン
  （`gwt-execute #N`）への handoff として報告する。この不整合自体を gwt の
  バグ Issue 候補として提示する。
