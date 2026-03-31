### 背景

現行 gwt では、アプリ設定が `config.toml` と複数の sidecar 設定ファイルに分散し、stale な値が保存結果を上書きしうる。特に API キーや recent projects などの app-wide settings は、保存先が複数ある状態を許すと整合性が壊れる。

この仕様では、**アプリ設定の正本を `~/.gwt/config.toml` に一本化**し、project 固有メタデータだけを `<project>/.gwt/project.toml` に限定する。`sessions` `logs` `cache` `history` は設定ではない別カテゴリとして扱う。

### ユーザーシナリオ

- **US-1 [P0]**: ユーザーが設定画面で変更した値は `~/.gwt/config.toml` にのみ保存され、再起動後も同じ値が読み込まれる
- **US-2 [P0]**: API キー、agent config、custom tools、recent projects は sidecar file を削除しても動作が変わらない
- **US-3 [P0]**: bare repo ベースの project は `<project>/.gwt/project.toml` の最小メタデータから解決される
- **US-4 [P1]**: repo-local `.gwt.toml` / `.gwt/config.toml` は global-only settings を上書きできない
- **US-5 [P1]**: retired sidecar file が存在しても、アプリはそれらを読まない

### 機能要件

| ID | 要件 |
|---|---|
| FR-001 | app-wide settings の唯一の正本は `~/.gwt/config.toml` とする |
| FR-002 | `config.toml` には少なくとも `profiles`, `agent_config`, `tools`, `recent_projects`, `agent`, `docker`, `appearance`, `voice_input`, `terminal` を保持できる |
| FR-003 | profiles の canonical shape は `[profiles]` に `version`, `active` を持ち、各 profile は `[profiles.<name>]` と `[profiles.<name>.ai]` で表現する |
| FR-004 | `profiles.profiles.<name>` のような二重ネストは canonical shape として認めない |
| FR-005 | `<project>/.gwt/project.toml` は project-local の最小 Git 補助メタデータのみを保存する |
| FR-006 | `project.toml` の必須キーは `bare_repo_name` とし、`remote_url`, `location`, `created_at` は補助メタデータとして保持できる |
| FR-007 | 次の sidecar 設定ファイルは非対応とし、読込・保存・暗黙吸収を行わない: `agents.toml`, `profiles.toml`, `recent-projects.toml`, `recent-projects.json`, `tools.toml`, `tools.json` |
| FR-008 | retired sidecar 設定ファイルに対する fallback, auto-migrate, implicit merge は行わない |
| FR-009 | repo-local 設定の読込・保存では global-only section を扱わない |
| FR-010 | `config.toml` の section 更新は他 section の値を失わない排他制御を持つ |
| FR-011 | env override は runtime behavior にのみ作用し、設定保存によって `config.toml` に永続化されない |
| FR-012 | `sessions`, `session-summaries`, `agent-history`, `stats`, `logs`, `cache`, `updates` は設定ファイルではなく runtime/state/cache として本仕様の対象外とする |

### 非機能要件

| ID | 要件 |
|---|---|
| NFR-001 | app settings は single source of truth として `config.toml` 一箇所に固定される |
| NFR-002 | `config.toml` の書き込みは atomic かつ serialized update path を持つ |
| NFR-003 | secret を repo-local 設定へ保存しない |
| NFR-004 | retired sidecar file の存在有無でアプリ挙動が変わらない |

### 成功基準

| ID | 基準 |
|---|---|
| SC-001 | API キー保存後、`~/.gwt/config.toml` の canonical profile data から再読込される |
| SC-002 | sidecar 設定ファイルを削除しても app settings の挙動が変わらない |
| SC-003 | `project.toml` から bare repo 解決ができる |
| SC-004 | repo-local 設定は global-only section を read/write しない |
| SC-005 | `config.toml` の同時更新で section の取りこぼしが起きない |
