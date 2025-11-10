# データモデル: Web UI機能の追加

**日付**: 2025-11-10
**仕様ID**: SPEC-d5e56259
**関連ドキュメント**: [spec.md](./spec.md), [plan.md](./plan.md), [research.md](./research.md)

## 概要

このドキュメントは、Web UI機能で使用する主要なデータエンティティとその関係を定義します。実装詳細（クラス構造、メソッド）は含まず、ビジネスドメインの観点からデータ構造を記述します。

## エンティティ定義

### 1. Branch（ブランチ）

Gitブランチを表すエンティティ。ローカルブランチとリモートブランチの両方を含む。

#### 属性

| 属性名 | 型 | 説明 | 必須 | バリデーション |
|--------|-----|------|------|----------------|
| name | string | ブランチ名（例: `feature/webui`, `origin/main`） | ✓ | 1-255文字、Git命名規則準拠 |
| type | 'local' \| 'remote' | ブランチタイプ | ✓ | 'local' または 'remote' |
| commitHash | string | 最新コミットのハッシュ（短縮形） | ✓ | 7文字のHEX |
| commitMessage | string | 最新コミットメッセージ | ○ | 最大500文字 |
| author | string | 最新コミットの著者 | ○ | 最大100文字 |
| commitDate | ISO8601 | 最新コミット日時 | ○ | ISO8601形式 |
| mergeStatus | 'unmerged' \| 'merged' \| 'unknown' | マージ状態 | ✓ | - |
| worktreePath | string \| null | 関連するWorktreeのパス（存在しない場合はnull） | ✓ | 絶対パス |
| divergence | object \| null | ブランチの差分情報 | ○ | 下記参照 |

#### divergence属性の構造

```typescript
{
  ahead: number;     // ローカルがリモートより進んでいるコミット数
  behind: number;    // ローカルがリモートより遅れているコミット数
  upToDate: boolean; // 差分がない場合true
}
```

#### 関係

- **Branch → Worktree**: 1対0..1（1つのブランチは最大1つのWorktreeを持つ）
- **Branch → Branch**: リモートブランチとローカルブランチの対応関係（例: `main` ↔ `origin/main`）

#### バリデーションルール

- **BR-001**: ブランチ名は空文字列不可
- **BR-002**: typeは'local'または'remote'のみ
- **BR-003**: commitHashは7文字の16進数
- **BR-004**: worktreePathが存在する場合、有効なディレクトリパスである必要がある

#### 状態遷移

```
[新規作成] → [unmerged] → [merged]
                ↓
           [Worktree関連付け] → worktreePath設定
```

---

### 2. Worktree（ワークツリー）

Gitワークツリーを表すエンティティ。特定のブランチに対応する作業ディレクトリ。

#### 属性

| 属性名 | 型 | 説明 | 必須 | バリデーション |
|--------|-----|------|------|----------------|
| path | string | ワークツリーのファイルシステムパス | ✓ | 絶対パス、ディレクトリ存在確認 |
| branchName | string | 関連するブランチ名 | ✓ | 1-255文字 |
| head | string | HEADコミットハッシュ | ✓ | 7文字のHEX |
| isLocked | boolean | ロック状態（他の操作で使用中） | ✓ | - |
| isPrunable | boolean | 削除可能状態（マージ済み等） | ✓ | - |
| createdAt | ISO8601 | 作成日時 | ○ | ISO8601形式 |
| lastAccessedAt | ISO8601 | 最終アクセス日時 | ○ | ISO8601形式 |

#### 関係

- **Worktree → Branch**: 1対1（1つのWorktreeは必ず1つのブランチに対応）
- **Worktree → AIToolSession**: 1対0..*（1つのWorktreeで複数のAIツールセッションを実行可能）

#### バリデーションルール

- **WT-001**: pathは絶対パス形式
- **WT-002**: pathは既存のディレクトリを指す
- **WT-003**: branchNameは対応するBranchエンティティが存在
- **WT-004**: 同じpathを持つWorktreeは重複不可（ユニーク制約）

#### 状態遷移

