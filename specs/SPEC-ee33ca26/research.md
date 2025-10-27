# 技術調査: 一括ブランチマージ機能

**仕様ID**: `SPEC-ee33ca26` | **日付**: 2025-10-27
**目的**: 実装方法と技術選定のための調査

## 既存コードベース分析

### 現在の技術スタック

**確認済み**:
- **言語**: TypeScript 5.8+ (ES2022, ESNext modules)
- **ランタイム**: Bun 1.0+
- **UIフレームワーク**: Ink.js 6.3+ (React 19.2+ for CLI)
- **Git実行**: execa 9.6+
- **テスト**: Vitest 2.1+ + ink-testing-library 4.0+ + @testing-library/react 16.3+

### アーキテクチャパターン

**3層構造**:
1. **Repository層** (`src/repositories/*.repository.ts`)
   - 低レベルのgit/GitHub API操作
   - 例: `GitRepository.getCurrentBranch()`, `GitRepository.getBranches()`

2. **Service層** (`src/services/*.service.ts`)
   - ビジネスロジック
   - 例: `GitService.getAllBranches()` - Repository呼び出し + データ整形

3. **UI層** (`src/ui/`)
   - Ink.jsコンポーネント（Screens, Parts, Hooks）
   - 例: `BranchListScreen`, `useGitData`

### 統合ポイント

**既存モジュール**:
- `src/git.ts`: 直接git操作関数（`execa("git", ...)`）
- `src/worktree.ts`: worktree管理関数（作成、削除、一覧取得）
- `src/ui/components/screens/BranchListScreen.tsx`: ブランチ一覧UI（'p'キー追加対象）
- `src/ui/hooks/useGitData.ts`: git データフック（branches, worktrees, loading, error）

**既存の型定義** (`src/ui/types.ts`):
- `BranchInfo`, `BranchItem`, `WorktreeInfo`, `Statistics`
- 一括マージ用の型を追加する必要あり

## 技術決定

### 決定1: git merge実装方法

**選択肢**:

| オプション | 概要 | メリット | デメリット |
|----------|------|----------|----------|
| A. git.ts拡張 | 既存git.tsにマージ関数追加 | 既存パターン踏襲、統合容易 | git.tsが肥大化 |
| B. 新規MergeRepository | repositories/配下に新規作成 | 責任分離、テスト容易 | 過度な抽象化 |
| C. BatchMergeServiceに内包 | Service内でexeca直接実行 | シンプル、依存少ない | 低レベル操作の再利用不可 |

**決定**: **A. git.ts拡張**

**根拠**:
- 既存のgit.tsが既にgit操作の集約ポイント（`createBranch`, `deleteBranch`, `fetchAllRemotes`など）
- マージ操作も他の機能で再利用される可能性が高い
- プロジェクトの「シンプルさの極限追求」原則に合致
- git.tsは200行程度で、4-5関数追加しても管理可能

**追加する関数**:
```typescript
// git.ts に追加
export async function mergeFromBranch(worktreePath: string, sourceBranch: string): Promise<void>
export async function hasMergeConflict(worktreePath: string): Promise<boolean>
export async function abortMerge(worktreePath: string): Promise<void>
export async function getMergeStatus(worktreePath: string): Promise<{inProgress: boolean, hasConflict: boolean}>
```

### 決定2: 進捗表示のリアルタイム更新方法

**選択肢**:

| オプション | 概要 | メリット | デメリット |
|----------|------|----------|----------|
| A. React state + 直接更新 | setState()で進捗更新 | シンプル、Ink.js標準 | Service層とUI層が密結合 |
| B. Event Emitter | BatchMergeServiceがイベント発行 | 疎結合、拡張性高い | 複雑度増加 |
| C. Callback関数渡し | executeBatchMerge(onProgress) | 中間的、実装容易 | 複数リスナー対応不可 |

**決定**: **C. Callback関数渡し**

**根拠**:
- Ink.jsはReactベースで、stateと相性が良い
- 本機能では単一のUI画面のみが進捗を受け取る（複数リスナー不要）
- Event Emitterは過剰設計（YAGNI原則）
- 既存の`getMergedPRWorktrees`などもシンプルな同期/非同期パターン

**実装例**:
```typescript
// BatchMergeService
async executeBatchMerge(
  config: BatchMergeConfig,
  onProgress?: (progress: BatchMergeProgress) => void
): Promise<BatchMergeResult>

// UI側
const [progress, setProgress] = useState<BatchMergeProgress | null>(null);
await batchMergeService.executeBatchMerge(config, setProgress);
```

### 決定3: ドライランモードの実装方法

**選択肢**:

| オプション | 概要 | メリット | デメリット |
|----------|------|----------|----------|
| A. 一時worktree作成 | 実際にマージして後で削除 | 精度100%、本番と同じ | ディスク/時間コスト大 |
| B. git merge-tree | Git 2.38+のマージシミュレーション | 高速、ディスク不要 | Git 2.38+必須、精度90% |
| C. git merge --no-commit + rollback | マージするがコミットせず | 精度高い、Git互換性高い | ロールバック複雑 |

**決定**: **C. git merge --no-commit + rollback**

**根拠**:
- Git 2.5+で動作（仕様の依存関係より）
- `git merge --no-commit <branch>` でマージ実行
- コンフリクト検出は本番と同等
- ロールバックは `git merge --abort` または `git reset --hard HEAD` で簡単
- ディスクへの書き込みは発生するが、実worktree作成よりは軽量

