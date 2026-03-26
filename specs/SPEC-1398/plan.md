1. `WorktreeSummaryPanel.test.ts` に回帰テストを追加し、誤表示を再現する。
2. `WorktreeSummaryPanel.svelte` の `resolvedPrNumber` 算出を修正して、`prNumber` 優先 + `latestBranchPr.state===OPEN` 条件を導入する。
3. PRタブの表示が Open PR のみ詳細表示対象になることをテストで検証する。
