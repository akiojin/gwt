<!-- GWT_SPEC_ARTIFACT:doc:data-model.md -->
doc:data-model.md

## Entities

### LogEvent

- `timestamp`: 出力時刻
- `level`: debug / info / warn / error
- `category`: subsystem 識別子 (`project`, `terminal`, `assistant`, `git`, `docker`, `report`, `config`, `startup_migration` など)
- `event`: 操作または状態遷移名
- `result`: `start` / `success` / `failure` / `progress`
- `workspace`: workspace 名
- `project_path?`: 対象 project path
- `branch?`: branch 名
- `issue_id?`: GitHub issue number
- `session_id?`: セッション識別子
- `command?`: 実行コマンドまたは外部依存名
- `duration_ms?`: 処理時間
- `error_code?`: typed error code
- `error_detail?`: 一次切り分けに必要な詳細

### FeatureCoverageTarget

- `id`: `FF-001` 形式
- `domain`: `startup`, `project`, `settings`, `issue_report`, `agent`, `worktree`, `git_pr`, `docker`, `migration`
- `flow_name`: 主要機能フロー名
- `required_points`: `start`, `success`, `failure`
- `status`: covered / gap

### IncidentCoverageTarget

- `id`: `IR-001` 形式
- `domain`: subsystem 名
- `scenario_name`: 代表障害シナリオ名
- `required_fields`: `category`, `event`, `context`, `error`
- `status`: covered / gap

### CoverageMatrix

- `feature_targets[]`: FeatureCoverageTarget 一覧
- `incident_targets[]`: IncidentCoverageTarget 一覧
- `coverage_ratio_feature`: covered / total
- `coverage_ratio_incident`: covered / total

## Invariants

- profiling 出力は `CoverageMatrix` 対象外
- `result=failure` の LogEvent は `error_code` または `error_detail` の少なくとも一方を持つ
- `FeatureCoverageTarget` の covered 判定には `start/success/failure` の 3 点が必要
- `IncidentCoverageTarget` の covered 判定には context と error の両方が必要

## Storage / Lifecycle

- 通常ログは既存どおり workspace 配下の日次 `gwt.jsonl` へ保存される
- profiling は `profile.json` に別保存され、通常ログ reader / report collector の対象外
- CoverageMatrix は SPEC artifact 上で管理し、実装後の acceptance 判定の source of truth とする
