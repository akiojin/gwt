<!-- GWT_SPEC_ARTIFACT:doc:quickstart.md -->
doc:quickstart.md

## Goal

通常ログ基盤の仕様が、profiling と分離され、主要機能フローと主要障害シナリオの 90% 以上をカバーする設計になっていることをレビューする。

## Reviewer Flow

1. `doc:spec.md` を読み、機能ログ/障害対応ログ coverage の定義が明確か確認する
2. `doc:data-model.md` を読み、通常ログの必須フィールド契約と profiling 分離が一貫しているか確認する
3. `doc:tasks.md` を見て、主要 subsystem と report 導線に対して test-first tasks が割り当てられているか確認する
4. coverage matrix の対象領域が `startup`, `project`, `settings`, `issue_report`, `agent`, `worktree`, `git_pr`, `docker`, `migration` を含むことを確認する
5. Issue report の `Application Logs` が通常ログのみ対象であることを acceptance と task の両方で確認する

## Minimum Validation Scenarios

- app startup / project open のログ追跡
- settings save failure の障害対応ログ
- git or docker failure の一次切り分けログ
- agent launch failure の context 付きログ
- report dialog で `Application Logs` が profiling を含まないこと

## Exit Criteria

- profiling と通常ログの責務分離が artifacts 間で矛盾しない
- feature coverage 90% と incident coverage 90% の判定方法が明文化されている
- tasks が test-first で、主要 subsystem に traceability を持つ
