1. spec に対応する failing tests を backend / frontend / skill 整合で先に追加する。
2. `gwt-core` / `gwt-tauri` に PR preflight 判定 API を追加する。
3. `WorktreeSummaryPanel` と `PrStatusSection` に PR 未作成時の preflight 表示を追加する。
4. `plugins/gwt/skills/gwt-pr` と各 command 文書を更新する。
5. Codex home `gh-pr` と Claude Code 側 `gh-pr` を同じ preflight rule に更新する。
6. 検証、issue / tasks 反映、コミット・push を行う。
