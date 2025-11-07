# 機能仕様: Releaseテスト安定化（保護ブランチ＆スピナー）

**仕様ID**: `SPEC-a5a44f4c`
**作成日**: 2025-11-07
**ステータス**: ドラフト
**入力**: ユーザー説明: "Fix release workflow test failures (mock initialization, protected branch switching, execa spinner test)"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - /release がテスト失敗で止まらない (優先度: P1)

リリース担当者として、`/release` コマンドが `useGitData` まわりの Vitest モック初期化エラーで止まらず、develop→main のマージを安全に進めたい。

**この優先度の理由**: リリースブロッカーであり、CI/CD が完全に停止しているため最優先で解決する必要がある。

**独立したテスト**: `bunx vitest run src/ui/__tests__/integration/navigation.test.tsx src/ui/__tests__/acceptance/navigation.acceptance.test.tsx` を単体で実行し、モック初期化エラーが再発しないことを確認できる。

**受け入れシナリオ**:

1. **前提条件** `vi.mock('../../../worktree.js')` を用いる統合・受入テストが存在する、**操作** テストを実行、**期待結果** `mockIsProtectedBranchName` および `acceptanceIsProtectedBranchName` の初期化エラーが発生せずパスする。
2. **前提条件** `vitest` が hoist する環境、**操作** `vi.hoisted` を含むテストを再実行、**期待結果** テストが deterministically PASS し、ログに hoist 警告が出ない。

---

### ユーザーストーリー 2 - 保護ブランチ切替の自動検証 (優先度: P2)

UI利用者として、保護ブランチを選択したときに `switchToProtectedBranch` が必ず呼ばれ、テストでもレグレッションを検知できるようにしたい。

**この優先度の理由**: クリティカルではないが、保護ブランチの UX 仕様を担保する唯一のテストであり、false negative を防ぐ必要がある。

**独立したテスト**: `bunx vitest run src/ui/__tests__/components/App.protected-branch.test.tsx` を単独で実行し、`switchToProtectedBranch` がモック呼び出しとして検証される。

**受け入れシナリオ**:

1. **前提条件** `App.protected-branch.test.tsx` が `switchToProtectedBranch` を spy している、**操作** テストを実行、**期待結果** `getRepositoryRoot` がモックされ `switchToProtectedBranch` 呼び出しが1回以上発生する。
2. **前提条件** `switchToProtectedBranch` が reject するケース、**操作** テストでエラーハンドリング分岐をシミュレート、**期待結果** `cleanupFooterMessage` にエラー文言を表示し再度 `navigateTo` しない（必要に応じて将来テスト追加）。

---

### ユーザーストーリー 3 - ワークツリースピナーの動作確認 (優先度: P3)

CLI開発者として、`tests/unit/worktree-spinner.test.ts` が `execa` の再定義エラーなくストリーム動作を検証できるようになってほしい。

**この優先度の理由**: リリースブロッカーではないが、スピナーの UX 回帰検知に必要なユニットテストであり、放置すると後続の回転処理が未検証になる。

**独立したテスト**: `bunx vitest run tests/unit/worktree-spinner.test.ts` を単独実行し、`Cannot redefine property: execa` エラーなく PASS する。

**受け入れシナリオ**:

1. **前提条件** `execa` を ES Module として mock する必要がある、**操作** `vi.hoisted` + `vi.mock('execa')` を適用、**期待結果** property 再定義エラーが発生しない。
2. **前提条件** spinner 停止処理を検証する、**操作** モックが `PassThrough` を流し `stopSpinner` を呼ばせる、**期待結果** `stopSpinner` が1回以上呼ばれ、`execaMock` 呼び出しを asserts できる。

### エッジケース

- Vitest の hoist により `vi.mock` ファクトリがトップレベル変数を参照した場合の Temporal Dead Zone。→ すべての共有モックは `vi.hoisted` で定義する。
- `getRepositoryRoot` が失敗した場合に `switchToProtectedBranch` が呼ばれず、テスト期待が満たせない。→ `App.protected-branch.test.tsx` ではインフラ層をモックし、アプリの振る舞いのみ検証する。
- `execa` を ESM 名前空間インポートとして扱うと property が再定義できない。→ `vi.mock` で import hook し、テスト内で `mockImplementation` を差し替える。

## 要件 *(必須)*

### 機能要件

