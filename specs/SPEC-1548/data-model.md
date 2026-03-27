### UI State

| Name | Kind | Fields | Notes |
|---|---|---|---|
| `UIManager` | MonoBehaviour | `_consolePanel`, `_leadInputField`, `_projectInfoBar`, `_gitDetailPanel`, `_issueDetailPanel`, `_agentSettingsPanel`, `_terminalOverlayPanel`, `_settingsMenu` | UI ルート管理 |
| `OverlayPanel` | base class | `IsOpen`, `_panel` | 全オーバーレイの共通開閉制御 |
| `ConsolePanel` | MonoBehaviour | `_buffer`, `_categoryBuffer`, `_currentFilter`, `_autoScroll` | RTS 風ログ表示 |
| `LeadInputField` | MonoBehaviour | `_inputField`, `OnLeadCommand` | HUD 常時表示の Lead 指示入力 |
| `ProjectInfoBar` | MonoBehaviour | project metadata fields | プロジェクト名 / Active Agent 数 / PR 数等 |
| `SettingsMenuController` | MonoBehaviour | `_settingsPanel`, `_resumeButton`, `_quitButton`, `_isPaused` | ESC メニュー |
| `TerminalOverlayPanel` | OverlayPanel | `_terminalRenderer`, `_terminalInputField`, `_terminalTabBar` | フローティングターミナル |
| `IssueDetailPanel` / `GitDetailPanel` | OverlayPanel | UI 参照群 | Issue / Git 詳細表示 |

### Focus Model

- `terminal focus` 中は terminal input 優先
- `studio focus` 中は HUD / shortcut 優先
- ESC はフォーカス解除ではなく、上位 overlay close に使う
- パネル外クリックでフォーカス解除

### Notification Model

- console log: 履歴保持
- toast / world-space marker: 即時通知
- OS notification: アプリ非フォーカス時の補助通知
- `ConsolePanel.AddMessage(category, text, color)` を基礎 API とする
- **エラー通知3段階**: Error=トースト+コンソール、Warning=コンソールのみ、Info=ログのみ

### GFM Parser Model

- `GfmParser`: 自前 GFM パーサー（Markdig 不使用）
- `GfmNode`: パース結果の AST ノード（Heading, Paragraph, List, CodeBlock, Table, Checkbox, Link, Image 等）
- `TmpRichTextRenderer`: GfmNode → TextMeshPro リッチテキスト変換
