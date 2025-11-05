# データモデル: カスタムAIツール対応機能

**日付**: 2025-10-28
**仕様ID**: SPEC-30f6d724

## 概要

カスタムAIツール対応機能で使用するデータモデルを定義する。設定ファイル、ツール定義、セッションデータの構造とその関係性を明確化する。

## 1. エンティティ定義

### 1.1 ToolsConfig

設定ファイル全体を表すルートエンティティ。

**属性**:

| 属性名 | 型 | 必須 | 説明 |
|--------|---|------|------|
| version | string | Yes | 設定フォーマットのバージョン（例: "1.0.0"） |
| customTools | CustomAITool[] | Yes | カスタムツール定義の配列 |

**検証ルール**:
- `version`は`"1.0.0"`形式（セマンティックバージョニング）
- `customTools`は空配列も許可（ビルトインツールのみ使用）

**例**:
```json
{
  "version": "1.0.0",
  "customTools": [
    { /* CustomAITool */ }
  ]
}
```

---

### 1.2 CustomAITool

個別のカスタムツール定義を表すエンティティ。

**属性**:

| 属性名 | 型 | 必須 | 説明 |
|--------|---|------|------|
| id | string | Yes | 一意識別子（英数字とハイフン、例: "my-ai-tool"） |
| displayName | string | Yes | UI表示名（日本語可、例: "私のAIツール"） |
| icon | string | No | アイコン文字（Unicode、例: "🤖"） |
| type | ToolExecutionType | Yes | 実行方式（"path" \| "bunx" \| "command"） |
| command | string | Yes | 実行パス/パッケージ名/コマンド名 |
| defaultArgs | string[] | No | 常に付与される引数 |
| modeArgs | ModeArgs | Yes | モード別引数 |
| permissionSkipArgs | string[] | No | 権限スキップ時に追加される引数 |
| env | Record<string, string> | No | ツール実行時の環境変数 |

**検証ルール**:

1. **id**:
   - パターン: `^[a-z0-9-]+$`（小文字英数字とハイフン）
   - 重複不可（同一ToolsConfig内で一意）

2. **displayName**:
   - 最小長: 1文字
   - 最大長: 50文字

3. **type**:
   - 許可値: `"path"`, `"bunx"`, `"command"`

4. **command**:
   - `type="path"`: 絶対パス（`/`で始まる、または`C:\`で始まる）
   - `type="bunx"`: パッケージ名（例: `@org/package@version`）
   - `type="command"`: コマンド名（例: `my-tool`）

5. **modeArgs**:
   - `normal`, `continue`, `resume` の少なくとも1つを定義

**例（type別）**:

```json
// type: "path"
{
  "id": "local-ai",
  "displayName": "Local AI Tool",
  "type": "path",
  "command": "/usr/local/bin/my-ai-tool",
  "modeArgs": {
    "normal": [],
    "continue": ["--continue"],
    "resume": ["--resume"]
  }
}

