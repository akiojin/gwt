### Rust テスト

- `issue_search_api_endpoint()` のクエリ文字列生成テスト（各カテゴリ: all/issues/specs）
- `parse_search_issues_json()` の REST API レスポンスパーステスト
- `per_page+1` による hasNextPage 判定テスト

### Frontend テスト

- Loading more 完了後にセンチネルが可視なら次ページがロードされるテスト
- loadingMore が Issue データ取得直後に false になるテスト（ブランチリンク完了前に）
