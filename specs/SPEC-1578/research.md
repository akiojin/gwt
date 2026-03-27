### Rust 実装の構造（移植元）

| 関数 | 役割 |
|------|------|
| `ensure_skills_for_project` | エントリポイント: 全アセット配置 + exclude + settings.local.json |
| `ensure_project_local_exclude_rules` | .git/info/exclude マネージドブロック管理 |
| `git_path_for_project_root` | worktree commondir 解決 |
| `rewrite_project_asset_content` | プレースホルダ置換 |
| `managed_hooks_definition` | settings.local.json フック定義生成 |
| `merge_managed_claude_hooks_into_settings` | settings.local.json マージ書き込み |

### Unity でのアセット埋め込み方式

| 方式 | メリット | デメリット | 採用 |
|------|---------|-----------|------|
| TextAsset (Resources) | `Resources.Load<TextAsset>()` で簡単に読込 | Resources フォルダの制約 | △ |
| StreamingAssets | ビルド後もファイルとしてアクセス可能 | パス解決が面倒 | × |
| C# const/static 埋め込み | Rust の `include_str!` と同等、依存なし | 変更時に再コンパイル必要 | ○ |

**採用**: C# の `const string` / `static readonly string` として直接埋め込む（Rust 実装と同パターン）。

---
