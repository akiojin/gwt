### Detection / launch
- `DetectedAgentType`
  - `Claude`
  - `Codex`
  - `Gemini`
  - `OpenCode`
  - `GithubCopilot`
  - `Custom`
- `DetectedAgent`
  - `Type`
  - `ExecutablePath`
  - `Version`
  - `IsAvailable`
- `AgentDetector`
  - PATH 検索
  - `DetectAsync(type)`
  - `DetectAllAsync()`
  - `FindInPath(command)`

### Session state
- `AgentSessionData`
  - `Id`
  - `AgentType`
  - `WorktreePath`
  - `Branch`
  - `PtySessionId`
  - `Status`
  - `CreatedAt`
  - `UpdatedAt`
  - `AgentSessionId`
  - `Model`
  - `ToolVersion`
  - `ConversationHistory: List`
  - `AutoPrCreated: bool`
- `TerminalPaneState`
  - `PaneId`
  - `AgentSessionId`
  - `PtySessionId`
  - `Terminal`
  - `Status`

### Config models already coupled to this issue
- `CustomAgentProfile`
  - `Id`
  - `DisplayName`
  - `CliPath`
  - `DefaultArgs`
  - `WorkdirArgName`
- pending planned model
  - `JobType` enum と session への紐付けは spec 上必要だが、現コードには未導入

### Services
- `IAgentService`
  - `GetAvailableAgentsAsync`
  - `HireAgentAsync`
  - `FireAgentAsync`
  - `SendInstructionAsync`
  - `GetSessionAsync`
  - `ListSessionsAsync`
  - `RestoreSessionAsync`
  - `SaveAllSessionsAsync`
  - `ActiveSessionCount`
  - `OnAgentStatusChanged`
  - `OnAgentOutput`
- `AgentService`
  - `IPtyService` + `ITerminalPaneManager` と連携
  - `~/.gwt/sessions/*.json` 永続化

### スキル埋め込みアーキテクチャ

gwt バイナリ（Rust）および Unity Studio（C#）がエージェント起動前にスキル・コマンド・フックをプロジェクトローカルに書き出す仕組み。

#### ソース

| カテゴリ | パス | 対象 |
|---------|------|------|
| スキル | `plugins/gwt/skills/gwt-*/` | 全エージェント共有（8スキル） |
| コマンド | `plugins/gwt/commands/gwt-*.md` | Claude Code 専用（7コマンド） |
| フック | `plugins/gwt/hooks/scripts/gwt-*.sh` | Claude Code 専用（5スクリプト） |

#### 埋め込み方式

| プラットフォーム | 埋め込み | 書き出し |
|---------------|--------|---------|
| Rust（gwt バイナリ） | `include_str!()` でコンパイル時にバイナリへ埋め込み | `write_managed_assets()` でランタイムに書き出し |
| C#（Unity Studio） | `SkillAssets.generated.cs` の const 文字列 | `SkillRegistrationService.RegisterAllAsync()` でエージェント起動前に書き出し |

#### 書き出し先

| エージェント | スキル | コマンド | フック |
|-------------|--------|---------|--------|
| Claude Code | `.claude/skills/gwt-*/` | `.claude/commands/gwt-*.md` | `.claude/hooks/scripts/gwt-*.sh` |
| Codex | `.codex/skills/gwt-*/` | N/A | N/A |
| Gemini | `.gemini/skills/gwt-*/` | N/A | N/A |

#### Git 除外

`.git/info/exclude` にマネージドブロックを書き込み、生成ファイルが誤コミットされないようにする:

```
# BEGIN gwt managed local assets
/.codex/skills/gwt-*/
/.gemini/skills/gwt-*/
/.claude/skills/gwt-*/
/.claude/commands/gwt-*.md
/.claude/hooks/scripts/gwt-*.sh
# END gwt managed local assets
```

#### ヘルスチェック

登録状態を `SkillRegistrationStatus` で返す:
- **ok**: 全エージェントの全アセットが書き出し済み
- **degraded**: 一部エージェントまたはアセットが欠損
- **failed**: 書き出しに失敗、または未チェック

#### 実装ファイル

| プラットフォーム | ファイル |
|---------------|--------|
| Rust | `crates/gwt-core/src/config/skill_registration.rs` |
| C# | `gwt/gwt/Assets/Scripts/Gwt/Agent/Services/SkillRegistration/SkillRegistrationService.cs` |
| C# (生成) | `gwt/gwt/Assets/Scripts/Gwt/Agent/Services/SkillRegistration/SkillAssets.generated.cs` |
| C# (ジェネレータ) | `gwt/gwt/Assets/Editor/SkillAssetsGenerator.cs` |
