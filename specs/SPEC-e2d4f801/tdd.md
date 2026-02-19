# TDDテスト仕様: OS環境変数の自動継承

## T2: parse_env_null_separated テスト

### test_parse_normal_env

NUL区切りの正常な環境変数バイト列をパースし、正しい `HashMap` が返ること。

```text
入力: b"HOME=/Users/test\0PATH=/usr/bin:/bin\0SHELL=/bin/zsh\0"
期待: {"HOME": "/Users/test", "PATH": "/usr/bin:/bin", "SHELL": "/bin/zsh"}
```

### test_parse_value_with_newline

値に改行を含む環境変数が正しくパースされること。

```text
入力: b"MULTI_LINE=line1\nline2\nline3\0NORMAL=value\0"
期待: {"MULTI_LINE": "line1\nline2\nline3", "NORMAL": "value"}
```

### test_parse_empty_input

空バイト列を渡した場合、空の `HashMap` が返ること。

```text
入力: b""
期待: {}
```

### test_parse_value_with_equals

値に `=` を含む環境変数が正しくパースされること（最初の `=` のみをキーと値の境界とする）。

```text
入力: b"DATABASE_URL=postgres://user:pass@host/db?opt=val\0"
期待: {"DATABASE_URL": "postgres://user:pass@host/db?opt=val"}
```

### test_parse_ignores_entries_without_equals

`=` を含まないエントリは無視されること（MOTD混入対策）。

```text
入力: b"Welcome to the server!\0HOME=/Users/test\0"
期待: {"HOME": "/Users/test"}
```

### test_parse_empty_value

値が空の環境変数が正しくパースされること。

```text
入力: b"EMPTY_VAR=\0NORMAL=value\0"
期待: {"EMPTY_VAR": "", "NORMAL": "value"}
```

### test_parse_empty_key_ignored

キーが空のエントリ（`=value`）は無視されること。

```text
入力: b"=bad_entry\0GOOD=value\0"
期待: {"GOOD": "value"}
```

## T3: parse_env_json テスト（nushell対応）

### test_parse_json_normal

正常なJSON出力をパースし、正しい `HashMap` が返ること。

```text
入力: {"HOME": "/Users/test", "PATH": "/usr/bin:/bin"}
期待: {"HOME": "/Users/test", "PATH": "/usr/bin:/bin"}
```

### test_parse_json_nested_values_as_string

nushellが値をネストされたオブジェクトとして返す場合（例: PATH）、文字列変換されること。

### test_parse_json_empty_object

空のJSONオブジェクト `{}` の場合、空の `HashMap` が返ること。

### test_parse_json_invalid

無効なJSON入力の場合、エラーが返ること。

## T4: シェル種別判定テスト

### test_detect_bash

`/bin/bash` を `ShellType::Bash` と判定すること。

### test_detect_zsh

`/bin/zsh` を `ShellType::Zsh` と判定すること。

### test_detect_fish

`/usr/bin/fish` や `/opt/homebrew/bin/fish` を `ShellType::Fish` と判定すること。

### test_detect_nushell

`/usr/bin/nu` や `/opt/homebrew/bin/nu` を `ShellType::Nushell` と判定すること。

### test_detect_unknown_falls_back_to_sh

不明なシェル（例: `/usr/bin/unknown-shell`）は `ShellType::Sh` にフォールバックすること。

### test_build_command_bash

bash シェルに対して `["/bin/bash", "-l", "-c", "env -0"]` コマンドが構築されること。

### test_build_command_zsh

zsh シェルに対して `["/bin/zsh", "-l", "-c", "env -0"]` コマンドが構築されること。

### test_build_command_fish

fish シェルに対して `["/usr/bin/fish", "-l", "-c", "env -0"]` コマンドが構築されること。

### test_build_command_nushell

nushell に対して `["/usr/bin/nu", "-l", "-c", "$env | to json"]` コマンドが構築されること。

### test_build_command_sh_fallback

不明シェルに対して `["/bin/sh", "-l", "-c", "env -0"]` コマンドが構築されること。

