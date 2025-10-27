# クイックスタート: 一括ブランチマージ機能開発

**仕様ID**: `SPEC-ee33ca26` | **日付**: 2025-10-27
**対象**: 本機能の開発者

## 概要

このガイドでは、一括ブランチマージ機能の開発を開始するための最小限の手順を説明します。

## 前提条件

既存の開発環境がセットアップ済みであること：
- ✅ Bun 1.0+ インストール済み
- ✅ リポジトリクローン済み
- ✅ `bun install` 実行済み

## セットアップ（追加なし）

**本機能では新規の依存関係は不要です**。既存の環境で開発できます。

既存の依存関係：
- TypeScript 5.8+
- Ink.js 6.3+
- execa 9.6+
- Vitest 2.1+

## 開発ワークフロー（TDDサイクル）

### Step 1: テスト作成（Red）

新規関数を実装する前に、必ずテストを作成します。

**例**: `src/git.ts` に `mergeFromBranch` 関数を追加する場合

1. `tests/unit/git.test.ts` を開く
2. テストケースを追加:

```typescript
import { describe, it, expect, vi } from 'vitest';
import { mergeFromBranch } from '../src/git.js';
import { execa } from 'execa';

vi.mock('execa');

describe('mergeFromBranch', () => {
  it('should execute git merge command in worktree', async () => {
    const mockExeca = vi.mocked(execa);
    mockExeca.mockResolvedValue({ stdout: '', stderr: '', exitCode: 0 } as any);

    await mergeFromBranch('/path/to/worktree', 'main');

    expect(mockExeca).toHaveBeenCalledWith(
      'git',
      ['merge', 'main'],
      { cwd: '/path/to/worktree' }
    );
  });

  it('should throw error when merge fails', async () => {
    const mockExeca = vi.mocked(execa);
    mockExeca.mockRejectedValue(new Error('Merge conflict'));

    await expect(mergeFromBranch('/path/to/worktree', 'main'))
      .rejects.toThrow('Failed to merge');
  });
});
```

3. テスト実行（失敗することを確認）:

```bash
bun run test tests/unit/git.test.ts
```

期待結果: **FAIL** (関数がまだ実装されていないため)

### Step 2: ユーザー承認

テストケースが仕様を正しく反映しているか確認します。

- [ ] テストが機能要件をカバーしているか
- [ ] エッジケースを含んでいるか
- [ ] エラーハンドリングをテストしているか

### Step 3: テスト実行（Fail確認）

```bash
bun run test
```

全てのテストが失敗することを確認します（Red状態）。

### Step 4: 実装（Green）

テストがパスするように実装します。

**例**: `src/git.ts` に関数実装

```typescript
export async function mergeFromBranch(
  worktreePath: string,
  sourceBranch: string
): Promise<void> {
  try {
    await execa("git", ["merge", sourceBranch], { cwd: worktreePath });
  } catch (error) {
    throw new GitError(`Failed to merge from ${sourceBranch}`, error);
  }
}
```

テスト実行:

```bash
bun run test tests/unit/git.test.ts
```

期待結果: **PASS** (Green状態)

### Step 5: リファクタリング（Refactor）

コードの品質を改善します。

- 重複コード削減
- 関数の分割
- コメント追加

リファクタリング後も必ずテスト実行:

```bash
bun run test
```

全テストがパスすることを確認します。

### Step 6: コミット＆プッシュ

```bash
git add .
git commit -m "feat: add mergeFromBranch function with tests"
git push origin feature/git-pull-command
```

## よくある操作

### テスト実行

```bash
# 全テスト実行
bun run test

# 特定のテストファイル実行
bun run test tests/unit/git.test.ts

# Watch モード（変更時に自動実行）
bun run test:watch

# カバレッジ測定
bun run test:coverage
```

### ビルド

```bash
# TypeScriptコンパイル
bun run build

# Watch モード（変更時に自動ビルド）
bun run dev
```

### Lint & Type Check

```bash
# ESLint実行
bun run lint

# Type Check
bun run type-check

# Prettier フォーマット確認
bun run format:check

# Prettier フォーマット適用
bun run format
```

### ローカル実行

```bash
# ビルド後に実行
bun run build
bunx .

# または
bun run start
```

### デバッグ

#### Console.log デバッグ

```typescript
// 開発中のデバッグ出力
console.error('[DEBUG] currentBranch:', currentBranch);
console.error('[DEBUG] progress:', JSON.stringify(progress, null, 2));
```

**注意**: `console.log` ではなく `console.error` を使用（stdoutを汚さない）

#### VSCode デバッガ

