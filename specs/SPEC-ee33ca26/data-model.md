# データモデル: 一括ブランチマージ機能

**仕様ID**: `SPEC-ee33ca26` | **日付**: 2025-10-27
**目的**: 一括マージ機能で使用するデータ構造と型定義

## 概要

本ドキュメントでは、一括ブランチマージ機能で使用する主要なデータモデルを定義します。全ての型は `src/ui/types.ts` に追加されます。

## 主要エンティティ

### 1. BatchMergeConfig

**目的**: 一括マージの実行設定

**属性**:

| 属性名 | 型 | 必須 | 説明 |
|--------|---|------|------|
| `sourceBranch` | `string` | ✅ | マージ元ブランチ名（例: "main", "develop"） |
| `targetBranches` | `string[]` | ✅ | マージ対象ブランチ名リスト（例: ["feature/a", "hotfix/b"]） |
| `dryRun` | `boolean` | ✅ | ドライランモード（true: シミュレーションのみ, false: 実マージ） |
| `autoPush` | `boolean` | ✅ | 自動プッシュ（true: マージ成功後に自動push, false: pushしない） |
| `remote` | `string` | ❌ | リモート名（デフォルト: "origin"） |

**検証ルール**:
- `sourceBranch` は空文字列不可
- `targetBranches` は空配列不可、重複不可
- `dryRun` と `autoPush` は同時にtrueにできない（ドライランではpush不可）

**TypeScript型定義**:
```typescript
export interface BatchMergeConfig {
  sourceBranch: string;
  targetBranches: string[];
  dryRun: boolean;
  autoPush: boolean;
  remote?: string; // デフォルト: "origin"
}
```

**使用例**:
```typescript
const config: BatchMergeConfig = {
  sourceBranch: "main",
  targetBranches: ["feature/login", "feature/dashboard", "hotfix/bug-123"],
  dryRun: false,
  autoPush: true,
  remote: "origin"
};
```

---

### 2. BatchMergeProgress

**目的**: リアルタイム進捗情報

**属性**:

| 属性名 | 型 | 必須 | 説明 |
|--------|---|------|------|
| `currentBranch` | `string` | ✅ | 現在処理中のブランチ名（例: "feature/login"） |
| `currentIndex` | `number` | ✅ | 現在のインデックス（0ベース） |
| `totalBranches` | `number` | ✅ | 総ブランチ数 |
| `percentage` | `number` | ✅ | 進捗率（0-100の整数） |
| `elapsedSeconds` | `number` | ✅ | 経過時間（秒） |
| `estimatedRemainingSeconds` | `number` | ❌ | 推定残り時間（秒、計算可能な場合のみ） |
| `currentPhase` | `MergePhase` | ✅ | 現在のフェーズ（fetch/worktree/merge/push） |

**状態遷移**:
- 初期状態: `currentIndex: 0, percentage: 0`
- 進行中: `currentIndex: 1...N-1, percentage: 1...99`
- 完了: `currentIndex: N, percentage: 100`

**TypeScript型定義**:
```typescript
export type MergePhase = "fetch" | "worktree" | "merge" | "push" | "cleanup";

export interface BatchMergeProgress {
  currentBranch: string;
  currentIndex: number;
  totalBranches: number;
  percentage: number; // 0-100
  elapsedSeconds: number;
  estimatedRemainingSeconds?: number;
  currentPhase: MergePhase;
}
```

**計算式**:
```typescript
percentage = Math.floor((currentIndex / totalBranches) * 100);
estimatedRemainingSeconds = (elapsedSeconds / currentIndex) * (totalBranches - currentIndex);
```

---

### 3. BranchMergeStatus

**目的**: 各ブランチの個別マージ結果

**属性**:

| 属性名 | 型 | 必須 | 説明 |
|--------|---|------|------|
| `branchName` | `string` | ✅ | ブランチ名 |
| `status` | `MergeStatus` | ✅ | マージステータス（success/skipped/failed） |
| `error` | `string` | ❌ | エラーメッセージ（失敗時のみ） |
| `conflictFiles` | `string[]` | ❌ | コンフリクトファイルリスト（skipped時のみ） |
| `pushStatus` | `PushStatus` | ❌ | プッシュ結果（autoPush有効時のみ） |
| `worktreeCreated` | `boolean` | ✅ | worktreeを新規作成したか |
| `durationSeconds` | `number` | ✅ | 処理時間（秒） |

