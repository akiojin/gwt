## 最小検証フロー

### 1. exact cache の基本動作確認

```
1. テスト用 repo で IssueExactCache を初期化（空）
2. resolve_issue_from_cache(repo, 42) → None（cache miss）
3. fetch_issue_detail(repo, 42) を実行 → cache に自動保存
4. resolve_issue_from_cache(repo, 42) → Some(entry) で title 取得
5. gh CLI を無効化して resolve → 既存 cache から返却されることを確認
```

### 2. REST fallback 確認

```
1. fetch_issue_detail() を呼ぶ
2. gh issue view を意図的に失敗させる（rate limit シミュレーション）
3. gh api repos/{slug}/issues/{number} で取得成功を確認
4. cache が更新されていることを確認
```

### 3. Linkage bootstrap 確認

```
1. テスト repo に feature/issue-100 branch を作成
2. bootstrap_linkage_from_branches() を実行
3. linkage store に branch→#100 が BranchParse で登録されていることを確認
4. main/develop branch が除外されていることを確認
```

### 4. Diff sync 確認

```
1. full sync 実行 → watermark 記録
2. GitHub 側で Issue を更新
3. diff sync 実行 → 更新分のみ取得、SyncResult.updated_count > 0
4. stale entry が削除されていないことを確認
```

### 5. Full sync + stale cleanup 確認

```
1. cache に存在しない Issue 番号のダミーエントリを挿入
2. full sync 実行
3. ダミーエントリが削除されていることを確認（SyncResult.deleted_count > 0）
```

### 6. UI 統合確認

```
1. Worktree を開く（feature/issue-42 branch）
2. Sidebar / tab に "#42 Issue Title" が表示されることを確認
3. ネットワークを切断して再表示 → cache から表示が維持されることを確認
4. 手動更新 UI で Diff Sync / Full Sync を実行 → 結果表示を確認
```