```
[未作成] → [作成中] → [アクティブ] → [アイドル]
                           ↓
                      [ロック中] → [アクティブ]
                           ↓
                      [削除可能] → [削除済み]
```

---

### 3. AIToolSession（AIツールセッション）

AI Tool（Claude Code、Codex CLI、カスタムツール）の実行セッションを表すエンティティ。

#### 属性

| 属性名 | 型 | 説明 | 必須 | バリデーション |
|--------|-----|------|------|----------------|
| sessionId | string | セッションの一意識別子（UUID v4） | ✓ | UUID v4形式 |
| toolType | 'claude-code' \| 'codex-cli' \| 'custom' | 使用するツールタイプ | ✓ | 3種類のいずれか |
| toolName | string | ツール名（customの場合に使用） | ○ | 最大100文字 |
| mode | 'normal' \| 'continue' \| 'resume' | 実行モード | ✓ | 3種類のいずれか |
| worktreePath | string | 実行するWorktreeパス | ✓ | 絶対パス |
| ptyPid | number \| null | PTYプロセスID（実行中のみ） | ✓ | 正の整数 |
| websocketId | string \| null | WebSocket接続ID（接続中のみ） | ✓ | ランダム文字列 |
| status | 'pending' \| 'running' \| 'completed' \| 'failed' | セッション状態 | ✓ | 4種類のいずれか |
| startedAt | ISO8601 | 開始日時 | ✓ | ISO8601形式 |
| endedAt | ISO8601 \| null | 終了日時（実行中はnull） | ✓ | ISO8601形式 |
| exitCode | number \| null | 終了コード（完了/失敗時のみ） | ✓ | 整数 |
| outputLog | string \| null | 出力ログ（保存する場合） | ○ | 最大10MB |
| errorMessage | string \| null | エラーメッセージ（失敗時のみ） | ○ | 最大1000文字 |

#### 関係

- **AIToolSession → Worktree**: 多対1（複数のセッションが1つのWorktreeで実行可能）
- **AIToolSession → CustomAITool**: 多対0..1（customタイプの場合、CustomAIToolエンティティを参照）

#### バリデーションルール

- **ATS-001**: sessionIdはUUID v4形式
- **ATS-002**: toolTypeが'custom'の場合、toolNameは必須
- **ATS-003**: statusが'running'の場合、ptyPidとwebsocketIdは必須
- **ATS-004**: statusが'completed'または'failed'の場合、endedAtとexitCodeは必須
- **ATS-005**: statusが'failed'の場合、errorMessageは必須

#### 状態遷移

```
[pending] → [running] → [completed]
                ↓
             [failed]
```

**遷移条件**:
- pending → running: PTY起動成功、WebSocket接続確立
- running → completed: PTYプロセス正常終了（exitCode=0）
- running → failed: PTYプロセス異常終了（exitCode≠0）またはWebSocket切断

---

### 4. CustomAITool（カスタムAIツール）

ユーザーが定義したカスタムAI Toolの設定を表すエンティティ。

#### 属性

| 属性名 | 型 | 説明 | 必須 | バリデーション |
|--------|-----|------|------|----------------|
| id | string | ツールの一意識別子（自動生成） | ✓ | UUID v4形式 |
| name | string | ツール表示名 | ✓ | 1-100文字、重複不可 |
| command | string | 実行コマンドまたはパス | ✓ | 1-500文字 |
| executionType | 'path' \| 'bunx' \| 'command' | 実行タイプ | ✓ | 3種類のいずれか |
| defaultArgs | string[] | デフォルト引数リスト | ○ | 各要素1-200文字 |
| env | Record<string, string> | 環境変数 | ○ | キー: 1-100文字、値: 1-500文字 |
| description | string | ツールの説明 | ○ | 最大500文字 |
| createdAt | ISO8601 | 作成日時 | ✓ | ISO8601形式 |
| updatedAt | ISO8601 | 最終更新日時 | ✓ | ISO8601形式 |

#### 関係

- **CustomAITool → AIToolSession**: 1対0..*（1つのカスタムツールから複数のセッションを起動可能）

#### バリデーションルール

