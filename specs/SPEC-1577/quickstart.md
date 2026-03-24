1. `crates/gwt-core/` に `assistant/tools/` ディレクトリを作成
2. `AssistantToolDefinition`, `AssistantToolHandler`, `AssistantToolRegistry` trait を定義
3. `ToolPermissionLevel` enum, `ToolResult` struct を実装
4. 最小ツール（`codebase_read_file`）を実装して Responses API の tools 形式出力を確認
5. Tauri の状態管理に `AssistantToolRegistry` を Singleton 登録

---
