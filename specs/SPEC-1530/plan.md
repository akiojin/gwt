1. `gh_cli.rs` の branch 単位 PR 集約を latest 優先から unsafe-first 集約へ変更する。
2. `cleanup.rs` の safety 判定を `Merged => Safe`, `Open|Closed|None => Warning` に変更する。
3. `CleanupModal.svelte` の effective safety を backend と同じ意味論に合わせる。
4. mixed PR history と `closed` safety の RED/GREEN テストを追加する。
