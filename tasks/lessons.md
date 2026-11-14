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

## 2026-07-27 — Issue Monitor「Git エラー再発」調査

### 事象

v9.69.0 で修正済みのはずの `Current branch is unavailable: Not a git repository` が
Issue Monitor に表示され続け、新規バグと誤認しかけた。さらに新規登録した Issue #3348 が
Monitor に表示されず、独立した scan driver 凍結 (#3349) を発見した。

### 原因

- エラー文言の生成元が「稼働中バイナリの版」に存在するかを最初に確認していなかった。
  実体は v9.66.0 時代に永続化された `failed_issues` の読み戻しだった。
- Monitor の scan driver 凍結は無音（persist が止まるだけ）で、UI は静止画を表示し続ける。

### 再発防止策

1. エラー文言の調査は、コードより先に **生成可能性の版特定**:
   `git show <tag>:<path>`（worktree のファイルは版が違う）、`git tag --contains <sha>`、
   `~/.gwt/logs/update-*.log`（中間版を飛ばすことがある）。生成元が現行版に無ければ
   persisted state（`issue-monitor.json` 等）の読み戻し。
2. Monitor 無音凍結の診断: `issue-monitor.json` の mtime（persist は毎 scan 後必須・
   poll 300s）が止まっていれば driver 凍結。`works.json` / `current.json` が更新中なら
   gwtd 生存 = monitor task だけの死。バナー不変（record_scan_error 未到達）なら
   エラーでなくハング。
3. intake セッションの workflow-policy hook が `issue.create` / `issue.comment` /
   `memory.add` を owner 未リンクで拒否する間は、封筒 JSON を `tasks/` に書き
   ユーザーに `! gwtd < tasks/<envelope>.json` を実行してもらう（#3265 の症状。
   パイプ・ファイル redirect・可視 heredoc すべてブロックされる）。
