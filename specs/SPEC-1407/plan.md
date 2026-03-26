1. `WorktreeSummaryPanel.svelte` に `latestBranchPrBranch` state 変数を追加
2. `loadLatestBranchPr` の全代入箇所で `latestBranchPrBranch` を同期更新
3. `resolvedPrNumber` の state チェックをブランチ名一致チェックに置換
4. 既存テストのアサーションを修正後の動作に合わせて更新
