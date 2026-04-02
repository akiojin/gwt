# Research: SPEC-1782

## session_id 検出方式の比較

### 方式 A: 各エージェントのセッションファイルを直接スキャン（採用）

gwt-cli で実証済み。各エージェントのセッションファイル形式に依存するが、外部プロセスを起動せずファイルシステムから直接取得できる。

| Agent | セッションファイル | 形式 |
|-------|-------------------|------|
| Claude Code | `~/.claude/projects/{encoded_path}/` + `~/.claude/projects/.history.jsonl` | JSONL — project + sessionId フィールド |
| Codex CLI | `~/.codex/sessions/` | JSONL — cwd マッチングで worktree スコープ |
| Gemini CLI | `~/.gemini/tmp/` | JSON — 最新ファイルの session_id |
| OpenCode | `~/.opencode/sessions/` | JSON — 最新セッション |

### 方式 B: PTY 出力パース（不採用）

エージェント起動時の PTY 出力から session_id を抽出する方式。エージェントの出力形式に依存し、VT100 エスケープシーケンスの中から正確にパースする必要がある。脆弱。

### 方式 C: SessionWatcher（不採用）

`~/.gwt/sessions/` の TOML ファイル変更を監視する方式。gwt-core に SessionWatcher が存在するが、これは gwt 自身のセッション記録を監視するもので、エージェント側のセッションファイルは監視しない。

## `--continue` vs `--resume` の判断

| フラグ | 動作 | 問題 |
|--------|------|------|
| `--continue` | 直前のセッションを継続 | グローバルスコープ。別ブランチで作業した後に戻ると、そのブランチのセッションではなく最後に使ったセッションを継続する |
| `--resume <id>` | 特定セッションを復元 | ブランチスコープ。session_id を指定するため意図したセッションを正確に復元できる |

**結論**: gwt はブランチ単位の管理ツールであるため、`--resume <id>` のみを使用する。