`.vscode/launch.json` 追加:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "node",
      "request": "launch",
      "name": "Debug Vitest Test",
      "runtimeExecutable": "bun",
      "runtimeArgs": ["test", "${file}"],
      "console": "integratedTerminal"
    }
  ]
}
```

## トラブルシューティング

### 問題1: テストが通らない（execa モック関連）

**症状**:
```
Error: Cannot find module 'execa'
```

**解決策**:
```typescript
// vi.mock() を正しく配置
import { vi } from 'vitest';
import { execa } from 'execa';

vi.mock('execa'); // import文の直後

describe('...', () => {
  const mockExeca = vi.mocked(execa);
  // ...
});
```

### 問題2: Ink.js コンポーネントのテストエラー

**症状**:
```
Error: Cannot render outside of Ink context
```

**解決策**:
```typescript
import { render } from 'ink-testing-library';

// 正しいレンダリング
const { lastFrame } = render(<YourComponent />);
expect(lastFrame()).toContain('expected text');
```

### 問題3: Worktree パス問題

**症状**:
```
Error: Worktree path does not exist
```

**解決策**:
統合テストでは実際のgitリポジトリを作成:

```typescript
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execa } from 'execa';

let tempDir: string;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'git-test-'));
  await execa('git', ['init'], { cwd: tempDir });
  await execa('git', ['config', 'user.name', 'Test'], { cwd: tempDir });
  await execa('git', ['config', 'user.email', 'test@example.com'], { cwd: tempDir });
});

afterEach(async () => {
  await rm(tempDir, { recursive: true, force: true });
});
```

### 問題4: コンフリクトの再現

**統合テストでコンフリクトを作成**:

```typescript
// mainブランチでfile.txtに"A"を書き込み、コミット
await writeFile(join(tempDir, 'file.txt'), 'A');
await execa('git', ['add', '.'], { cwd: tempDir });
await execa('git', ['commit', '-m', 'Add A'], { cwd: tempDir });

// featureブランチを作成
await execa('git', ['checkout', '-b', 'feature'], { cwd: tempDir });

// featureブランチでfile.txtに"B"を書き込み、コミット
await writeFile(join(tempDir, 'file.txt'), 'B');
await execa('git', ['add', '.'], { cwd: tempDir });
await execa('git', ['commit', '-m', 'Add B'], { cwd: tempDir });

// mainブランチに戻る
await execa('git', ['checkout', 'main'], { cwd: tempDir });

// mainブランチでfile.txtに"C"を書き込み、コミット
await writeFile(join(tempDir, 'file.txt'), 'C');
await execa('git', ['add', '.'], { cwd: tempDir });
await execa('git', ['commit', '-m', 'Add C'], { cwd: tempDir });

// featureブランチにmainをマージ → コンフリクト発生
await execa('git', ['checkout', 'feature'], { cwd: tempDir });
await expect(execa('git', ['merge', 'main'], { cwd: tempDir }))
  .rejects.toThrow(); // コンフリクトでエラー
```

## ファイル構成

### 新規作成ファイル

```
src/
├── services/
│   └── BatchMergeService.ts          # [新規] サービス層
├── ui/
│   ├── components/
│   │   ├── screens/
│   │   │   ├── BatchMergeProgressScreen.tsx   # [新規] 進捗画面
│   │   │   └── BatchMergeResultScreen.tsx     # [新規] 結果画面
│   │   └── parts/
│   │       ├── ProgressBar.tsx        # [新規] 進捗バー
│   │       └── MergeStatusList.tsx    # [新規] ステータスリスト
│   └── hooks/
│       └── useBatchMerge.ts           # [新規] カスタムフック

tests/
├── unit/
│   └── services/
│       └── BatchMergeService.test.ts  # [新規]
├── integration/
│   └── batch-merge.test.ts            # [新規]
└── e2e/
    └── batch-merge-workflow.test.ts   # [新規]
```

### 拡張ファイル

```
src/
├── git.ts                             # [拡張] マージ関数追加
└── ui/
    ├── types.ts                       # [拡張] 型追加
    └── components/
        ├── screens/
        │   └── BranchListScreen.tsx   # [拡張] 'p'キー追加
        └── App.tsx                    # [拡張] 画面遷移追加

tests/
└── unit/
    └── git.test.ts                    # [拡張] テスト追加
```

## 次のステップ

1. ✅ クイックスタート確認
2. ⏩ `/speckit.tasks` でタスク生成
3. ⏭️ `/speckit.implement` で実装開始

## 参考資料

- [仕様書](./spec.md)
- [実装計画](./plan.md)
- [調査結果](./research.md)
- [データモデル](./data-model.md)
- [Vitest公式ドキュメント](https://vitest.dev/)
- [Ink.js公式ドキュメント](https://github.com/vadimdemedes/ink)