**実装フロー**:
1. `git merge --no-commit <sourceBranch>` 実行
2. 結果確認（成功 or コンフリクト）
3. `git merge --abort` でロールバック
4. 結果を記録

### 決定4: テスト戦略

**選択肢**:

| レベル | モック vs 実環境 | 決定 | 根拠 |
|--------|----------------|------|------|
| Unit | execa をモック | モック | 高速、依存なし、境界値テスト容易 |
| Integration | 実gitリポジトリ作成 | 実環境 | 実際のgit動作検証、コンフリクト再現 |
| E2E | ink-testing-library | 実環境 | UI操作とgit操作の統合検証 |

**決定**: **ハイブリッドアプローチ**

**Unit Test**:
- execaをvi.mock()でモック
- 戻り値をシミュレート
- 例: `mergeFromBranch`が正しいgitコマンドを呼ぶか検証

**Integration Test**:
- 一時ディレクトリに実gitリポジトリ作成
- 実際のgitコマンド実行
- コンフリクトシナリオを再現
- テスト後にクリーンアップ

**E2E Test**:
- ink-testing-libraryで仮想ターミナル
- 実際の'p'キー押下をシミュレート
- 画面出力を検証

**既存パターンとの整合性**:
- `tests/unit/git.test.ts`: execa モック使用
- `tests/integration/branch-selection.test.ts`: 実gitリポジトリ使用
- `tests/e2e/branch-to-worktree.test.ts`: ink-testing-library使用

## 制約の確認

### 制約1: Worktree設計思想

**確認内容**:
- ブランチ切り替え禁止（`git checkout`、`git switch` 使用禁止）
- worktree経由でのブランチ操作のみ

**対応**:
- マージ対象ブランチのworktreeが存在しない場合、`createWorktree()`で作成
- マージは worktree内で実行（`cwd: worktreePath`）
- 現在のブランチを切り替えない

### 制約2: 既存モジュールとの互換性

**確認内容**:
- git.ts に関数追加しても既存関数に影響なし
- worktree.ts の `createWorktree()` をそのまま利用
- ui/types.ts に型追加しても既存型に影響なし

**対応**:
- Pure function として追加（副作用最小化）
- 既存関数のシグネチャ変更なし
- 新規型はexportで追加（既存型と名前空間衝突なし）

### 制約3: Ink.jsのCLI制約

**確認内容**:
- ターミナルサイズ制限（rows, columns）
- 更新頻度制限（過度な再描画でちらつき）

**対応**:
- 進捗更新は500ms間隔に制限
- React.memo()で不要な再描画防止
- 既存の`useTerminalSize`フック活用
- 長いブランチ名は省略表示

## ベストプラクティス

### Git操作のベストプラクティス

1. **エラーハンドリング**:
   ```typescript
   try {
     await execa("git", ["merge", sourceBranch], { cwd: worktreePath });
   } catch (error) {
     if (error.stderr?.includes("CONFLICT")) {
       // コンフリクト処理
     }
     throw new GitError("Merge failed", error);
   }
   ```

2. **作業ディレクトリ指定**:
   - 全git コマンドに `{ cwd: worktreePath }` 指定
   - 現在のディレクトリに依存しない

3. **Gitステート確認**:
   - マージ前に `git status --porcelain` で未コミット変更確認
   - マージ後に `getMergeStatus()` でコンフリクト検出

### Ink.jsのベストプラクティス

1. **パフォーマンス最適化**:
   ```typescript
   const ProgressDisplay = React.memo(({ progress }: Props) => { ... });
   ```

2. **状態管理**:
   - useStateでローカル状態管理
   - propsで親から状態受け取り

3. **エラー表示**:
   - `<Text color="red">` でエラーメッセージ
   - 既存の`ErrorBoundary`コンポーネント活用

## 代替案の評価

### 却下された代替案

**代替案1: 並列マージ処理**
- **却下理由**: 仕様が「順次処理」を明示、コンフリクト時の状態管理が複雑化
- **将来の拡張**: P4として並列化オプション追加可能

**代替案2: インタラクティブなコンフリクト解決**
- **却下理由**: 仕様の範囲外、CLIでの複雑なUI操作は困難
- **代替**: コンフリクト発生時はスキップし、手動解決を促すメッセージ表示

**代替案3: マージ結果のログファイル保存**
- **却下理由**: 仕様が「画面表示のみ」を明示
- **代替**: 画面出力を充実させる（詳細な結果サマリー）

## 調査結果サマリー

### 主要な技術決定

| 項目 | 決定内容 | リスク | 緩和策 |
|------|---------|--------|--------|
| git merge実装 | git.ts拡張 | 低 | 既存パターン踏襲 |
| 進捗更新 | Callback関数 | 低 | Ink.js標準パターン |
| ドライラン | --no-commit + rollback | 中 | 統合テストで検証 |
| テスト | ハイブリッド | 低 | 既存テスト構造踏襲 |

### 次のステップ

**Phase 1へ進行可**: 全ての技術的不明点が解決され、実装方法が確定しました。

次は以下のドキュメントを作成します：
1. `data-model.md` - データモデルと型定義
2. `quickstart.md` - 開発者向けクイックスタート
3. Agent context更新 - `.specify/memory/` へ技術情報追加
