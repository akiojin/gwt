### 背景

OpenAI互換API設定の運用において、Profiles 構成の揺れ（`default` 不在、`default.ai` 未設定）により
設定画面・バックエンド解決ロジックで分岐が増え、挙動が不安定になる。

### ユーザーシナリオとテスト（受け入れシナリオ）

**US-1: default profile は常に存在する** [P0]

- 前提: 既存 profiles に `default` がない（または空）
- 操作: Profiles を保存/読込する
- 期待: `default` profile が必ず存在する

**US-2: default profile は常に AI 設定を持つ** [P0]

- 前提: `profiles.default.ai` が `null` / 未設定
- 操作: Profiles を保存/読込する
- 期待: `profiles.default.ai` が必ず存在する（API key は空で可）

**US-3: API key は任意のまま維持される** [P0]

- 前提: `profiles.default.ai.api_key` が空
- 操作: Profiles を保存/読込する
- 期待: エラーにならず、空API keyを許容する

### 機能要件

| ID | 要件 |
|----|------|
| FR-001 | `ProfilesConfig` は保存時/読込時に `default` profile 不在を自動補正しなければならない |
| FR-002 | `profiles.default.ai` が未設定の場合、デフォルトAI設定を自動補完しなければならない |
| FR-003 | `profiles.default.ai.api_key` は空文字を許容しなければならない |
| FR-004 | `default` 以外の profile には AI 設定を強制しない |

### 非機能要件

| ID | 要件 |
|----|------|
| NFR-001 | 変更は `crates/gwt-core/src/config/profile.rs` を中心とした最小範囲に限定する |
| NFR-002 | 回帰防止として config 単体テストを追加し、RED→GREEN を確認する |

### 成功基準

| ID | 基準 |
|----|------|
| SC-001 | `default` 欠落 config の保存/読込で `default` が補完されるテストがGREEN |
| SC-002 | `profiles.default.ai` 欠落 config の保存/読込で AI設定が補完されるテストがGREEN |
| SC-003 | 補完後も `api_key` 空文字を許容するテストがGREEN |
