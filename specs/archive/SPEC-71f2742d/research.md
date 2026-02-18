# 調査レポート: カスタムコーディングエージェント登録機能

**仕様ID**: `SPEC-71f2742d` | **調査日**: 2026-01-26

## 1. 既存のコードベース分析

### 1.1 現在の技術スタック

- **言語**: Rust 2021 Edition (stable)
- **CLI UI**: ratatui 0.29 + crossterm 0.28
- **シリアライズ**: serde, serde_json
- **日時**: chrono
- **パス**: directories, dirs
- **設定**: figment (TOML + 環境変数)

### 1.2 関連モジュール構造

```text
crates/
├── gwt-cli/src/
│   ├── main.rs              # エージェント起動ロジック (build_agent_args, launch_agent)
│   └── tui/
│       ├── app.rs           # アプリ状態管理
│       └── screens/
│           └── wizard.rs    # CodingAgent enum、Wizard状態、UI描画
└── gwt-core/src/
    ├── agent/
    │   └── mod.rs           # AgentType enum、AgentManager
    ├── ai/
    │   └── agent_history.rs # AgentHistoryStore（履歴保存）
    └── config/
        ├── settings.rs      # Settings、AgentSettings
        ├── profile.rs       # Profile、ProfilesConfig
        └── ts_session.rs    # ToolSessionEntry（セッション履歴）
```

### 1.3 既存のCodingAgent実装

`wizard.rs` に固定の4種エージェントがハードコード：

```rust
pub enum CodingAgent {
    ClaudeCode,
    CodexCli,
    GeminiCli,
    OpenCode,
}

impl CodingAgent {
    pub fn all() -> &'static [CodingAgent] { ... }  // 固定配列
    pub fn label(&self) -> &'static str { ... }
    pub fn id(&self) -> &'static str { ... }
    pub fn color(&self) -> Color { ... }
    pub fn npm_package(&self) -> &'static str { ... }
    pub fn command_name(&self) -> &'static str { ... }
    pub fn models(&self) -> Vec<ModelOption> { ... }
}
```

### 1.4 エージェント起動フロー

1. `WizardState` でエージェント選択、モデル選択、バージョン選択
2. `AgentLaunchConfig` を構築
3. `build_agent_args()` でエージェント固有の引数を生成
4. `build_launch_plan()` で実行計画を作成
5. `launch_agent()` または tmux 経由で起動

### 1.5 設定ファイル構造

現在の設定読み込み優先順位：

1. `.gwt.toml` (プロジェクトローカル)
2. `.gwt/config.toml` (プロジェクトローカル)
3. `~/.config/gwt/config.toml` (グローバル)

## 2. 技術的決定

### 2.1 tools.json 配置場所

**決定**: ~/.gwt/tools.json (グローバル) と .gwt/tools.json (ローカル)

**理由**:
- 既存の ~/.gwt/ ディレクトリを活用
- プロジェクト固有の設定は .gwt/ に統一

### 2.2 エージェント表現方式

**決定**: 新しい `UnifiedAgent` trait + `CustomAgent` 構造体

**理由**:
- 既存の `CodingAgent` enum は拡張困難（バリアント追加が必要）
- trait object でビルトインとカスタムを統一的に扱う
- `CodingAgent` は `BuiltinAgent` として trait を実装

### 2.3 設定ファイル形式

**決定**: JSON (tools.json)

**理由**:
- 仕様で JSON と明示
- serde_json は既存依存
- カスタムエージェント定義に適切な構造

### 2.4 タブ切り替え実装

**決定**: 既存の Screen enum に Settings バリアント追加

**理由**:
- 最小限の変更で実装可能
- 既存のタブ切り替えロジック（Tab キー処理）を拡張

## 3. 制約と依存関係

### 3.1 制約

1. **後方互換性**: 既存の CodingAgent enum は残す（trait 実装を追加）
2. **パフォーマンス**: Wizard 開始時の tools.json 読み込みは非同期不要（起動時のみ）
3. **メモリ**: カスタムエージェント数は現実的に数十個程度を想定

### 3.2 依存関係

1. **serde_json**: 既存（バージョン確認不要）
2. **directories/dirs**: 既存（~/.gwt 取得用）
3. **追加依存なし**: 現状の依存で実装可能

## 4. 実装アプローチ

### 4.1 Phase 1: データ層

1. `gwt-core/src/config/tools.rs` 新規作成
   - `ToolsConfig`、`CustomCodingAgent` 構造体
   - 読み込み・マージ・バリデーションロジック

### 4.2 Phase 2: エージェント抽象化

1. `gwt-cli/src/tui/screens/wizard.rs` 修正
   - `AgentEntry` trait または enum を導入
   - ビルトインとカスタムを統一表示

### 4.3 Phase 3: 起動ロジック

1. `gwt-cli/src/main.rs` 修正
   - `build_agent_args()` をカスタムエージェント対応に拡張
   - type (command/path/bunx) による分岐

### 4.4 Phase 4: TUI設定画面

1. `gwt-cli/src/tui/screens/settings.rs` 新規作成
   - カスタムエージェント管理UI
   - Profile設定との統合

### 4.5 Phase 5: 履歴統合

1. `gwt-core/src/ai/agent_history.rs` 修正
   - カスタムエージェントID対応（既存構造で対応可能）

## 5. リスク評価

### 5.1 技術的リスク

| リスク | 影響度 | 対策 |
|--------|--------|------|
| CodingAgent enum との互換性破壊 | 高 | trait 抽象化で既存コード維持 |
| Wizard状態管理の複雑化 | 中 | 統一エージェントリストで単純化 |
| tools.json パースエラー | 低 | バリデーション＋フォールバック |

### 5.2 依存関係リスク

- bunx 未インストール時の type: "bunx" → グレーアウト表示で対応
- カスタムコマンド未インストール → "Not installed" 表示で対応

## 6. 結論

既存コードベースの分析により、以下のアプローチが最適と判断：

1. **データ層**: 新規 `tools.rs` モジュールで tools.json 管理
2. **表示層**: `wizard.rs` の `CodingAgent::all()` を動的リストに拡張
3. **起動層**: `build_agent_args()` のカスタムエージェント分岐追加
4. **設定UI**: 新規 `settings.rs` スクリーンでTUI管理

追加の外部依存なしで実装可能。