**ステータス定義**:

| Status | 説明 | 次のアクション |
|--------|------|---------------|
| `success` | マージ成功 | （autoPush有効時はpush） |
| `skipped` | コンフリクトでスキップ | 手動解決を促す |
| `failed` | その他のエラーで失敗 | エラー内容を表示 |

**TypeScript型定義**:
```typescript
export type MergeStatus = "success" | "skipped" | "failed";
export type PushStatus = "success" | "failed" | "not_executed";

export interface BranchMergeStatus {
  branchName: string;
  status: MergeStatus;
  error?: string;
  conflictFiles?: string[];
  pushStatus?: PushStatus;
  worktreeCreated: boolean;
  durationSeconds: number;
}
```

**使用例**:
```typescript
// 成功例
const successStatus: BranchMergeStatus = {
  branchName: "feature/login",
  status: "success",
  pushStatus: "success",
  worktreeCreated: false,
  durationSeconds: 8.5
};

// スキップ例（コンフリクト）
const skippedStatus: BranchMergeStatus = {
  branchName: "feature/dashboard",
  status: "skipped",
  conflictFiles: ["src/components/Header.tsx", "src/utils/api.ts"],
  worktreeCreated: true,
  durationSeconds: 3.2
};

// 失敗例
const failedStatus: BranchMergeStatus = {
  branchName: "hotfix/bug-123",
  status: "failed",
  error: "Failed to create worktree: Disk full",
  worktreeCreated: false,
  durationSeconds: 1.0
};
```

---

### 4. BatchMergeResult

**目的**: 一括マージの最終結果サマリー

**属性**:

| 属性名 | 型 | 必須 | 説明 |
|--------|---|------|------|
| `statuses` | `BranchMergeStatus[]` | ✅ | 全ブランチのマージ結果リスト |
| `summary` | `BatchMergeSummary` | ✅ | サマリー統計 |
| `totalDurationSeconds` | `number` | ✅ | 総処理時間（秒） |
| `cancelled` | `boolean` | ✅ | ユーザーによるキャンセルフラグ |
| `config` | `BatchMergeConfig` | ✅ | 実行時の設定（記録用） |

**TypeScript型定義**:
```typescript
export interface BatchMergeSummary {
  totalCount: number;      // 総ブランチ数
  successCount: number;    // 成功数
  skippedCount: number;    // スキップ数（コンフリクト）
  failedCount: number;     // 失敗数
  pushedCount: number;     // プッシュ成功数（autoPush有効時）
  pushFailedCount: number; // プッシュ失敗数（autoPush有効時）
}

export interface BatchMergeResult {
  statuses: BranchMergeStatus[];
  summary: BatchMergeSummary;
  totalDurationSeconds: number;
  cancelled: boolean;
  config: BatchMergeConfig;
}
```

**計算式**:
```typescript
summary.totalCount = statuses.length;
summary.successCount = statuses.filter(s => s.status === "success").length;
summary.skippedCount = statuses.filter(s => s.status === "skipped").length;
summary.failedCount = statuses.filter(s => s.status === "failed").length;
summary.pushedCount = statuses.filter(s => s.pushStatus === "success").length;
summary.pushFailedCount = statuses.filter(s => s.pushStatus === "failed").length;
```

**使用例**:
```typescript
const result: BatchMergeResult = {
  statuses: [
    { branchName: "feature/a", status: "success", worktreeCreated: false, durationSeconds: 5 },
    { branchName: "feature/b", status: "skipped", conflictFiles: ["file.ts"], worktreeCreated: true, durationSeconds: 3 },
    { branchName: "feature/c", status: "success", pushStatus: "success", worktreeCreated: false, durationSeconds: 8 }
  ],
  summary: {
    totalCount: 3,
    successCount: 2,
    skippedCount: 1,
    failedCount: 0,
    pushedCount: 2,
    pushFailedCount: 0
  },
  totalDurationSeconds: 16,
  cancelled: false,
  config: { sourceBranch: "main", targetBranches: ["feature/a", "feature/b", "feature/c"], dryRun: false, autoPush: true }
};
```