## T5: capture_login_shell_env テスト

### test_fallback_on_invalid_shell

`$SHELL` が存在しないパスの場合、`std::env::vars()` にフォールバックし、`EnvSource::StdEnvFallback` が返ること。

### test_fallback_on_timeout

シェルが応答しない場合（モックで `sleep` を使用）、5秒後にフォールバックすること。

### test_shell_not_set_uses_bin_sh

`$SHELL` 未設定時に `/bin/sh` が使用されること。

### test_windows_uses_std_env

Windows 環境では直接 `std::env::vars()` を使用すること（`#[cfg(target_os = "windows")]`）。

### test_env_source_login_shell_on_success

正常にシェルから取得できた場合、`EnvSource::LoginShell` が返ること。

## T9: 三層マージテスト

### test_merge_os_base_only

OS環境変数のみの場合、そのまま全変数が返ること。

```text
OS: {"PATH": "/usr/bin", "HOME": "/Users/test"}
Profile: {env: {}, disabled_env: []}
Overrides: {}
期待: {"PATH": "/usr/bin", "HOME": "/Users/test"}
```

### test_merge_profile_overrides_os

profiles.toml の `env` が OS環境変数を上書きすること。

```text
OS: {"PATH": "/usr/bin", "MY_VAR": "os-value"}
Profile: {env: {"MY_VAR": "profile-value"}, disabled_env: []}
Overrides: {}
期待: {"PATH": "/usr/bin", "MY_VAR": "profile-value"}
```

### test_merge_disabled_env_removes_os

profiles.toml の `disabled_env` が OS環境変数を削除すること。

```text
OS: {"PATH": "/usr/bin", "SECRET": "sensitive"}
Profile: {env: {}, disabled_env: ["SECRET"]}
Overrides: {}
期待: {"PATH": "/usr/bin"} (SECRET は存在しない)
```

### test_merge_overrides_win_over_all

`env_overrides` が全ての層を上書きすること。

```text
OS: {"MY_VAR": "os"}
Profile: {env: {"MY_VAR": "profile"}, disabled_env: []}
Overrides: {"MY_VAR": "override"}
期待: {"MY_VAR": "override"}
```

### test_merge_overrides_restore_disabled

`disabled_env` で削除された変数を `env_overrides` で復活させられること。

```text
OS: {"SECRET": "original"}
Profile: {env: {}, disabled_env: ["SECRET"]}
Overrides: {"SECRET": "restored"}
期待: {"SECRET": "restored"}
```

### test_merge_glm_overrides_os

GLMプロバイダー設定がOS環境変数の同名変数を上書きすること。

```text
OS: {"ANTHROPIC_BASE_URL": "https://os-api.example.com"}
GLM設定: {"ANTHROPIC_BASE_URL": "https://glm.example.com"}
期待: {"ANTHROPIC_BASE_URL": "https://glm.example.com"}
```

### test_merge_gwt_context_vars_added

gwt コンテキスト変数（GWT_PROJECT_ROOT等）が正しく追加されること。

## T10: merge_profile_env テスト

### test_profile_env_adds_new_vars

プロファイルで追加された変数がOS環境変数に追加されること。

### test_profile_disabled_env_removes_vars

`disabled_env` の変数がOS環境変数から削除されること。

### test_empty_profile_returns_os_unchanged

空のプロファイルの場合、OS環境変数がそのまま返ること。

### test_disabled_env_case_sensitive

`disabled_env` は大文字小文字を区別すること（Unix環境変数はcase-sensitive）。

## T11: EnvSnapshot廃止後のrunner.rsテスト

### test_resolve_command_uses_os_env_path

OS環境変数のPATHを使ってコマンドが解決されること（EnvSnapshotなし）。

### test_resolve_command_prefers_global_over_node_modules

従来通り、node_modules/.bin よりグローバルインストールが優先されること。

### test_resolve_command_empty_path_still_checks_common_locations

PATHが空でも /usr/local/bin 等の共通パスが検索されること。
