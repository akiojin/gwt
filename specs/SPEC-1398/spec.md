### 背景

Worktree Summary の PR タブで、`latestBranchPr`（過去の closed/merged PR を含む）が現在のブランチ状態より優先されるため、
過去PR由来の `Merged` / `Checks warning` が現状と不一致のまま表示されることがある。

### ユーザーシナリオ

- US1 (P0): 過去にマージ済みPRがあるブランチで新規コミットを積み、まだPRを作っていない状態でPRタブを開いたとき、`No PR` が表示される。
- US2 (P0): サイドバーのライブPR状態 (`prNumber`) と `latestBranchPr` が競合したとき、ライブ状態が優先される。

### 機能要件

- FR-001: PR詳細表示の対象番号は、`latestBranchPr` が `state=OPEN` の場合にのみ採用する。
- FR-002: `prNumber`（サイドバー由来のライブ状態）がある場合は `latestBranchPr` より常に優先する。
- FR-003: Open PR が存在しない場合、PR詳細を読み込まず `No PR` を表示する。

### 非機能要件

- NFR-001: 既存のPRタブ遷移・ポーリング性能に回帰がないこと。

### 成功基準

- SC-001: 過去 merged PR しかないケースで `Merged` バッジではなく `No PR` が表示される。
- SC-002: `prNumber` と `latestBranchPr` が不一致でも、`prNumber` のPR詳細が表示される。
- SC-003: 追加した回帰テストが RED→GREEN を確認できる。
