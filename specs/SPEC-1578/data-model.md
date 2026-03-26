### Core Types

| Name | Kind | Fields | Notes |
|------|------|--------|-------|
| `ISkillRegistrationService` | interface | `EnsureSkillsForProject(projectRoot)`, `EnsureProjectLocalExcludeRules(projectRoot)`, `EnsureSettingsLocalJson(projectRoot)` | メインサービス |
| `ManagedAsset` | class | `RelativePath`, `Body`, `Executable`, `RewriteForProject` | 配置アセット定義 |
| `SkillRegistrationException` | class | `Message`, `ExcludePath` | エラー（不正マーカー等） |
| `ExcludeBlockState` | enum | `Normal`, `InManagedBlock` | exclude パース状態 |

### Constants

| Name | Value |
|------|-------|
| `MANAGED_BLOCK_BEGIN` | `# BEGIN gwt managed local assets` |
| `MANAGED_BLOCK_END` | `# END gwt managed local assets` |
| `EXCLUDE_PATTERNS` | `/.codex/skills/gwt-*/`, `/.gemini/skills/gwt-*/`, `/.claude/skills/gwt-*/`, `/.claude/commands/gwt-*.md`, `/.claude/hooks/scripts/gwt-*.sh` |
| `LEGACY_PATTERNS` | `.gwt/`, `/.gwt/`, `/.codex/skills/gwt-*/**` |

---