- **FR-001**: すべての `vi.mock('../../../worktree.js', …)` を使用する UI テストは、`isProtectedBranchName`/`switchToProtectedBranch` を `vi.hoisted` で生成した共有モックに差し替え、Temporal Dead Zone を回避しなければならない。
- **FR-002**: `App.protected-branch.test.tsx` は `getRepositoryRoot` を `vi.spyOn` でスタブし、保護ブランチ切替フロー中に `switchToProtectedBranch` が確実に呼ばれることを検証しなければならない。
- **FR-003**: `handleProtectedBranchSwitch` 実行後に `navigateTo('ai-tool-selector')` と `refresh()` が呼ばれることをテストで確認し、UX の一貫性を守らなければならない。
- **FR-004**: `tests/unit/worktree-spinner.test.ts` は `execa` モジュールを `vi.mock` で完全に置き換え、`PassThrough` ストリームを返してスピナーの start/stop を検証できなければならない。
- **FR-005**: 上記テストは bun 1.0+ 環境で `bunx vitest run` を使用しても安定して再現性を保たなければならない。
- **FR-006**: 新たなテストヘルパーやフックを導入する場合は TypeScript 型安全性を維持し、既存の `happy-dom` / `@testing-library/react` 依存を変更しないこと。

### 主要エンティティ *(機能がデータを含む場合は含める)*

- **ProtectedBranchMock**: `isProtectedBranchName` と `switchToProtectedBranch` の Vitest モックを指し、`mockReset`, `mockResolvedValue` などの状態 API を提供する。
- **RepoRootStub**: `getRepositoryRoot` をスタブする `vi.SpiedFunction`, 期待される `repoRoot` 文字列（例: `/repo`）を返す。
- **ExecaMockProcess**: `PassThrough` stdout/stderr を保持し、`stopSpinner` を発火させる擬似 `execa` 応答。

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: `bunx vitest run src/ui/__tests__/integration/navigation.test.tsx src/ui/__tests__/acceptance/navigation.acceptance.test.tsx` が 2 分以内に PASS し、モック初期化エラーが 0 件である。
- **SC-002**: `bunx vitest run src/ui/__tests__/components/App.protected-branch.test.tsx` が 1 分以内に PASS し、`switchToProtectedBranch` 呼び出し回数が 1 回以上である。
- **SC-003**: `bunx vitest run tests/unit/worktree-spinner.test.ts` が `Cannot redefine property: execa` を出さずに PASS する。
- **SC-004**: リポジトリルートで `bun run test` を実行した際、今回触れたテストファイルに関連する失敗が 0 件である。

## 制約と仮定 *(該当する場合)*

### 制約

- Vitest 2.1.x + bun 1.0 環境で動作すること。Node.js 18 互換コードのみ使用可。
- 既存の `worktree.ts` などアプリ本体の挙動は変更せず、テストコードと補助モジュールのみに修正を限定する。
- `happy-dom` をテストランタイムとして継続使用し、JSDOM 切替などの大規模変更は行わない。

### 仮定

- bun 1.1.0 以降で `bunx vitest` が使用可能である。
- `git` CLI が存在しない CI でも `getRepositoryRoot` をモックすればテストが成立する。
- `vi.hoisted` が Vitest 2.1.8 でサポートされている。

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- `worktree.ts` 本体のビジネスロジック変更
- CLI UI の新規機能追加やキーバインド変更
- release フロー以外の GitHub Actions 改修

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- テストで `getRepositoryRoot` をモックしても実ディレクトリへのアクセス情報はログに出力しない。
- `execa` モックはシェルコマンドを実行しないため、CI のサンドボックスに追加の権限を要求しない。
- 追加のログ出力を行う場合も、個人情報やローカルパスを含まないようにする。

## 依存関係 *(該当する場合)*

- Vitest 2.1.8 / happy-dom 20.0.8 / @testing-library/react 16.3.0
- bun 1.0+ / TypeScript 5.8.x
- 既存の git/worktree ユーティリティ (`getRepositoryRoot`, `switchToProtectedBranch`, `startSpinner`)

## 参考資料 *(該当する場合)*

- `.claude/commands/release` - release コマンド仕様と失敗ログ
- `src/ui/components/App.tsx` - 保護ブランチ切替ロジック
- `tests/unit/worktree-spinner.test.ts` - スピナー統合テスト
- `src/ui/__tests__/integration/navigation.test.tsx` - Navigation 統合テスト
- `src/ui/__tests__/acceptance/navigation.acceptance.test.tsx` - Navigation 受入テスト
