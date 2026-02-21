# TDD: Docker compose 起動後の service ヘルス検証

**仕様ID**: `SPEC-4e2f1028`  
**更新日**: 2026-02-21

## 対象

- US3: service 即時終了時に Launch エラーへ原因を表示する。

## テスト戦略

1. RED: service 判定ロジックの不足を再現する単体テストを追加する。
2. GREEN: 判定ロジックを実装し、追加テストを通す。
3. REFACTOR: 既存 Docker 関連テストを再実行して回帰がないことを確認する。

## 追加テストケース

1. `compose_services_output_contains_matches_trimmed_exact_line`
   - `docker compose ps --services` 相当出力の前後空白を許容し、service 名の完全一致を正しく判定する。
2. `compose_services_output_contains_does_not_match_partial_names`
   - `app` と `application` のような部分一致を誤って true にしない。

## 実行コマンド

- `cargo test -p gwt-tauri compose_services_output_contains -- --nocapture`
- `cargo test -p gwt-tauri build_docker_compose_up_args_build_and_recreate_flags -- --nocapture`
- `cargo test -p gwt-tauri docker_ -- --nocapture`

## 完了条件

- 追加テストがすべて成功する。
- 既存 Docker 関連テストに失敗がない。
- 起動時に service が `running` でない場合、Launch エラーに service 未起動と logs が含まれる。
