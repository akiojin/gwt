# クイックスタートガイド: カスタムAIツール対応機能

**日付**: 2025-10-28
**仕様ID**: SPEC-30f6d724

## 概要

このガイドでは、claude-worktreeにカスタムAIツールを追加する方法を説明します。

## 1. セットアップ手順

### 1.1 設定ファイルの作成

カスタムツール設定ファイルを作成します。

```bash
# ディレクトリ作成（存在しない場合）
mkdir -p ~/.claude-worktree

# 設定ファイル作成
touch ~/.claude-worktree/tools.json
```

### 1.2 基本的な設定例

`~/.claude-worktree/tools.json`に以下の内容を記述します。

```json
{
  "version": "1.0.0",
  "customTools": []
}
```

## 2. カスタムツールの追加

### 2.1 実行タイプ別の設定例

#### タイプ1: bunxでパッケージを実行

```json
{
  "version": "1.0.0",
  "customTools": [
    {
      "id": "aider",
      "displayName": "Aider",
      "icon": "🤖",
      "type": "bunx",
      "command": "aider-chat@latest",
      "modeArgs": {
        "normal": [],
        "continue": ["--continue"],
        "resume": ["--resume"]
      },
      "permissionSkipArgs": ["--yes"],
      "env": {
        "OPENAI_API_KEY": "sk-your-api-key"
      }
    }
  ]
}
```

#### タイプ2: 絶対パスで実行

```json
{
  "version": "1.0.0",
  "customTools": [
    {
      "id": "local-ai",
      "displayName": "Local AI Tool",
      "type": "path",
      "command": "/usr/local/bin/my-ai-tool",
      "modeArgs": {
        "normal": ["--mode", "interactive"],
        "continue": ["--continue"],
        "resume": ["--resume"]
      }
    }
  ]
}
```

#### タイプ3: コマンド名で実行（PATHから探す）

```json
{
  "version": "1.0.0",
  "customTools": [
    {
      "id": "cursor-cli",
      "displayName": "Cursor CLI",
      "type": "command",
      "command": "cursor",
      "defaultArgs": ["--verbose"],
      "modeArgs": {
        "normal": [],
        "continue": ["--continue"],
        "resume": ["--resume"]
      }
    }
  ]
}
```

### 2.2 複数ツールの登録

```json
{
  "version": "1.0.0",
  "customTools": [
    {
      "id": "aider",
      "displayName": "Aider",
      "type": "bunx",
      "command": "aider-chat@latest",
      "modeArgs": {
        "normal": []
      }
    },
    {
      "id": "cursor",
      "displayName": "Cursor",
      "type": "command",
      "command": "cursor",
      "modeArgs": {
        "normal": []
      }
    },
    {
      "id": "custom-claude",
      "displayName": "Custom Claude Wrapper",
      "type": "bunx",
      "command": "@my-org/claude-wrapper@latest",
      "defaultArgs": ["--config", "custom"],
      "modeArgs": {
        "normal": [],
        "continue": ["-c"],
        "resume": ["-r"]
      },
      "permissionSkipArgs": ["--yes"]
    }
  ]
}
```

## 3. 設定項目の詳細

### 3.1 必須フィールド

| フィールド | 説明 | 例 |
|------------|------|-----|
| `id` | ツールの一意識別子（英数字とハイフン） | `"aider"`, `"my-ai-tool"` |
| `displayName` | UI表示名 | `"Aider"`, `"私のAIツール"` |
| `type` | 実行方式（`"path"`, `"bunx"`, `"command"`） | `"bunx"` |
| `command` | 実行パス/パッケージ名/コマンド名 | `"aider-chat@latest"` |
| `modeArgs` | モード別引数 | `{ "normal": [] }` |

### 3.2 オプショナルフィールド

| フィールド | 説明 | 例 |
|------------|------|-----|
| `icon` | アイコン文字（Unicode） | `"🤖"`, `"🔧"` |
| `defaultArgs` | 常に付与される引数 | `["--verbose", "--auto-commit"]` |
| `permissionSkipArgs` | 権限スキップ時の引数 | `["--yes", "--skip-confirm"]` |
| `env` | 環境変数 | `{ "API_KEY": "sk-..." }` |

### 3.3 modeArgsの詳細

| モード | 説明 | 使用例 |
|--------|------|--------|
| `normal` | 通常起動時の引数 | `[]` または `["--mode", "interactive"]` |
| `continue` | 継続モード時の引数 | `["-c"]` または `["--continue"]` |
| `resume` | 再開モード時の引数 | `["-r"]` または `["resume", "--last"]` |

**重要**: 少なくとも1つのモードを定義する必要があります。

## 4. 開発ワークフロー

### 4.1 カスタムツールの追加手順

1. **ツールのインストール**（必要に応じて）
   ```bash
   # bunxの場合は不要（実行時に自動ダウンロード）
   # commandの場合は事前にインストール
   brew install aider  # 例
   ```

2. **設定ファイルの編集**
   ```bash
   # エディタで編集
   code ~/.claude-worktree/tools.json
   ```

3. **設定の検証**
   ```bash
   # claude-worktreeを起動して確認
   bunx .
   # または
   bun run start
   ```

4. **ツール選択画面で確認**
   - カスタムツールが一覧に表示されることを確認
   - ツールを選択して起動できることを確認

### 4.2 デバッグ方法

#### 設定読み込みのデバッグ

```bash
# 設定ファイルの読み込みログを表示
DEBUG_CONFIG=true bunx .
```

#### JSON構文エラーの確認

```bash
# JSON構文チェック（macOS/Linux）
cat ~/.claude-worktree/tools.json | jq .

# エラーがある場合、行番号が表示される
```

