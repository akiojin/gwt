# データモデル: main/develop保護強化

## 1. SelectedBranchState (更新)

| フィールド | 型 | 説明 |
| --- | --- | --- |
| `name` | `string` | ローカルブランチ名 |
| `displayName` | `string` | UI表示用（リモートの場合は`origin/xxx`） |
| `branchType` | `'local' | 'remote'` | 取得元を表す既存フィールド |
| `branchCategory` | `'main' | 'develop' | 'feature' | 'hotfix' | 'release' | 'other'` | 新規追加。`BranchInfo.branchType`をそのまま保持し、保護判定に使用する |
| `remoteBranch?` | `string` | リモートブランチのフル名 |

### バリデーション

- `branchCategory` が `main` / `develop` / `other` 等となるが、保護対象判定は `PROTECTED_BRANCHES`（`main`/`develop`/`master`）との比較で行う。

## 2. ProtectedBranchPolicy (新規概念)

- 実体は`src/worktree.ts`の`PROTECTED_BRANCHES`配列を再利用。
- UIレイヤーでは `new Set(PROTECTED_BRANCHES)` を生成し、クライアント側分岐に用いる。

## 3. UI状態 (補足)

- `App.tsx` の `cleanupFooterMessage` を使って保護ブランチ選択時の警告文を表示。構造は `{ text: string; color?: 'cyan' | 'green' | 'yellow' | 'red' }`。
- 保護ブランチを選択した際は`setCleanupFooterMessage`で黄色メッセージを設定し、別のブランチを選ぶときにリセットする。
