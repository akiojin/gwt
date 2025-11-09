# データモデル: Worktree内でのコマンド実行制限機能

**日付**: 2025-11-09
**仕様ID**: SPEC-eae13040

## 概要

このドキュメントは、PreToolUseフックスクリプトで使用されるデータ構造を定義します。フックスクリプトはステートレスであり、永続化されるデータは存在しませんが、以下のデータ構造を入出力として扱います。

## 1. フック入力スキーマ

### 1.1 JSON入力フォーマット

Claude CodeのPreToolUseフック機構から、以下のJSON形式でデータが`stdin`経由で渡されます。

```json
{
  "tool_name": "Bash",
  "tool_input": {
    "command": "git checkout main",
    "description": "Switch to main branch",
    "timeout": 120000
  }
}
```

**フィールド定義**:

| フィールド | 型 | 必須 | 説明 |
|------------|-----|------|------|
| `tool_name` | `string` | Yes | ツール名。Bashツールの場合は`"Bash"` |
| `tool_input` | `object` | Yes | ツール入力パラメータ |
| `tool_input.command` | `string` | Yes | 実行されるBashコマンド |
| `tool_input.description` | `string` | No | コマンドの説明(省略可) |
| `tool_input.timeout` | `number` | No | タイムアウト(ミリ秒、省略可) |

**検証ルール**:
- `tool_name`が`"Bash"`でない場合、フックは許可(exit 0)
- `tool_input.command`が空の場合、フックは許可(exit 0)

## 2. フック出力スキーマ

### 2.1 ブロック時のJSON出力

コマンドがブロックされる場合、以下のJSON形式で`stdout`に出力されます。

```json
{
  "decision": "block",
  "reason": "🚫 Branch switching, creation, and worktree commands are not allowed",
  "stopReason": "Worktree is designed to complete work on the launched branch. Branch operations such as git checkout, git switch, git branch, and git worktree cannot be executed.\n\nBlocked command: git checkout main"
}
```

**フィールド定義**:

| フィールド | 型 | 必須 | 説明 |
|------------|-----|------|------|
| `decision` | `string` | Yes | 常に`"block"` |
| `reason` | `string` | Yes | ブロック理由の短い要約(絵文字付き) |
| `stopReason` | `string` | Yes | ブロック理由の詳細説明と代替手段 |

**検証ルール**:
- `decision`は常に`"block"`
- `reason`は1行の簡潔なメッセージ
- `stopReason`は複数行の詳細説明を含む

### 2.2 許可時の出力

コマンドが許可される場合、JSON出力なし(`exit 0`のみ)。

## 3. コマンドパターン定義

### 3.1 禁止コマンドパターン

各フックスクリプトで禁止されるコマンドパターン。

#### 3.1.1 block-cd-command.sh

| パターン | 正規表現 | 説明 |
|----------|---------|------|
| cd to absolute path | `^(builtin\s+)?(command\s+)?cd\s+/` | 絶対パスへのcd |
| cd to home | `^(builtin\s+)?(command\s+)?cd\s*(~\|$)` | ホームディレクトリへのcd |
| cd to parent | `^(builtin\s+)?(command\s+)?cd\s+\.\./` | 親ディレクトリへのcd(Worktree外の場合) |

**判定ロジック**:
1. コマンドセグメントから`cd`を抽出
2. ターゲットパスを抽出
3. `is_within_worktree()`関数でWorktree境界をチェック
4. Worktree外なら`block`

#### 3.1.2 block-git-branch-ops.sh

| パターン | 正規表現 | 説明 |
|----------|---------|------|
| git checkout branch | `^git\s+checkout\s+[^-]` | ブランチ切り替え(ファイル復元以外) |
| git switch | `^git\s+switch\b` | ブランチ切り替え |
| git branch create | `^git\s+branch\s+[^-]` | ブランチ作成 |
| git branch delete | `^git\s+branch\s+(-d\|-D\|--delete)` | ブランチ削除 |
| git branch move | `^git\s+branch\s+(-m\|-M\|--move)` | ブランチ移動/リネーム |
| git worktree | `^git\s+worktree\b` | Worktree操作 |

**例外パターン(許可)**:
| パターン | 正規表現 | 説明 |
|----------|---------|------|
| git checkout file | `^git\s+checkout\s+--\s` | ファイル復元 |
| git branch list | `^git\s+branch\s+(--list\|--contains\|--merged\|--no-merged\|--points-at\|--format\|--sort\|--abbrev)` | 参照系オプション |
| git branch no args | `^git\s+branch\s*$` | 引数なし(現在のブランチ表示) |

#### 3.1.3 block-file-ops.sh (新規)

| パターン | 正規表現 | 説明 |
|----------|---------|------|
| mkdir outside worktree | `^mkdir\s+` | Worktree外へのディレクトリ作成 |
| rm outside worktree | `^rm\s+` | Worktree外のファイル/ディレクトリ削除 |
| touch outside worktree | `^touch\s+` | Worktree外のファイル作成 |
| cp to/from outside | `^cp\s+` | Worktree外へのコピー |
| mv to/from outside | `^mv\s+` | Worktree外への移動 |

**判定ロジック**:
1. コマンドセグメントからファイル操作コマンドを抽出
2. 全ての引数(パス)を抽出
3. 各パスに対して`is_within_worktree()`でチェック
4. 1つでもWorktree外のパスがあれば`block`

### 3.2 許可オプションリスト(git branch)

`is_read_only_git_branch()`関数で使用される参照系オプションのリスト。

