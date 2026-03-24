### 技術コンテキスト

- Rust/Tauri 実装では `profile`, `settings`, `profiles command`, `terminal command`, `sessions/version_history command` が対象
- #1542 を file layout canonical source とし、本 issue は AI settings shape と runtime consumption の canonical source とする

### 実装アプローチ

- AI settings は profile 配下に統合する
- `Settings` は runtime model とし、`ConfigToml` が `~/.gwt/config.toml` の serde を担当する
- `[profiles]` の on-disk shape は `ProfilesSectionToml` が担い、runtime では `ProfilesConfig` と `Profile` に抽出する
- runtime command は canonical profile data のみを見る
- sidecar file や legacy helper field に依存しない

### フェーズ分割

1. canonical AI persistence shape を spec に固定する
2. runtime consumption points を canonical shape に整合させる
3. regression test で再起動後保持と sidecar 非依存を固定する
4. runtime model と TOML DTO の境界を spec と実装で一致させる
