# タスク: ブランチ一覧の表示順序改善

**仕様ID**: `SPEC-a5ae4916`
**入力**: `/specs/SPEC-a5ae4916/` の設計ドキュメント一式（spec.md / plan.md / research.md / data-model.md / quickstart.md）
**前提条件**: Bun 1.x 環境、既存のテストスイート（Vitest）

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- `[P]` は並列実行可能タスク
- `[USn]` はユーザーストーリー番号（US1〜US3）
- 説明には必ず対象ファイル/パスを含める

---
## フェーズ1: セットアップ

目的: 既存実装とテストのベースライン確認

- [x] T001 分析のため `specs/SPEC-a5ae4916/spec.md` のユーザーストーリー優先度と Clarifications を確認する
- [x] T002 `src/ui/table.ts` の現行ソート処理を精読し、既存優先度の挙動を整理する
- [x] T003 `bun run test tests/unit/ui/table.test.ts` を実行し、現状のテストがグリーンであることを確認する

## フェーズ2: 共通基盤整備（Foundational）

目的: 3つのストーリーで共通利用するテスト基盤を整備

- [x] T004 `tests/unit/ui/table.test.ts` に BranchInfo/WorktreeInfo を生成するヘルパー関数を追加してデータ定義を集中管理する
- [x] T005 [P] `tests/unit/ui/table.test.ts` に複数条件のソート結果を検証できるユーティリティ（期待順比較ロジック）を追加する

## フェーズ3: ユーザーストーリー1 (P1) - Worktree付きブランチの優先表示

独立価値: 作業中（worktreeあり）のブランチへ最短でアクセスできる

- [x] T006 [US1] `tests/unit/ui/table.test.ts` に worktree 有無で順位が変わるテストケースを追加する
- [x] T007 [US1] `src/ui/table.ts` に worktreeMap 判定を追加し、main/develop 後に worktree 有無で分岐させる
- [x] T008 [US1] `bun run test tests/unit/ui/table.test.ts` を実行し、US1 の新規テストがパスすることを確認する

## フェーズ4: ユーザーストーリー2 (P2) - ローカルブランチの優先表示

独立価値: 即時作業可能なローカルブランチへのアクセス向上

- [x] T009 [US2] `tests/unit/ui/table.test.ts` にローカル vs リモートオンリーの優先度を検証するテストケースを追加する
- [x] T010 [US2] `src/ui/table.ts` に `branch.type` を利用したローカル優先ロジックを追加し、worktree優先処理の直後に配置する
- [x] T011 [US2] `bun run test tests/unit/ui/table.test.ts` を実行し、US2 追加テストを含むテストが成功することを確認する

## フェーズ5: ユーザーストーリー4 (P2) - 最新コミット順でのソート

独立価値: Worktree有無のグループ内で最新の作業コンテキストへ素早くアクセスできる

- [x] T020 [US4] `src/git.ts` で `git for-each-ref` の結果から最新コミットUNIXタイムスタンプを取得し、`BranchInfo` に `latestCommitTimestamp` を追加する
- [x] T021 [US4] `src/ui/utils/branchFormatter.ts` のソート処理に最新コミット降順ロジックを追加し、worktree優先とローカル優先との優先順位を整理する
- [x] T022 [US4] `src/ui/__tests__/utils/branchFormatter.test.ts` に最新コミット順を検証するテスト（worktreeあり/なし双方のケース）を追加する
- [x] T023 [US4] `bun test src/ui/__tests__/utils/branchFormatter.test.ts` を実行し、新規テストがパスすることを確認する
- [x] T024 [US4] `src/ui/components/common/Select.tsx` と `src/ui/components/screens/BranchListScreen.tsx` を更新し、最終更新時刻の表示と行全体の背景ハイライトを実装する
- [x] T025 [US4] `src/ui/__tests__/components/screens/BranchListScreen.test.tsx` に最終更新表示と背景ハイライトを検証するテストを追加する
- [x] T026 [US4] `bun test src/ui/__tests__/components/screens/BranchListScreen.test.tsx` を実行し、UI テストが成功することを確認する

## フェーズ6: ユーザーストーリー3 (P3) - 既存優先順位の維持

独立価値: main / develop / 既存ルールの互換性保証

- [x] T012 [US3] `tests/unit/ui/table.test.ts` に main→develop→その他 の順序と release/hotfix が一般ルールに従うことを検証するテストを追加する
- [x] T013 [US3] `src/ui/table.ts` に develop 専用の優先分岐を追加し、release/hotfix は一般ルールへフォールバックするよう整理する
- [x] T014 [US3] `bun run test tests/unit/ui/table.test.ts` を実行し、US3 で追加したテストが全て通過することを確認する

## フェーズ7: ポリッシュ & 回帰確認

目的: 全体品質と体験の最終確認

- [x] T015 `bun run test` を実行し、全テストスイートの回帰を確認する
- [x] T016 [P] `bun run lint` を実行し、スタイル/静的解析警告が無いことを確認する
- [x] T017 `bun run start -- --help` で CLI を起動し、ブランチ一覧表示の新しい順序を手動確認する（実行ログを共有）
- [x] T018 [P] `specs/SPEC-a5ae4916/quickstart.md` を更新し、手順やチェックリストに変更があれば追記する
- [x] T019 [P] `docs/troubleshooting.md` のブランチ表示に関する節を確認し、必要なら新しい優先ルールを反映する

## 実装戦略 & 依存関係

- MVP: フェーズ3 (US1) 完了時点で最小価値を提供
- 依存順: フェーズ1 → フェーズ2 → US1 → US2 → US3 → ポリッシュ
- 並列候補: T005, T009 と T006 はデータ整備後に並列実施可。ポリッシュの T016〜T019 も相互依存なし。

## 独立テスト基準

- US1: worktree 付きブランチが最上位グループに並ぶことをテストで検証
- US2: ローカルブランチがリモートオンリーより上に並ぶことをテストで検証
- US3: 現在→main→develop→worktree→最新コミット降順→ローカル→その他→名前順 が維持され、release/hotfix が一般ルールに従うことをテストで検証
- US4: worktree有無が同じブランチ群で最新コミットタイムスタンプの降順になること、各行に「最終更新:」が表示され選択行のみ背景色が変化することをテストで検証

## 推奨MVP範囲

- フェーズ3（US1）までを完了し、worktree 優先の価値を最短で提供。その後 US2→US3 と段階的に拡張する。
