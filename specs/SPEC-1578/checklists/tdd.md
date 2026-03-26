### テストシナリオ

| ID | シナリオ | カテゴリ |
|----|---------|---------|
| TDD-001 | 空の exclude ファイルにマネージドブロックが追加される | exclude |
| TDD-002 | 既存マネージドブロックが新ブロックに差し替えられる | exclude |
| TDD-003 | ユーザー独自ルールが保持される | exclude |
| TDD-004 | レガシーパターンが除去される | exclude |
| TDD-005 | 2 回実行で結果が同一（冪等性） | exclude |
| TDD-006 | 入れ子 BEGIN マーカーでエラー | exclude |
| TDD-007 | END なし BEGIN でエラー | exclude |
| TDD-008 | BEGIN なし END でエラー | exclude |
| TDD-009 | worktree で commondir に書き込まれる | exclude |
| TDD-010 | スキルファイルが正しい内容で配置される | asset |
| TDD-011 | プレースホルダが置換される | asset |
| TDD-012 | UNIX でスクリプトに実行権限が付与される | asset |
| TDD-013 | settings.local.json にフック定義が登録される | settings |
| TDD-014 | settings.local.json の既存設定が破壊されない | settings |

### テストコード（RED 状態）

```csharp
// SkillRegistrationTests.cs
using NUnit.Framework;
using System.IO;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class ProjectLocalExcludeTests
    {
        private string _tempDir;
        private string _gitInfoDir;
        private string _excludePath;

        [SetUp]
        public void SetUp()
        {
            _tempDir = Path.Combine(Path.GetTempPath(), Path.GetRandomFileName());
            _gitInfoDir = Path.Combine(_tempDir, ".git", "info");
            Directory.CreateDirectory(_gitInfoDir);
            _excludePath = Path.Combine(_gitInfoDir, "exclude");
        }

        [TearDown]
        public void TearDown()
        {
            if (Directory.Exists(_tempDir))
                Directory.Delete(_tempDir, true);
        }

        [Test]
        public void EmptyExclude_AddsManagedBlock()
        {
            File.WriteAllText(_excludePath, "");
            var service = new SkillRegistrationService();
            service.EnsureProjectLocalExcludeRules(_tempDir);
            var content = File.ReadAllText(_excludePath);
            Assert.That(content, Does.Contain("# BEGIN gwt managed local assets"));
            Assert.That(content, Does.Contain("# END gwt managed local assets"));
            Assert.That(content, Does.Contain("/.codex/skills/gwt-*/"));
            Assert.That(content, Does.Contain("/.gemini/skills/gwt-*/"));
            Assert.That(content, Does.Contain("/.claude/skills/gwt-*/"));
            Assert.That(content, Does.Contain("/.claude/commands/gwt-*.md"));
            Assert.That(content, Does.Contain("/.claude/hooks/scripts/gwt-*.sh"));
        }

        [Test]
        public void ExistingManagedBlock_IsReplaced()
        {
            File.WriteAllText(_excludePath,
                "# BEGIN gwt managed local assets\nold-pattern\n# END gwt managed local assets\n");
            var service = new SkillRegistrationService();
            service.EnsureProjectLocalExcludeRules(_tempDir);
            var content = File.ReadAllText(_excludePath);
            Assert.That(content, Does.Not.Contain("old-pattern"));
            Assert.That(content, Does.Contain("/.claude/skills/gwt-*/"));
        }

        [Test]
        public void UserRules_ArePreserved()
        {
            File.WriteAllText(_excludePath,
                "# custom rule\ncustom-pattern\n# BEGIN gwt managed local assets\nold\n# END gwt managed local assets\nanother-pattern\n");
            var service = new SkillRegistrationService();
            service.EnsureProjectLocalExcludeRules(_tempDir);
            var content = File.ReadAllText(_excludePath);
            Assert.That(content, Does.Contain("# custom rule"));
            Assert.That(content, Does.Contain("custom-pattern"));
            Assert.That(content, Does.Contain("another-pattern"));
        }

        [Test]
        public void LegacyPatterns_AreRemoved()
        {
            File.WriteAllText(_excludePath,
                "/.codex/skills/gwt-*/**\n.gwt/\n/.gwt/\nuser-rule\n");
            var service = new SkillRegistrationService();
            service.EnsureProjectLocalExcludeRules(_tempDir);
            var content = File.ReadAllText(_excludePath);
            Assert.That(content, Does.Not.Contain("/.codex/skills/gwt-*/**"));
            Assert.That(content, Does.Not.Contain(".gwt/"));
            Assert.That(content, Does.Contain("user-rule"));
            Assert.That(content, Does.Contain("/.codex/skills/gwt-*/"));
        }

        [Test]
        public void Idempotent_SecondRunProducesSameResult()
        {
            File.WriteAllText(_excludePath, "");
            var service = new SkillRegistrationService();
            service.EnsureProjectLocalExcludeRules(_tempDir);
            var first = File.ReadAllText(_excludePath);
            service.EnsureProjectLocalExcludeRules(_tempDir);
            var second = File.ReadAllText(_excludePath);
            Assert.That(second, Is.EqualTo(first));
        }

        [Test]
        public void NestedBeginMarker_ReturnsError()
        {
            File.WriteAllText(_excludePath,
                "# BEGIN gwt managed local assets\n# BEGIN gwt managed local assets\n# END gwt managed local assets\n");
            var service = new SkillRegistrationService();
            Assert.Throws<SkillRegistrationException>(() =>
                service.EnsureProjectLocalExcludeRules(_tempDir));
        }

        [Test]
        public void UnterminatedBeginMarker_ReturnsError()
        {
            File.WriteAllText(_excludePath,
                "# BEGIN gwt managed local assets\nsome-rule\n");
            var service = new SkillRegistrationService();
            Assert.Throws<SkillRegistrationException>(() =>
                service.EnsureProjectLocalExcludeRules(_tempDir));
        }

        [Test]
        public void EndWithoutBegin_ReturnsError()
        {
            File.WriteAllText(_excludePath,
                "# END gwt managed local assets\n");
            var service = new SkillRegistrationService();
            Assert.Throws<SkillRegistrationException>(() =>
                service.EnsureProjectLocalExcludeRules(_tempDir));
        }
    }

    [TestFixture]
    public class SettingsJsonRegistrationTests
    {
        [Test]
        public void EmptySettings_AddsHookDefinitions()
        {
            var tempDir = Path.Combine(Path.GetTempPath(), Path.GetRandomFileName());
            var claudeDir = Path.Combine(tempDir, ".claude");
            Directory.CreateDirectory(claudeDir);
            var settingsPath = Path.Combine(claudeDir, "settings.local.json");
            File.WriteAllText(settingsPath, "{}");

            var service = new SkillRegistrationService();
            service.EnsureSettingsLocalJson(tempDir);

            var content = File.ReadAllText(settingsPath);
            Assert.That(content, Does.Contain("UserPromptSubmit"));
            Assert.That(content, Does.Contain("PreToolUse"));

            Directory.Delete(tempDir, true);
        }

        [Test]
        public void ExistingSettings_PreservesUserConfig()
        {
            var tempDir = Path.Combine(Path.GetTempPath(), Path.GetRandomFileName());
            var claudeDir = Path.Combine(tempDir, ".claude");
            Directory.CreateDirectory(claudeDir);
            var settingsPath = Path.Combine(claudeDir, "settings.local.json");
            File.WriteAllText(settingsPath, "{\"customSetting\": true}");

            var service = new SkillRegistrationService();
            service.EnsureSettingsLocalJson(tempDir);

            var content = File.ReadAllText(settingsPath);
            Assert.That(content, Does.Contain("\"customSetting\""));

            Directory.Delete(tempDir, true);
        }
    }
}
```

---
