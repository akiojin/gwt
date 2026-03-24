### 背景

Issue #1398 の修正（コミット `18ee87c4`）で `resolvedPrNumber` に `latestBranchPr.state !== "OPEN"` チェックを追加した。
これはブランチ切り替え時に旧ブランチの MERGED PR が残留する問題への対処だったが、
副作用として **MERGED/CLOSED PR しか持たないブランチでは常に "No PR" になる** リグレッションが発生した。

サイドバーのポーリング（`fetch_pr_status`）は OPEN PR のみ返す設計のため、OPEN PR がなければサイドバーの PR バッジも表示されない。
結果として両方のパスが null を返し、全ブランチで "No PR" となる。

### ユーザーシナリオとテスト（受け入れシナリオ）

- US1 (P0): MERGED PR しかないブランチで PR タブを開いたとき、MERGED PR の詳細が表示される。
- US2 (P0): ブランチ切り替え後に旧ブランチの MERGED PR データが残留していても、新ブランチでは表示されない（#1398 の意図を維持）。
- US3 (P0): `prNumber`（サイドバー由来）がある場合は `latestBranchPr` より常に優先される（既存動作を維持）。

### 機能要件

- FR-001: `resolvedPrNumber` は PR state ではなくブランチ名で staleness を判定する。
- FR-002: `latestBranchPrBranch` 変数を追加し、`latestBranchPr` がどのブランチ用のデータかを追跡する。
- FR-003: `resolvedPrNumber` では state チェックの代わりにブランチ名の一致を検証する。
- FR-004: `prNumber`（サイドバー由来）がある場合は `latestBranchPr` より常に優先する（既存動作維持）。

### 非機能要件

- NFR-001: 既存の PR タブ遷移・ポーリング性能に回帰がないこと。

### 成功基準

- SC-001: MERGED PR のみのブランチで PR 詳細が表示される。
- SC-002: ブランチ切り替え時に旧ブランチの PR データがブロックされる。
- SC-003: 既存テスト全 87 件が pass する。
