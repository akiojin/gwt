# 技術調査: シンプルターミナルタブ

**仕様ID**: `SPEC-f490dded` | **日付**: 2026-02-13

## 調査項目と結論

### R-001: PaneManager.launch_agent() はシェルコマンドで使えるか

**結論**: 使用可能だが、ブランチマッピング問題あり

`launch_agent()` は `BuiltinLaunchConfig.command` に任意の文字列を受け付ける。
ただし内部で `save_branch_mapping(repo_root, &config.branch_name, &pane_id)` を呼び出し、
ブランチ→pane_id のマッピングを `~/.gwt/terminals/index/` に保存する。

ターミナルタブではブランチとの紐付けが不要。

**対応方針**: PaneManager に `spawn_shell()` メソッドを追加する。
`launch_agent()` のロジックを再利用しつつ、`save_branch_mapping()` を省略する。

```
crates/gwt-core/src/terminal/manager.rs:131-163
```

### R-002: AgentColor でグレーを表現できるか

**結論**: `AgentColor::White` を流用し、フロントエンドで `--text-muted` にマッピング

AgentColor enum の全バリアント:

- Green, Blue, Cyan, Red, Yellow, Magenta, White, Rgb(u8,u8,u8), Indexed(u8)

バックエンドは `White` を使用。フロントエンドの MainArea.svelte でタブタイプが
`terminal` の場合はドットカラーを `--text-muted` にオーバーライドする。

```
crates/gwt-core/src/terminal/mod.rs:15-26
```

### R-003: Ctrl+` は Tauri v2 の accelerator でサポートされるか

**結論**: 不確実 → フロントエンドキーイベントで実装

Tauri v2 の accelerator は `CmdOrCtrl+N` 等の形式を使用。バッククォートのサポートは
明示的に確認されていない。既存メニューでは `CmdOrCtrl+,` のように句読点を使用している
例がある。

**対応方針**: ネイティブメニューの accelerator としては設定を試みるが、
動作しない場合はフロントエンド側の `keydown` イベントリスナーで `Ctrl+Backquote`
を検出し、`menu-action` をディスパッチするフォールバックを実装。

```
crates/gwt-tauri/src/menu.rs:98-204
```

### R-004: stream_pty_output() に OSC 7 パースを追加する影響

**結論**: 低リスク・低コスト

`stream_pty_output()` は 4096 バイトバッファで PTY 出力を読み取り、
`terminal-output` イベントを発行する。OSC 7 チェックはバッファ内の `0x1b` バイトを
スキャンするだけで、見つからなければ即座にスキップ。

**注意事項**:

- OSC 7 シーケンスがバッファ境界で分断される可能性がある
- 対策: pane 単位で不完全なシーケンスを保持するバッファリングステートを追加
- ターミナルタブ専用なので、pane_id でフィルタリングしてエージェントタブへの影響を排除

```
crates/gwt-tauri/src/commands/terminal.rs:2596-2826
```

### R-005: BuiltinLaunchConfig の必須フィールド

**結論**: 全フィールド必須、デフォルト値なし

ターミナルタブ用の値:

| フィールド | ターミナルタブの値 |
|---|---|
| command | `$SHELL` or `/bin/sh` |
| args | `[]`（空） |
| working_dir | Worktree パス / プロジェクトルート / ホーム |
| branch_name | `""` or `"terminal"` |
| agent_name | `"terminal"` |
| agent_color | `AgentColor::White` |
| env_vars | `{}` |

```
crates/gwt-core/src/terminal/mod.rs:28-44
```

### R-006: OSC 7 シーケンスのフォーマット

**結論**: 標準仕様に準拠したパーサーを実装

OSC 7 フォーマット:

```
ESC ] 7 ; file://hostname/path BEL
ESC ] 7 ; file://hostname/path ESC \
```

- ESC = 0x1b
- BEL = 0x07
- ST (String Terminator) = ESC \ = 0x1b 0x5c
- hostname は省略可能（`file:///path` の形式）
- path は URL エンコードされている場合がある（%20 等）

macOS zsh が出力する実際の形式:

```
\e]7;file://MacBookPro.local/Users/akio/Workbench\a
```

パース手順:

1. バイトストリームで `0x1b 0x5d 0x37 0x3b` (ESC ] 7 ;) を検出
2. `file://` プレフィックスをスキップ
3. hostname の後の `/` からパスを取得
4. BEL (0x07) または ESC \ (0x1b 0x5c) で終端
5. URL デコード（% エンコードを解除）
