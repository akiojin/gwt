# タスク: Releaseテスト安定化（保護ブランチ＆スピナー）

**入力**: `specs/SPEC-a5a44f4c/` の設計ドキュメント
**前提条件**: plan.md / spec.md / research.md / data-model.md / contracts/ / quickstart.md

## フォーマット: `- [ ] T001 [P?] [US?] 説明 (file)`

## フェーズ1: セットアップ

- [ ] T001 [P] 再現ログを確保するため `package.json:scripts.test` に従って `bunx vitest run src/ui/__tests__/integration/navigation.test.tsx` を実行し失敗内容を記録
- [ ] T002 `specs/SPEC-a5a44f4c/spec.md` と `plan.md` を精読し受入条件と優先度を確認
- [ ] T003 `specs/SPEC-a5a44f4c/research.md` の決定事項を作業ノートへ転記し、モック方針を共有

## フェーズ2: 基盤

- [ ] T010 `specs/SPEC-a5a44f4c/data-model.md` に定義した ProtectedBranchMock/RepoRootStub/ExecaMockProcess を参照し、編集対象ファイルごとの影響範囲を洗い出す
- [ ] T011 `specs/SPEC-a5a44f4c/quickstart.md` を更新してテスト再実行コマンドとトラブルシュートを最新化

## フェーズ3: ユーザーストーリー1 - /release がテスト失敗で止まらない (P1)

- [ ] T101 [P] [US1] `src/ui/__tests__/integration/navigation.test.tsx` に `vi.hoisted` を導入し `mockIsProtectedBranchName` / `mockSwitchToProtectedBranch` を前方宣言
- [ ] T102 [US1] 同ファイルの `beforeEach`/`afterAll` を整理し、hoisted モックの `mockReset` と `mockImplementation` を統一
- [ ] T103 [P] [US1] `src/ui/__tests__/acceptance/navigation.acceptance.test.tsx` にも hoisted モックを追加し、`Mock` 型キャストと `mockReset` を調整
- [ ] T104 [US1] 上記2ファイルで `getAllBranches` などの既存モック参照が崩れないよう import/型注釈を再整備
- [ ] T105 [US1] `bunx vitest run src/ui/__tests__/integration/navigation.test.tsx src/ui/__tests__/acceptance/navigation.acceptance.test.tsx` を実行し Temporal Dead Zone エラーが消えることを確認

## フェーズ4: ユーザーストーリー2 - 保護ブランチ切替の自動検証 (P2)

- [ ] T201 [US2] `src/ui/__tests__/components/App.protected-branch.test.tsx` で `gitModule.getRepositoryRoot` を `vi.spyOn` し `/repo` を返すよう `beforeEach` に追加
- [ ] T202 [US2] 同ファイルで `afterEach/afterAll` に `getRepositoryRoot` のリセット処理を追加し、副作用を除去
- [ ] T203 [US2] テスト内の `act` ブロックを整理して `await act(async () => { ... })` 内で `switchToProtectedBranch` 呼び出しを確実に await
- [ ] T204 [US2] `bunx vitest run src/ui/__tests__/components/App.protected-branch.test.tsx` を実行し `switchToProtectedBranch` が期待通りに呼ばれることを確認

## フェーズ5: ユーザーストーリー3 - ワークツリースピナーの動作確認 (P3)

- [ ] T301 [P] [US3] `tests/unit/worktree-spinner.test.ts` に `vi.hoisted` で共有 `execaMock` を定義し `vi.mock('execa', () => ({ execa: execaMock }))` へ変更
- [ ] T302 [US3] 同テストで `execaMock.mockImplementation` を更新し、`PassThrough` と `stopSpinner` のライフサイクルをコメント付きで整理
- [ ] T303 [US3] テスト末尾に `await Promise.resolve()` 等でマイクロタスクを flush し、`stopSpinner`/`execaMock` の assertion を安定化
- [ ] T304 [US3] `bunx vitest run tests/unit/worktree-spinner.test.ts` を実行し `Cannot redefine property: execa` が解消されたことを確認

## フェーズ6: 統合・ポリッシュ

- [ ] T401 `bun run test` を実行し release コマンド相当の回帰テストを完走
- [ ] T402 `bun run lint` `bun run format:check` `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` を流しコーディング規約に準拠
- [ ] T403 変更したテストファイル（`src/ui/__tests__/integration/navigation.test.tsx` など）にコメント又は docstring を追加し、将来の hoist 問題再発時の指針を残す
- [ ] T404 Conventional Commit (`fix: stabilize release test suite`) を作成し、`git status -sb` で変更範囲を最終確認

## 依存関係

1. フェーズ1→フェーズ2 は順番必須（前提知識/ドキュメント整備）
2. US1 (P1) 完了後に US2 (P2) を着手可能。US3 (P3) は US1 と並列可（別ディレクトリ）
3. ポリッシュフェーズは全ストーリー完了後

## 並列実行例

- T101 と T103 は異なるファイルの hoisted 対応なので並列可能
- T201 系と T301 系は互いに独立 (UI vs unit test) なので別エージェントで着手可

## MVP スコープ

- MVP は US1 の完了（T101〜T105）で達成。これにより `/release` フローのブロッカーが解除される。

## 独立テスト基準

- **US1**: 対象テスト2本が Vitest で PASS し、ログに初期化エラーが無い
- **US2**: `App.protected-branch.test.tsx` が PASS し、`switchToProtectedBranch` 呼び出しをアサート
- **US3**: `tests/unit/worktree-spinner.test.ts` が PASS し、`stopSpinner` 呼び出し回数>0