- **CAT-001**: idはUUID v4形式
- **CAT-002**: nameは重複不可（ユニーク制約）
- **CAT-003**: executionTypeが'path'の場合、commandは絶対パスまたは実行可能ファイル名
- **CAT-004**: executionTypeが'bunx'の場合、commandはパッケージ名
- **CAT-005**: executionTypeが'command'の場合、commandは実行可能コマンド
- **CAT-006**: env のキーは英数字とアンダースコアのみ

#### 永続化

- **保存先**: `~/.claude-worktree/tools.json`
- **フォーマット**: JSON配列

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "name": "My Custom Tool",
    "command": "/usr/local/bin/my-tool",
    "executionType": "path",
    "defaultArgs": ["--verbose"],
    "env": { "DEBUG": "true" },
    "description": "My custom development tool",
    "createdAt": "2025-11-10T00:00:00Z",
    "updatedAt": "2025-11-10T00:00:00Z"
  }
]
```

---

## エンティティ関係図

```
┌─────────────────┐
│     Branch      │
│                 │
│ - name          │
│ - type          │
│ - commitHash    │
│ - mergeStatus   │
│ - worktreePath  │──────┐
└─────────────────┘      │
                         │ 1:0..1
                         ↓
┌─────────────────┐  ┌─────────────────┐
│ AIToolSession   │  │    Worktree     │
│                 │  │                 │
│ - sessionId     │  │ - path          │
│ - toolType      │  │ - branchName    │
│ - mode          │  │ - isLocked      │
│ - status        │  │ - isPrunable    │
│ - ptyPid        │  └─────────────────┘
│ - websocketId   │         │
└─────────────────┘         │ 1:0..*
        │                   │
        │ *:0..1            ↓
        ↓
┌─────────────────┐
│ CustomAITool    │
│                 │
│ - id            │
│ - name          │
│ - command       │
│ - executionType │
└─────────────────┘
```

**関係の説明**:
- Branch → Worktree: 1対0..1（1つのブランチは最大1つのWorktreeを持つ）
- Worktree → AIToolSession: 1対0..*（1つのWorktreeで複数のセッションを実行可能）
- CustomAITool → AIToolSession: 1対0..*（1つのカスタムツールから複数のセッションを起動可能）

---

## データフロー

### 1. Worktree作成フロー

```
1. ユーザーがBranchを選択
2. システムがBranch.worktreePathをチェック
   - 既存: エラー表示
   - 未存在: 続行
3. システムがWorktreeを作成
4. Branch.worktreePathを更新
5. Worktreeエンティティを永続化
```

### 2. AI Tool起動フロー

```
1. ユーザーがWorktreeとAI Toolを選択
2. システムがAIToolSessionを作成（status='pending'）
3. PTYプロセスを起動
4. WebSocket接続確立
5. AIToolSessionを更新（status='running', ptyPid, websocketId）
6. PTY出力をWebSocketで転送
7. PTYプロセス終了
8. AIToolSessionを更新（status='completed/failed', endedAt, exitCode）
```

### 3. 設定管理フロー

```
1. ユーザーが設定画面にアクセス
2. システムが`~/.claude-worktree/tools.json`を読み込み
3. CustomAIToolエンティティリストを表示
4. ユーザーがCustomAIToolを追加/編集/削除
5. システムがバリデーション実行
6. システムが`tools.json`に保存
7. CustomAIToolエンティティを更新
```

---

## データストレージ

### REST API（メモリ内）

- **Branch**: Git操作の結果を都度取得（永続化不要）
- **Worktree**: Git操作の結果を都度取得（永続化不要）
- **AIToolSession**: メモリ内Map（`sessionId` → `AIToolSession`）、再起動で消失

### ファイルシステム

- **CustomAITool**: `~/.claude-worktree/tools.json`（JSON配列）
- **セッション履歴**: `~/.claude-worktree/sessions.json`（オプション、将来実装）

---

## 次のステップ

1. ✅ データモデル定義完了
2. ⏭️ REST API契約定義（contracts/rest-api.yaml）
3. ⏭️ WebSocketプロトコル定義（contracts/websocket.md）
4. ⏭️ クイックスタートガイド作成（quickstart.md）