// type: "bunx"
{
  "id": "custom-claude",
  "displayName": "Custom Claude",
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

// type: "command"
{
  "id": "aider",
  "displayName": "Aider",
  "type": "command",
  "command": "aider",
  "modeArgs": {
    "normal": [],
    "continue": ["--continue"],
    "resume": ["--resume"]
  },
  "env": {
    "AIDER_API_KEY": "sk-..."
  }
}
```

---

### 1.3 ModeArgs

実行モード別の引数を定義するサブエンティティ。

**属性**:

| 属性名 | 型 | 必須 | 説明 |
|--------|---|------|------|
| normal | string[] | No | 通常モード時の引数（デフォルト: `[]`） |
| continue | string[] | No | 継続モード時の引数（デフォルト: `[]`） |
| resume | string[] | No | 再開モード時の引数（デフォルト: `[]`） |

**検証ルール**:
- 少なくとも1つのフィールドを定義
- 各配列は空配列も許可

**例**:
```json
{
  "normal": [],
  "continue": ["-c", "--auto-commit"],
  "resume": ["-r"]
}
```

---

### 1.4 SessionData（拡張）

セッション情報を保存するエンティティ。既存フィールドに`lastUsedTool`を追加。

**既存属性**:

| 属性名 | 型 | 必須 | 説明 |
|--------|---|------|------|
| lastWorktreePath | string \| null | Yes | 最後に使用したworktreeのパス |
| lastBranch | string \| null | Yes | 最後に使用したブランチ名 |
| timestamp | number | Yes | セッション保存時刻（Unixタイムスタンプミリ秒） |
| repositoryRoot | string | Yes | リポジトリルートパス |

**新規属性**:

| 属性名 | 型 | 必須 | 説明 |
|--------|---|------|------|
| lastUsedTool | string | No | 最後に使用したツールID（カスタムまたはビルトイン） |

**検証ルール**:
- `lastUsedTool`は省略可能（後方互換性）
- `timestamp`から24時間以内のセッションのみ有効

**例**:
```json
{
  "lastWorktreePath": "/path/to/worktree",
  "lastBranch": "feature/my-feature",
  "timestamp": 1698765432000,
  "repositoryRoot": "/path/to/repo",
  "lastUsedTool": "custom-claude"
}
```

---

### 1.5 AIToolConfig（内部使用）

ビルトインツールとカスタムツールを統合して扱うための内部エンティティ。

**属性**:

| 属性名 | 型 | 必須 | 説明 |
|--------|---|------|------|
| id | string | Yes | ツールID |
| displayName | string | Yes | UI表示名 |
| icon | string | No | アイコン文字 |
| isBuiltin | boolean | Yes | ビルトインツールか（true）、カスタムか（false） |
| customConfig | CustomAITool | No | カスタムツールの場合、元の設定 |

**例**:
```typescript
// ビルトインツール
{
  id: "claude-code",
  displayName: "Claude Code",
  isBuiltin: true,
}

// カスタムツール
{
  id: "my-ai",
  displayName: "My AI Tool",
  icon: "🤖",
  isBuiltin: false,
  customConfig: { /* CustomAITool */ }
}
```

## 2. エンティティ関係図

```text
ToolsConfig (1)
  │
  ├─── customTools (0..*)
  │    │
  │    └─── CustomAITool
  │         │
  │         ├─── modeArgs (1)
  │         │    └─── ModeArgs
  │         │
  │         └─── env (0..1)
  │              └─── Record<string, string>
  │
  └─── [Builtin Tools]
       └─── CustomAITool (内部定義)

SessionData (1)
  │
  └─── lastUsedTool (0..1)
       └─── ツールID（CustomAITool.id または Builtin ID）
```

**関係**:

1. **ToolsConfig → CustomAITool**: 1対多（1つのToolsConfigが0個以上のCustomAIToolを持つ）
2. **CustomAITool → ModeArgs**: 1対1（各CustomAIToolが1つのModeArgsを持つ）
3. **SessionData → ツールID**: 0対1（SessionDataは0個または1個のツールIDを参照）

## 3. 状態遷移

### 3.1 ツール選択フロー

```text
[起動]
  │
  ├─→ [設定ファイル読み込み]
  │    │
  │    ├─ ファイル存在 → [ToolsConfig解析]
  │    │                   │
  │    │                   ├─ JSON正常 → [検証]
  │    │                   │              │
  │    │                   │              ├─ 検証OK → [カスタムツール登録]
  │    │                   │              └─ 検証NG → [エラー表示] → [終了]
  │    │                   │
  │    │                   └─ JSON異常 → [エラー表示] → [終了]
  │    │
  │    └─ ファイル不在 → [ビルトインツールのみ]
  │
  ├─→ [ツール一覧生成]
  │    │
  │    └─ getAllTools() → [ビルトイン + カスタム]
  │
  ├─→ [UI表示]
  │    │
  │    └─ AIToolSelectorScreen → [ツール選択待ち]
  │
  ├─→ [ツール選択]
  │    │
  │    └─ ツールID取得 → [実行モード選択]
  │
  ├─→ [実行モード選択]
  │    │
  │    └─ モード（normal/continue/resume） + 権限スキップ → [起動]
  │
  └─→ [ツール起動]
       │
       ├─ type="path" → [絶対パスで実行]
       ├─ type="bunx" → [bunx経由で実行]
       └─ type="command" → [PATH解決 → 実行]
```

### 3.2 セッション保存フロー

```text
[ツール終了]
  │
  └─→ [SessionData作成]
       │
       ├─ lastWorktreePath
       ├─ lastBranch
       ├─ timestamp ← Date.now()
       ├─ repositoryRoot
       └─ lastUsedTool ← 選択したツールID
       │
       └─→ [saveSession()]
            │
            └─→ ~/.config/claude-worktree/sessions/{repo}_{hash}.json
```

## 4. バリデーション仕様

### 4.1 ToolsConfig検証

```typescript
function validateToolsConfig(config: any): ToolsConfig {
  // 1. version検証
  if (!config.version || typeof config.version !== 'string') {
    throw new Error('version is required and must be a string');
  }

  // 2. customTools検証
  if (!Array.isArray(config.customTools)) {
    throw new Error('customTools must be an array');
  }

  // 3. 各CustomAIToolを検証
  const seenIds = new Set<string>();
  for (const tool of config.customTools) {
    validateCustomAITool(tool);

    // ID重複チェック
    if (seenIds.has(tool.id)) {
      throw new Error(`Duplicate tool ID: ${tool.id}`);
    }
    seenIds.add(tool.id);
  }

  return config as ToolsConfig;
}
```

### 4.2 CustomAITool検証

```typescript
function validateCustomAITool(tool: any): void {
  // 必須フィールド
  const required = ['id', 'displayName', 'type', 'command', 'modeArgs'];
  for (const field of required) {
    if (!tool[field]) {
      throw new Error(`${field} is required for tool`);
    }
  }

  // type検証
  const validTypes = ['path', 'bunx', 'command'];
  if (!validTypes.includes(tool.type)) {
    throw new Error(`Invalid type: ${tool.type}. Must be one of: ${validTypes.join(', ')}`);
  }

  // id形式検証
  if (!/^[a-z0-9-]+$/.test(tool.id)) {
    throw new Error(`Invalid id format: ${tool.id}. Must match ^[a-z0-9-]+$`);
  }

  // command検証（type別）
  if (tool.type === 'path' && !path.isAbsolute(tool.command)) {
    throw new Error(`command must be an absolute path for type="path": ${tool.command}`);
  }

  // modeArgs検証
  if (!tool.modeArgs.normal && !tool.modeArgs.continue && !tool.modeArgs.resume) {
    throw new Error('modeArgs must have at least one mode defined');
  }
}
```

## 5. 永続化仕様

### 5.1 設定ファイルパス

**パス**: `~/.claude-worktree/tools.json`

**フォーマット**: JSON（UTF-8、インデント2スペース）

**例**:
```json
{
  "version": "1.0.0",
  "customTools": [
    {
      "id": "my-ai",
      "displayName": "My AI Tool",
      "icon": "🤖",
      "type": "bunx",
      "command": "@my-org/ai-tool@latest",
      "defaultArgs": ["--verbose"],
      "modeArgs": {
        "normal": [],
        "continue": ["-c"],
        "resume": ["-r"]
      },
      "permissionSkipArgs": ["--yes"],
      "env": {
        "MY_API_KEY": "sk-..."
      }
    }
  ]
}
```

### 5.2 セッションファイルパス

**パス**: `~/.config/claude-worktree/sessions/{repoName}_{repoHash}.json`

**フォーマット**: JSON（UTF-8、インデント2スペース）

**例**:
```json
{
  "lastWorktreePath": "/path/to/worktree",
  "lastBranch": "feature/my-feature",
  "timestamp": 1698765432000,
  "repositoryRoot": "/path/to/repo",
  "lastUsedTool": "my-ai"
}
```

## 6. まとめ

### 主要エンティティ

1. **ToolsConfig**: 設定ファイルのルート
2. **CustomAITool**: カスタムツール定義
3. **ModeArgs**: モード別引数
4. **SessionData**: セッション情報（拡張）
5. **AIToolConfig**: ビルトイン+カスタムの統合（内部）

### 検証フロー

設定読み込み → JSON解析 → 型検証 → id重複チェック → 使用可能

### 永続化

- 設定: `~/.claude-worktree/tools.json`
- セッション: `~/.config/claude-worktree/sessions/*.json`
