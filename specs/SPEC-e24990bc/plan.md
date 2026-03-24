### 技術コンテキスト

- Rust/Tauri 実装では `settings`, `profile`, `agent_config`, `tools`, `recent_projects`, `bare_project`, `project command` が対象
- Unity/C# 側へ移植する際も、settings shape と file layout はこの spec を canonical とする

### 実装アプローチ

- app-wide settings は `config.toml` の section に統合する
- `Settings` は runtime model とし、`~/.gwt/config.toml` の serde は `ConfigToml` が担当する
- `[profiles]` section の on-disk shape は `ProfilesSectionToml` が担当し、runtime では `ProfilesConfig` と `Profile` へ抽出する
- project-local metadata は `project.toml` に分離する
- sidecar 設定ファイルは仕様上サポート外とし、fallback や移行コードを増やさない
- cache/state/history は settings と切り離して扱う

### フェーズ分割

1. canonical file layout と section schema を固定する
2. runtime implementation が sidecar 設定ファイルを参照しない状態へ合わせる
3. repo-local 設定と global-only settings の境界を固定する
4. regression test で canonical shape と sidecar 非依存を検証する
5. runtime model と TOML DTO の責務をコードと spec の両方で一致させる
