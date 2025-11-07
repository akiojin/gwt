# データモデル: Worktree側での.gitignore更新と重複登録防止

## 1. ActiveWorktreeRoot

| フィールド | 型 | 説明 |
| --- | --- | --- |
| `path` | `string` | `git rev-parse --show-toplevel` が返す絶対パス。`.gitignore` のターゲットディレクトリ |
| `mode` | `"worktree" \| "root"` | `isInWorktree()` 判定またはフォールバックで決定。ロギングやフォールバック判別に使用 |

## 2. GitignoreSnapshot

| フィールド | 型 | 説明 |
| --- | --- | --- |
| `content` | `string` | 既存 `.gitignore` 全文（存在しない場合は空文字） |
| `lines` | `string[]` | `content.split(/\r?\n/)` 後に `trim()` した配列。重複検知に使用 |
| `eol` | `"\n" \| "\r\n"` | `content` から推定した改行コード。追記でも同じ改行を利用 |
| `hasTrailingNewline` | `boolean` | 末尾が `\n` / `\r\n` で終わっているか。追記前の改行挿入判断に使用 |

## 3. GitignoreUpdateResult

| フィールド | 型 | 説明 |
| --- | --- | --- |
| `updated` | `boolean` | `.worktrees/` 行を追加した場合 true |
| `path` | `string` | 更新した `.gitignore` のパス |
| `reason` | `"already-present" \| "added" \| "skipped"` | 追加しなかった理由。ログやテスト assertions に利用する拡張余地 |

> 今回はシンプルな文字列操作のみだが、明示的に構造を定義することでテスト容易性と将来の拡張を担保する。
