# リサーチノート: SPEC-a1b2c3d4

## 1. sysinfo crate のフィーチャーフラグ

CPU/メモリのみ必要なため、最小フィーチャーセットを使用する:

```toml
sysinfo = { version = "0.32", default-features = false, features = ["system"] }
```

`"system"` フィーチャーに CPU・メモリが含まれる。`processes`, `networks`, `disks` は不要。

## 2. nvml-wrapper crate のクロスプラットフォーム対応

nvml-wrapper は NVIDIA CUDA ライブラリ（libcuda.so / nvml.dll）に依存するため、macOS ではコンパイル不可。

**対策**: オプション依存 + プラットフォームゲート

```toml
[dependencies]
nvml-wrapper = { version = "0.10", optional = true }

[features]
nvidia-gpu = ["nvml-wrapper"]

[target.'cfg(any(target_os = "linux", target_os = "windows"))'.dependencies]
nvml-wrapper = { version = "0.10", optional = true }
```

コード内では `#[cfg(feature = "nvidia-gpu")]` で条件コンパイル。macOS ビルドでは自動的にスキップされる。

## 3. 既存 config load/save パターン

`crates/gwt-core/src/config/agent_config.rs` のパターン:

- **パス解決**: `dirs::home_dir().unwrap_or(".").join(".gwt").join(FILENAME)`
- **ロード**: ファイル存在チェック → TOML パース → エラー時は `backup_broken_file()` + デフォルト返却
- **セーブ**: `ensure_config_dir()` → `toml::to_string_pretty()` → `write_atomic()` (temp + rename)
- **エラー**: `GwtError::ConfigWriteError` / `ConfigParseError` を使用
- **ログ**: `tracing::info!` / `tracing::warn!` で category="config"

ヘルパー関数（`crates/gwt-core/src/config/migration.rs`）:

- `ensure_config_dir(dir)`: ディレクトリ作成 + Unix で 0o700 パーミッション
- `write_atomic(path, content)`: temp ファイル書き込み → Unix で 0o600 → rename
- `backup_broken_file(path)`: `.broken` 拡張子にリネーム

## 4. ワークスペース依存関係（既存）

すでに利用可能な依存:

- `serde` (derive), `toml`, `serde_json`, `chrono`, `tracing`, `tokio`, `thiserror`, `dirs`

新規追加が必要:

- `sysinfo` → gwt-core の Cargo.toml に追加
- `nvml-wrapper` → gwt-core の Cargo.toml にオプション追加

## 5. AppState への SystemMonitor 追加

`crates/gwt-tauri/src/state.rs` の AppState に `Mutex<SystemMonitor>` を追加。既存パターン（pane_manager, agent_versions_cache 等）と同じ。`AppState::new()` で初期化。

## 6. フロントエンドテストパターン

- vitest + @testing-library/svelte + jsdom 環境
- Tauri API は `vi.mock("@tauri-apps/api/core")` でモック
- `beforeEach` で `invokeMock.mockReset()`
- `afterEach` で `cleanup()`

## 7. config ディレクトリ解決

```rust
pub fn new_global_config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".gwt"))
}
```

全 config ファイルは `~/.gwt/` 配下に配置する統一パターン。
