### 背景
Issue #1655 では、Version History を開くと `Profile name uses reserved key in config.toml: profiles` で失敗する。原因は、過去バージョンが生成した `~/.gwt/config.toml` の `[profiles.profiles.<name>]` 形式を現行コードが予約キー `profiles` を持つ壊れたプロファイルとして扱ってしまうため。

### ユーザーシナリオとテスト

**S1: 既存ユーザーの Version History 表示**
- Given: `~/.gwt/config.toml` に legacy な `[profiles.profiles.default]` 形式が保存されている
- When: ユーザーが Git > Version History を開く
- Then: 設定読込で失敗せず、Version History が利用できる

**S2: legacy profiles 設定の読込**
- Given: `~/.gwt/config.toml` が `[profiles.profiles.<name>]` を含む
- When: `ProfilesConfig::load()` を実行する
- Then: `<name>` を通常の profile 名として解釈してロードできる

**S3: 正常系の保存形式維持**
- Given: 現行の `ProfilesConfig` を保存する
- When: `config.toml` を出力する
- Then: 保存形式は canonical な `[profiles.<name>]` を維持し、`[profiles.profiles.<name>]` を出力しない

### 機能要件

**FR-01: backward compatibility**
- `ProfilesConfig::load()` は legacy な nested profiles 形式を受理する

**FR-02: canonical save format**
- `ProfilesConfig::save()` / global settings save は canonical な `[profiles.<name>]` を維持する

**FR-03: Version History availability**
- Version History は profiles 設定の legacy 形式が残っていても設定読込エラーで失敗しない

### 成功基準

1. legacy config を使うテストが RED → GREEN になる
2. canonical save format を使う既存テストが維持される
3. Version History が `profiles` 予約キーエラーを返さない

---
