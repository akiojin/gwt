### Assembly 構成

```
Gwt.Core.asmdef          # コアサービス (PTY, Git, GitHub, Data, Error Handling)
Gwt.Agent.asmdef          # エージェント管理 + Lead
Gwt.Studio.asmdef         # スタジオ + エンティティ + HUD
Gwt.AI.asmdef             # AI API + 音声
Gwt.Infra.asmdef          # Docker, Build, Index
Gwt.Lifecycle.asmdef      # プロジェクトLC + マルチプロジェクト
Gwt.Shared.asmdef         # 共有ユーティリティ
Gwt.Tests.Editor.asmdef   # EditMode テスト
Gwt.Tests.Runtime.asmdef  # PlayMode テスト
```

### 主要インターフェース一覧

| Assembly | インターフェース | 責務 |
|---------|---------------|------|
| Gwt.Core | `IPtyService` | PTY 生成・入出力・リサイズ・終了 |
| Gwt.Core | `IPlatformShellDetector` | OS 別シェル検出 |
| Gwt.Core | `IGitService` | git CLI ラッパー |
| Gwt.Core | `IGitHubService` | gh CLI ラッパー |
| Gwt.Core | `IConfigService` | 設定ファイル読み書き (JSON) |
| Gwt.Core | `ISessionService` | セッション永続化・復元 |
| Gwt.Core | `IRecentProjectsService` | 最近開いたプロジェクト履歴 |
| Gwt.Core | `IErrorHandlingService` | エラーハンドリング基盤 |
| Gwt.Agent | `IAgentService` | エージェント管理 |
| Gwt.Agent | `ILeadService` | Lead 協調オーケストレーション |
| Gwt.AI | `IAIService` | OpenAI 互換 API 呼び出し |
| Gwt.Studio | `IStudioService` | スタジオ生成・管理 |
| Gwt.Studio | `IEntityService` | エンティティ CRUD |
| Gwt.Lifecycle | `IProjectService` | プロジェクト開閉・作成 |
| Gwt.Lifecycle | `IMultiProjectService` | プロジェクト切替 |

### データ永続化構造

```
~/.gwt/
├── config/
│   ├── settings.json       # グローバル設定
│   ├── profiles.json       # AI プロバイダー認証
│   └── agent-config.json   # エージェント設定
├── sessions/               # セッション永続化 (JSON)
├── logs/                   # アプリケーションログ
└── recent-projects.json    # 最近開いたプロジェクト
<project>/.gwt/
└── project.json            # プロジェクト固有設定
```
