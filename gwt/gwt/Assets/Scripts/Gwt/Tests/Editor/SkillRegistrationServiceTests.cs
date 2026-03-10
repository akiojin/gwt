using System;
using System.Collections;
using System.IO;
using System.Linq;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Agent.Services.SkillRegistration;
using NUnit.Framework;
using UnityEngine.TestTools;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class SkillRegistrationServiceTests
    {
        private string _tmpRoot;
        private SkillRegistrationService _service;

        [SetUp]
        public void SetUp()
        {
            _tmpRoot = Path.Combine(Path.GetTempPath(), "gwt-skill-test-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(_tmpRoot);
            _service = new SkillRegistrationService();
        }

        [TearDown]
        public void TearDown()
        {
            if (Directory.Exists(_tmpRoot))
                Directory.Delete(_tmpRoot, true);
        }

        [UnityTest]
        public IEnumerator RegisterAll_WritesSkillFilesToAllAgentRoots() => UniTask.ToCoroutine(async () =>
        {
            await _service.RegisterAllAsync(_tmpRoot, CancellationToken.None);

            var claudeSkill = Path.Combine(_tmpRoot, ".claude", "skills", "gwt-spec-ops", "SKILL.md");
            var codexSkill = Path.Combine(_tmpRoot, ".codex", "skills", "gwt-spec-ops", "SKILL.md");
            var geminiSkill = Path.Combine(_tmpRoot, ".gemini", "skills", "gwt-spec-ops", "SKILL.md");

            Assert.IsTrue(File.Exists(claudeSkill), $"Expected: {claudeSkill}");
            Assert.IsTrue(File.Exists(codexSkill), $"Expected: {codexSkill}");
            Assert.IsTrue(File.Exists(geminiSkill), $"Expected: {geminiSkill}");
        });

        [UnityTest]
        public IEnumerator RegisterAll_WritesClaudeCommandsOnlyToClaudeRoot() => UniTask.ToCoroutine(async () =>
        {
            await _service.RegisterAllAsync(_tmpRoot, CancellationToken.None);

            var claudeCmd = Path.Combine(_tmpRoot, ".claude", "commands", "gwt-spec-ops.md");
            Assert.IsTrue(File.Exists(claudeCmd), $"Expected: {claudeCmd}");

            var codexCmds = Path.Combine(_tmpRoot, ".codex", "commands");
            var geminiCmds = Path.Combine(_tmpRoot, ".gemini", "commands");
            Assert.IsFalse(Directory.Exists(codexCmds), "Codex should not have commands directory");
            Assert.IsFalse(Directory.Exists(geminiCmds), "Gemini should not have commands directory");
        });

        [UnityTest]
        public IEnumerator RegisterAll_WritesClaudeHooksOnlyToClaudeRoot() => UniTask.ToCoroutine(async () =>
        {
            await _service.RegisterAllAsync(_tmpRoot, CancellationToken.None);

            var hookPath = Path.Combine(_tmpRoot, ".claude", "hooks", "scripts", "gwt-forward-hook.sh");
            Assert.IsTrue(File.Exists(hookPath), $"Expected: {hookPath}");

            // Check executable on macOS/Linux
            #if UNITY_EDITOR_OSX || UNITY_EDITOR_LINUX
            var info = new System.Diagnostics.ProcessStartInfo("test", $"-x \"{hookPath}\"")
            {
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true,
            };
            using var proc = System.Diagnostics.Process.Start(info);
            proc.WaitForExit(5000);
            Assert.AreEqual(0, proc.ExitCode, "Hook script should be executable");
            #endif

            var codexHooks = Path.Combine(_tmpRoot, ".codex", "hooks");
            var geminiHooks = Path.Combine(_tmpRoot, ".gemini", "hooks");
            Assert.IsFalse(Directory.Exists(codexHooks), "Codex should not have hooks directory");
            Assert.IsFalse(Directory.Exists(geminiHooks), "Gemini should not have hooks directory");
        });

        [UnityTest]
        public IEnumerator RegisterAll_CleansLegacyAssets() => UniTask.ToCoroutine(async () =>
        {
            // Pre-create legacy assets
            var legacySkillDir = Path.Combine(_tmpRoot, ".claude", "skills", "gwt-fix-issue");
            Directory.CreateDirectory(legacySkillDir);
            File.WriteAllText(Path.Combine(legacySkillDir, "SKILL.md"), "old");

            var legacyCmdDir = Path.Combine(_tmpRoot, ".claude", "commands");
            Directory.CreateDirectory(legacyCmdDir);
            File.WriteAllText(Path.Combine(legacyCmdDir, "gwt-issue-spec-ops.md"), "old");

            await _service.RegisterAllAsync(_tmpRoot, CancellationToken.None);

            Assert.IsFalse(Directory.Exists(legacySkillDir), "Legacy skill directory should be deleted");
            Assert.IsFalse(File.Exists(Path.Combine(legacyCmdDir, "gwt-issue-spec-ops.md")),
                "Legacy command file should be deleted");
        });

        [UnityTest]
        public IEnumerator RegisterAll_CreatesExcludeRules() => UniTask.ToCoroutine(async () =>
        {
            var gitInfoDir = Path.Combine(_tmpRoot, ".git", "info");
            Directory.CreateDirectory(gitInfoDir);

            await _service.RegisterAllAsync(_tmpRoot, CancellationToken.None);

            var excludePath = Path.Combine(gitInfoDir, "exclude");
            Assert.IsTrue(File.Exists(excludePath), "exclude file should exist");

            var content = File.ReadAllText(excludePath);
            Assert.IsTrue(content.Contains("# BEGIN gwt managed local assets"),
                "Should contain BEGIN marker");
            Assert.IsTrue(content.Contains("# END gwt managed local assets"),
                "Should contain END marker");
        });

        [UnityTest]
        public IEnumerator RegisterAgent_Claude_WritesSkillsCommandsHooks() => UniTask.ToCoroutine(async () =>
        {
            await _service.RegisterAgentAsync(SkillAgentType.Claude, _tmpRoot, CancellationToken.None);

            var claudeRoot = Path.Combine(_tmpRoot, ".claude");

            // Skills
            Assert.IsTrue(Directory.Exists(Path.Combine(claudeRoot, "skills")),
                "Claude should have skills directory");
            Assert.IsTrue(File.Exists(Path.Combine(claudeRoot, "skills", "gwt-spec-ops", "SKILL.md")),
                "Claude should have gwt-spec-ops skill");

            // Commands
            Assert.IsTrue(Directory.Exists(Path.Combine(claudeRoot, "commands")),
                "Claude should have commands directory");
            Assert.IsTrue(File.Exists(Path.Combine(claudeRoot, "commands", "gwt-spec-ops.md")),
                "Claude should have gwt-spec-ops command");

            // Hooks
            Assert.IsTrue(Directory.Exists(Path.Combine(claudeRoot, "hooks", "scripts")),
                "Claude should have hooks/scripts directory");
            Assert.IsTrue(File.Exists(Path.Combine(claudeRoot, "hooks", "scripts", "gwt-forward-hook.sh")),
                "Claude should have gwt-forward-hook.sh");
        });

        [UnityTest]
        public IEnumerator RegisterAgent_Codex_WritesSkillsOnly() => UniTask.ToCoroutine(async () =>
        {
            await _service.RegisterAgentAsync(SkillAgentType.Codex, _tmpRoot, CancellationToken.None);

            var codexRoot = Path.Combine(_tmpRoot, ".codex");

            // Skills should exist
            Assert.IsTrue(Directory.Exists(Path.Combine(codexRoot, "skills")),
                "Codex should have skills directory");
            Assert.IsTrue(File.Exists(Path.Combine(codexRoot, "skills", "gwt-spec-ops", "SKILL.md")),
                "Codex should have gwt-spec-ops skill");

            // Commands and hooks should NOT exist
            Assert.IsFalse(Directory.Exists(Path.Combine(codexRoot, "commands")),
                "Codex should not have commands directory");
            Assert.IsFalse(Directory.Exists(Path.Combine(codexRoot, "hooks")),
                "Codex should not have hooks directory");
        });

        [UnityTest]
        public IEnumerator GetStatus_ReturnsOk_WhenAllFilesPresent() => UniTask.ToCoroutine(async () =>
        {
            await _service.RegisterAllAsync(_tmpRoot, CancellationToken.None);

            var status = _service.GetStatus(_tmpRoot);

            Assert.AreEqual("ok", status.Overall);
            Assert.IsTrue(status.Agents.All(a => a.Registered),
                "All agents should be registered");
        });

        [UnityTest]
        public IEnumerator GetStatus_ReturnsDegraded_WhenSomeFilesMissing() => UniTask.ToCoroutine(async () =>
        {
            await _service.RegisterAllAsync(_tmpRoot, CancellationToken.None);

            // Delete one skill file from Claude
            var skillFile = Path.Combine(_tmpRoot, ".claude", "skills", "gwt-spec-ops", "SKILL.md");
            if (File.Exists(skillFile))
                File.Delete(skillFile);

            var status = _service.GetStatus(_tmpRoot);

            Assert.AreEqual("degraded", status.Overall);
        });

        [UnityTest]
        public IEnumerator WriteAsset_RewritesPluginRootPlaceholder() => UniTask.ToCoroutine(async () =>
        {
            await _service.RegisterAgentAsync(SkillAgentType.Claude, _tmpRoot, CancellationToken.None);

            // Find a skill file that has RewriteForProject=true
            var skillFile = Path.Combine(_tmpRoot, ".claude", "skills", "gwt-spec-ops", "SKILL.md");
            Assert.IsTrue(File.Exists(skillFile), $"Skill file should exist: {skillFile}");

            var content = File.ReadAllText(skillFile);
            Assert.IsFalse(content.Contains("${CLAUDE_PLUGIN_ROOT}"),
                "Placeholder should be replaced");
        });
    }
}
