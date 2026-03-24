# API コントラクト: Tauri コマンド

**仕様ID**: `SPEC-f490dded` | **日付**: 2026-02-13

## spawn_shell

素のシェル PTY を生成し、pane_id を返す。

### リクエスト

```
Command: spawn_shell
Params:
  working_dir: Option<String>  // 起動ディレクトリ。None → ホームディレクトリ
```

### レスポンス

```
Ok: String  // 生成された pane_id (例: "pane-a1b2c3d4")
Err: String // エラーメッセージ
```

### 振る舞い

1. `$SHELL` 環境変数を参照。未設定なら `/bin/sh`
2. `working_dir` が `None` → `$HOME` を使用
3. `working_dir` が存在しないパス → `$HOME` にフォールバック
4. PaneManager.spawn_shell() で PTY 生成
5. stream_pty_output() スレッドを起動
6. pane_id を返却

## Tauri イベント

### terminal-cwd-changed

ターミナルタブの cwd が OSC 7 で変更された際に発行。

```
Event: "terminal-cwd-changed"
Payload: {
  pane_id: String,
  cwd: String        // フルパス（URL デコード済み）
}
```

### 発行条件

- OSC 7 シーケンスがパースされ、新しい cwd が前回の値と異なる場合のみ発行
- ターミナルタブ（agent_name == "terminal"）の pane のみ対象