---

## エンティティ関係図

```
BatchMergeConfig
       |
       | (入力)
       v
BatchMergeService.executeBatchMerge()
       |
       | (進捗callback)
       v
BatchMergeProgress -----> UI表示（リアルタイム）
       |
       | (各ブランチ処理)
       v
BranchMergeStatus (x N個)
       |
       | (集約)
       v
BatchMergeResult -----> UI表示（最終結果）
```

## データフロー

### 実行フロー

1. **初期化**:
   ```typescript
   const config = buildBatchMergeConfig(branches, options);
   ```

2. **進捗更新** (N回):
   ```typescript
   onProgress?.({
     currentBranch: "feature/a",
     currentIndex: 1,
     totalBranches: 5,
     percentage: 20,
     elapsedSeconds: 10,
     currentPhase: "merge"
   });
   ```

3. **結果収集**:
   ```typescript
   const statuses: BranchMergeStatus[] = [];
   for (const branch of targetBranches) {
     const status = await mergeBranch(branch);
     statuses.push(status);
   }
   ```

4. **最終結果**:
   ```typescript
   const result: BatchMergeResult = {
     statuses,
     summary: calculateSummary(statuses),
     totalDurationSeconds: Date.now() - startTime,
     cancelled: false,
     config
   };
   return result;
   ```

## 検証ルール

### Config検証

```typescript
function validateConfig(config: BatchMergeConfig): void {
  if (!config.sourceBranch) throw new Error("sourceBranch is required");
  if (config.targetBranches.length === 0) throw new Error("targetBranches cannot be empty");
  if (config.dryRun && config.autoPush) throw new Error("Cannot auto-push in dry-run mode");

  const uniqueBranches = new Set(config.targetBranches);
  if (uniqueBranches.size !== config.targetBranches.length) {
    throw new Error("targetBranches contains duplicates");
  }
}
```

### Progress検証

```typescript
function validateProgress(progress: BatchMergeProgress): void {
  if (progress.percentage < 0 || progress.percentage > 100) {
    throw new Error("Invalid percentage");
  }
  if (progress.currentIndex < 0 || progress.currentIndex > progress.totalBranches) {
    throw new Error("Invalid currentIndex");
  }
}
```

## 状態遷移

### MergeStatus遷移

```
[開始]
  |
  v
処理中 -----(成功)-----> success
  |
  |------(コンフリクト)-> skipped
  |
  |------(その他エラー)-> failed
```

### PushStatus遷移（autoPush有効時のみ）

```
MergeStatus: success
  |
  v
プッシュ実行 -----(成功)-----> pushStatus: success
  |
  |------(失敗)-----> pushStatus: failed
```

## 永続化

**該当なし** - 全てのデータはメモリ内で管理され、ファイルやDBへの保存は行いません。

画面表示後はユーザーが必要に応じてスクリーンショットやログを保存します。

## 拡張性

### 将来の拡張

1. **並列処理対応** (P4):
   ```typescript
   interface BatchMergeConfig {
     // ... 既存フィールド
     parallelism?: number; // 並列度（デフォルト: 1 = 順次）
   }
   ```

2. **ログ保存** (範囲外だが将来):
   ```typescript
   interface BatchMergeConfig {
     // ... 既存フィールド
     logFilePath?: string; // ログ保存先
   }
   ```

3. **カスタムマージメッセージ** (範囲外だが将来):
   ```typescript
   interface BatchMergeConfig {
     // ... 既存フィールド
     mergeMessage?: string; // カスタムマージコミットメッセージ
   }
   ```

## 型安全性

### TypeScript厳密モード対応

- `tsconfig.json` の `strict: true` に準拠
- `exactOptionalPropertyTypes: true` に準拠
- `noUncheckedIndexedAccess: true` に準拠

### Null安全性

- オプショナルフィールドは `?:` で明示
- undefined許容フィールドは `| undefined` で明示
- エラーメッセージは常に `string | undefined`

## 次のステップ

1. ✅ データモデル定義完了
2. ⏩ quickstart.md作成
3. ⏭️ src/ui/types.ts へ型追加
4. ⏭️ テストでの型使用
