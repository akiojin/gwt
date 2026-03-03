# Lessons Learned

## L001: 実装前ワークフロー（仕様策定 + TDD）の省略

- **事象**: プランの実装指示を受けた際、CLAUDE.md の実装前ワークフロー（GitHub Issue gwt-spec 作成 → TDD RED 確認）を省略し、直接実装に着手した
- **原因**: プランが詳細だったため、そのまま実装に入れると判断した。CLAUDE.md の必須ワークフローを確認しなかった
- **再発防止策**: feat / fix / refactor の実装開始前に必ず以下を確認する
  1. GitHub Issue (`gwt-spec` ラベル) が存在するか → なければ作成
  2. テストを先に書いて RED を確認 → その後に実装コードを書く
  3. `tasks/todo.md` に Plan・ステップを記録してから着手する
