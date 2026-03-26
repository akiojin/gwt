### Backend

- `head` が base に対して behind のとき `status=behind` になり `blockingReason` を返す
- `head` が base に対して diverged のとき `status=diverged` になり blocking される
- `head` が ahead-only / up-to-date のとき blocking されない
- `origin/<base>` が解決できない場合は error を返す

### Frontend

- PR 未作成 + behind で blocking banner が表示される
- PR 未作成 + diverged で blocking banner が表示される
- PR 未作成 + up-to-date で banner は表示されない
- preflight エラー時に既存の PR なし表示は保たれる

### Skills / Commands

- `gwt-pr` skill に preflight 手順が含まれる
- Codex `gh-pr` skill に preflight 手順が含まれる
- Claude Code `gh-pr` skill / command に preflight 手順が含まれる
