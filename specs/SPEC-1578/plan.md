### アプローチ

1. Rust 側の現行実装（`skill_registration.rs`）を分析し、ロジックを完全に把握する
2. C# に移植する際、Unity 固有の制約（TextAsset, Resources, StreamingAssets）を考慮する
3. `ISkillRegistrationService` として VContainer に DI 登録する
4. プロジェクトオープンイベントでスキル登録を自動トリガーする

### 設計方針

- **Rust 実装の忠実な移植**: ロジック（exclude ブロック差し替え、レガシー移行、worktree commondir 解決）は Rust 実装をそのまま C# に移植する
- **アセット埋め込み**: `include_str!` 相当は C# の `const string` / `static readonly string` として直接埋め込む（Rust 実装と同パターン、依存なし）
- **冪等性保証**: 何回実行しても同じ結果になることをテストで保証する

### リスク

| リスク | 影響 | 対策 |
|--------|------|------|
| Unity の `System.IO` 制約 | ファイル操作の互換性 | .NET Standard 2.1 API 範囲で実装、Unity 固有制約を事前検証 |
| worktree の commondir 解決 | `git rev-parse` コマンド依存 | `System.Diagnostics.Process` で git コマンドを呼び出す（既存パターン踏襲） |
| Windows のパス区切り | exclude パターンの `/` と `\` | パターンは常に `/` で記述（Git の仕様に準拠） |

---