**参照系オプション**:
- `--list` / `-l`: ブランチ一覧表示
- `--contains <commit>`: 特定コミットを含むブランチ
- `--merged [<commit>]`: マージ済みブランチ
- `--no-merged [<commit>]`: 未マージブランチ
- `--points-at <object>`: 特定オブジェクトを指すブランチ
- `--format <format>`: フォーマット指定
- `--sort <key>`: ソート順指定
- `--abbrev[=<n>]`: SHA-1の省略形式

**変更系オプション(禁止)**:
- `-d` / `-D` / `--delete`: ブランチ削除
- `-m` / `-M` / `--move`: ブランチ移動/リネーム
- `-c` / `-C` / `--copy`: ブランチコピー
- `--create-reflog`: reflog作成
- `--set-upstream-to`: アップストリーム設定
- `--unset-upstream`: アップストリーム解除
- `--track` / `--no-track`: トラッキング設定
- `--edit-description`: ブランチ説明編集
- `-f` / `--force`: 強制実行

## 4. Worktree境界情報

### 4.1 Worktreeルートパス

**取得方法**:
```bash
WORKTREE_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)
if [ -z "$WORKTREE_ROOT" ]; then
    WORKTREE_ROOT=$(pwd)
fi
```

**データ型**: `string` (絶対パス)

**例**:
```
/claude-worktree/.worktrees/feature-disallow-bash-command
```

**検証ルール**:
- 空文字列でない
- 絶対パス形式(`/`で始まる)

### 4.2 判定結果

`is_within_worktree()`関数の戻り値。

**データ型**: `number` (exit code)

| 値 | 意味 |
|----|------|
| 0 | Worktree内 |
| 1 | Worktree外 |

**例**:
```bash
if is_within_worktree "/tmp/file.txt"; then
    echo "Worktree内"  # 実行されない
else
    echo "Worktree外"  # 実行される
fi
```

## 5. コマンドセグメント

### 5.1 複合コマンドの分割

複合コマンドを演算子で分割した個別のコマンド単位。

**演算子リスト**:
- `&&`: AND演算子
- `||`: OR演算子
- `;`: セミコロン
- `|`: パイプ
- `|&`: パイプ(stderr含む)
- `&`: バックグラウンド実行

**分割方法**:
```bash
command_segments=$(printf '%s\n' "$command" | sed -E 's/\|&/\n/g; s/\|\|/\n/g; s/&&/\n/g; s/[;|&]/\n/g')
```

**例**:
```
入力: echo "test" && git checkout main
出力:
  セグメント1: echo "test"
  セグメント2: git checkout main
```

**検証ルール**:
- 各セグメントは独立して判定される
- 1つでも禁止コマンドが含まれていれば、全体をブロック

### 5.2 トークン化(Python shlex.split())

コマンドセグメントをトークンに分割。

**使用例**:
```python
import shlex
tokens = shlex.split("git branch --list 'my branch'")
# ['git', 'branch', '--list', 'my branch']
```

**特徴**:
- クォート(`'`, `"`)を正しく処理
- エスケープ(`\`)を正しく処理
- ホワイトスペースを正しく分割

**フォールバック**:
Python3が利用不可の場合は`read -r -a`を使用(クォート処理が不完全)。

## 6. エラー応答

### 6.1 ブロックメッセージフォーマット

**stdoutへのJSON出力**:
```json
{
  "decision": "block",
  "reason": "🚫 <短い理由>",
  "stopReason": "<詳細な説明>\n\n<代替手段>"
}
```

**stderrへのログ出力**:
```
🚫 Blocked: <command>
Reason: <理由>
Worktree root: <WORKTREE_ROOT>
```

**終了コード**: `exit 2`

### 6.2 許可メッセージ

**stdout**: 出力なし
**stderr**: 出力なし
**終了コード**: `exit 0`

## 7. 状態遷移

フックスクリプトはステートレスのため、状態遷移は存在しません。各実行は独立しており、過去の実行結果に影響されません。

## 8. 制約

### 8.1 データサイズ制約

- コマンド長: 制限なし(Bash/シェルの制限に依存)
- JSON入力サイズ: 制限なし
- JSON出力サイズ: 制限なし

### 8.2 パフォーマンス制約

- フック実行時間: <100ms (目標)
- コマンドパターンマッチング: O(n) (nはセグメント数)
- Worktree境界判定: O(1)

## 9. セキュリティ考慮事項

### 9.1 インジェクション対策

- JSONパースにjqを使用(eval回避)
- コマンドパターンマッチングに正規表現を使用(eval回避)
- パス解決に`realpath`を使用(シンボリックリンク攻撃対策)

### 9.2 機密情報の取り扱い

- エラーメッセージにWorktreeルートパスを含む(絶対パス)
- ユーザー名やホームディレクトリパスが露出する可能性
- **緩和策**: エラーメッセージは開発環境内でのみ表示され、外部に送信されない

## 10. 拡張性

### 10.1 新しいコマンドパターンの追加

新しい禁止コマンドを追加する場合:

1. 正規表現パターンを定義
2. コマンドセグメントループ内に判定ロジックを追加
3. エラーメッセージを定義
4. テストケースを追加

### 10.2 新しいフックスクリプトの追加

新しいフックスクリプトを追加する場合:

1. `.claude/hooks/`に新しいスクリプトを作成
2. `.claude/settings.json`の`hooks.PreToolUse`に登録
3. JSON入出力フォーマットを踏襲
4. テストケースを追加

## 11. 参考資料

- [Claude Code PreToolUseフック仕様](https://docs.claude.com/claude-code/hooks)
- [既存のblock-cd-command.sh](../../../.claude/hooks/block-cd-command.sh)
- [既存のblock-git-branch-ops.sh](../../../.claude/hooks/block-git-branch-ops.sh)