#### ツール起動のデバッグ

```bash
# ツール起動時の詳細ログ
DEBUG=true bunx .
```

## 5. よくある操作

### 5.1 新しいツールの登録

1. **ツールIDの決定**: 小文字英数字とハイフンのみ（例: `my-ai-tool`）
2. **実行タイプの選択**:
   - パッケージ名がある → `"bunx"`
   - 絶対パスで指定したい → `"path"`
   - PATHから探したい → `"command"`
3. **modeArgsの設定**: ツールのヘルプを確認して適切な引数を設定

### 5.2 モード別引数の設定

```json
// 例1: フラグ形式（Claude Code風）
"modeArgs": {
  "normal": [],
  "continue": ["-c"],
  "resume": ["-r"]
}

// 例2: サブコマンド形式（Codex CLI風）
"modeArgs": {
  "normal": [],
  "continue": ["resume", "--last"],
  "resume": ["resume"]
}

// 例3: オプション形式
"modeArgs": {
  "normal": ["--mode", "interactive"],
  "continue": ["--mode", "continue", "--auto-commit"],
  "resume": ["--mode", "resume"]
}
```

### 5.3 環境変数の設定

```json
{
  "id": "my-ai",
  "env": {
    "OPENAI_API_KEY": "sk-your-key",
    "ANTHROPIC_API_KEY": "sk-ant-...",
    "MY_TOOL_CONFIG": "/path/to/config.json"
  }
}
```

**注意**: APIキーは平文保存されるため、ファイルパーミッションに注意してください。

```bash
# 設定ファイルを自分だけ読み書き可能に
chmod 600 ~/.claude-worktree/tools.json
```

## 6. トラブルシューティング

### 6.1 JSON構文エラー

**症状**: `claude-worktree`起動時にエラーが表示される

**原因**: JSON形式が正しくない

**解決方法**:
```bash
# JSON構文をチェック
cat ~/.claude-worktree/tools.json | jq .

# エラーメッセージを確認して修正
```

**よくあるミス**:
- カンマの付け忘れ/余分なカンマ
- 引用符の閉じ忘れ
- 配列/オブジェクトの閉じ忘れ

### 6.2 コマンドが見つからない

**症状**: `type: "command"`のツールで「コマンドが見つかりません」エラー

**原因**: コマンドがPATH環境変数に存在しない

**解決方法**:
```bash
# コマンドのパスを確認
which my-command  # macOS/Linux
where my-command  # Windows

# 見つからない場合、絶対パスで指定
{
  "type": "path",
  "command": "/usr/local/bin/my-command"
}
```

### 6.3 権限エラー

**症状**: `type: "path"`のツールで「Permission denied」エラー

**原因**: 実行権限がない

**解決方法**:
```bash
# 実行権限を付与
chmod +x /path/to/your/tool
```

### 6.4 環境変数が反映されない

**症状**: ツール起動時に環境変数が設定されていない

**原因**: `env`フィールドの記述ミス

**解決方法**:
```json
// 正しい形式
"env": {
  "MY_VAR": "value"
}

// 間違った形式
"env": "MY_VAR=value"  // NG
"env": ["MY_VAR=value"]  // NG
```

### 6.5 ツールが一覧に表示されない

**症状**: カスタムツールが選択画面に表示されない

**原因**:
1. 設定ファイルのパスが間違っている
2. JSON構文エラー
3. 検証エラー（id重複、必須フィールド不足）

**解決方法**:
```bash
# 設定ファイルの場所を確認
ls -la ~/.claude-worktree/tools.json

# デバッグモードで起動
DEBUG_CONFIG=true bunx .
```

## 7. サンプル設定

### 7.1 最小構成

```json
{
  "version": "1.0.0",
  "customTools": [
    {
      "id": "aider",
      "displayName": "Aider",
      "type": "command",
      "command": "aider",
      "modeArgs": {
        "normal": []
      }
    }
  ]
}
```

### 7.2 完全な設定例

```json
{
  "version": "1.0.0",
  "customTools": [
    {
      "id": "aider",
      "displayName": "Aider",
      "icon": "🤖",
      "type": "bunx",
      "command": "aider-chat@latest",
      "defaultArgs": ["--auto-commits"],
      "modeArgs": {
        "normal": [],
        "continue": ["--continue"],
        "resume": ["--resume"]
      },
      "permissionSkipArgs": ["--yes"],
      "env": {
        "OPENAI_API_KEY": "sk-your-key"
      }
    },
    {
      "id": "cursor",
      "displayName": "Cursor AI",
      "icon": "📝",
      "type": "command",
      "command": "cursor",
      "modeArgs": {
        "normal": ["--verbose"]
      }
    },
    {
      "id": "custom-ai",
      "displayName": "My Custom AI",
      "type": "path",
      "command": "/Users/me/bin/my-ai",
      "defaultArgs": ["--config", "/Users/me/.config/my-ai.json"],
      "modeArgs": {
        "normal": [],
        "continue": ["--continue"],
        "resume": ["--resume", "--interactive"]
      },
      "env": {
        "MY_AI_API_KEY": "secret",
        "MY_AI_MODEL": "gpt-4"
      }
    }
  ]
}
```

## 8. 次のステップ

1. **設定ファイルを作成** (`~/.claude-worktree/tools.json`)
2. **カスタムツールを追加** (上記の例を参考に)
3. **claude-worktreeを起動** して動作確認
4. **問題があれば** トラブルシューティングを参照

詳細な仕様は以下を参照してください：
- [spec.md](./spec.md) - 機能仕様
- [data-model.md](./data-model.md) - データモデル詳細
- [plan.md](./plan.md) - 実装計画
