# データモデル: カスタムコーディングエージェント登録機能

**仕様ID**: `SPEC-71f2742d` | **作成日**: 2026-01-26

## 1. 主要エンティティ

### 1.1 ToolsConfig

tools.json ファイル全体の構造を表す。

| 属性 | 型 | 必須 | 説明 |
|------|-----|------|------|
| version | String | Yes | スキーマバージョン（例: "1.0.0"） |
| customCodingAgents | Vec\<CustomCodingAgent\> | No | カスタムエージェント定義配列 |

**バリデーション**:
- version が未定義の場合、ファイル全体を無効とする
- version は semver 形式（\d+\.\d+\.\d+）

### 1.2 CustomCodingAgent

個々のカスタムエージェント定義。

| 属性 | 型 | 必須 | 説明 |
|------|-----|------|------|
| id | String | Yes | 一意識別子（英数字とハイフン） |
| displayName | String | Yes | UI表示名 |
| type | AgentType | Yes | 実行タイプ（command/path/bunx） |
| command | String | Yes | 実行コマンドまたはパス |
| defaultArgs | Vec\<String\> | No | デフォルト引数 |
| modeArgs | ModeArgs | No | モード別引数 |
| permissionSkipArgs | Vec\<String\> | No | パーミッションスキップ引数 |
| env | HashMap\<String, String\> | No | 環境変数 |
| models | Vec\<ModelDef\> | No | モデル定義一覧 |
| versionCommand | String | No | バージョン取得コマンド |

**バリデーション**:
- id: `^[a-zA-Z0-9-]+$` パターン
- type: "command" | "path" | "bunx" のいずれか
- 必須フィールド欠損時は該当エントリをスキップ

### 1.3 AgentType (enum)

エージェントの実行方式。

| バリアント | 説明 |
|-----------|------|
| Command | PATH 検索で実行 |
| Path | 絶対パスで実行 |
| Bunx | bunx 経由で実行 |

### 1.4 ModeArgs

実行モード別の追加引数。

| 属性 | 型 | 必須 | 説明 |
|------|-----|------|------|
| normal | Vec\<String\> | No | 通常モード引数 |
| continue | Vec\<String\> | No | Continue モード引数 |
| resume | Vec\<String\> | No | Resume モード引数 |

**注意**: 定義されていないモードは Wizard で非表示。

### 1.5 ModelDef

モデル選択肢の定義。

| 属性 | 型 | 必須 | 説明 |
|------|-----|------|------|
| id | String | Yes | モデル識別子 |
| label | String | Yes | UI表示ラベル |
| arg | String | Yes | コマンドライン引数 |

### 1.6 AgentEntry (統一エージェント表現)

ビルトインとカスタムを統一的に扱うための抽象。

| 属性 | 型 | 説明 |
|------|-----|------|
| id | String | 一意識別子 |
| displayName | String | 表示名 |
| color | Color | 表示色 |
| isBuiltin | bool | ビルトインフラグ |
| isInstalled | bool | インストール済みフラグ |
| models | Vec\<ModelOption\> | モデル一覧 |
| supportedModes | Vec\<ExecutionMode\> | サポートモード |

## 2. 関連図

```text
┌─────────────────────┐
│    ToolsConfig      │
│─────────────────────│
│ version: String     │
│ customCodingAgents  │──┐
└─────────────────────┘  │
                         │ 1..*
                         ▼
┌─────────────────────────────────┐
│       CustomCodingAgent         │
│─────────────────────────────────│
│ id: String                      │
│ displayName: String             │
│ type: AgentType                 │
│ command: String                 │
│ defaultArgs: Vec<String>        │
│ modeArgs: ModeArgs              │──┐
│ permissionSkipArgs: Vec<String> │  │
│ env: HashMap<String, String>    │  │
│ models: Vec<ModelDef>           │──┼─┐
│ versionCommand: Option<String>  │  │ │
└─────────────────────────────────┘  │ │
                                     │ │
         ┌───────────────────────────┘ │
         ▼                             │
┌─────────────────────┐                │
│      ModeArgs       │                │
│─────────────────────│                │
│ normal: Vec<String> │                │
│ continue: Vec<String│                │
│ resume: Vec<String> │                │
└─────────────────────┘                │
                                       │
         ┌─────────────────────────────┘
         ▼
┌─────────────────────┐
│      ModelDef       │
│─────────────────────│
│ id: String          │
│ label: String       │
│ arg: String         │
└─────────────────────┘

┌─────────────────────┐
│     AgentType       │
│─────────────────────│
│ ◇ Command           │
│ ◇ Path              │
│ ◇ Bunx              │
└─────────────────────┘
```

## 3. ファイル読み込みフロー

```text
Wizard 開始
    │
    ▼
┌─────────────────────────────┐
│ グローバル読み込み           │
│ ~/.gwt/tools.json           │
└─────────────────────────────┘
    │
    ▼
┌─────────────────────────────┐
│ ローカル読み込み             │
│ .gwt/tools.json             │
└─────────────────────────────┘
    │
    ▼
┌─────────────────────────────┐
│ マージ処理                   │
│ - 同一ID: ローカル優先       │
│ - 異なるID: 両方保持         │
└─────────────────────────────┘
    │
    ▼
┌─────────────────────────────┐
│ バリデーション               │
│ - version 必須チェック       │
│ - 必須フィールドチェック     │
│ - 無効エントリスキップ       │
└─────────────────────────────┘
    │
    ▼
┌─────────────────────────────┐
│ インストール状態チェック     │
│ - command 存在確認           │
│ - 非存在: グレーアウト       │
└─────────────────────────────┘
    │
    ▼
┌─────────────────────────────┐
│ 色自動割り当て               │
│ Blue → Red → White → Gray   │
└─────────────────────────────┘
    │
    ▼
Wizard に AgentEntry リストとして提供
```

## 4. 統合ポイント

### 4.1 既存 CodingAgent との関係

- 既存 `CodingAgent` enum は `BuiltinAgent` として扱う
- `AgentEntry` に変換してカスタムと統一
- 既存のビルトイン固有ロジック（models, npm_package 等）は保持

### 4.2 履歴保存との関係

- `AgentHistoryStore` の `agent_id` フィールドにカスタムIDを保存
- 既存の構造変更なし（String 型で対応済み）

### 4.3 セッション履歴との関係

- `ToolSessionEntry` の `tool_id` フィールドにカスタムIDを保存
- 既存の構造変更なし（String 型で対応済み）
