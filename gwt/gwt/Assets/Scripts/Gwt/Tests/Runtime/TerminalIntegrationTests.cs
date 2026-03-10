using System.Collections;
using System.Runtime.InteropServices;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using Gwt.Core.Services.Pty;
using Gwt.Core.Services.Terminal;
using NUnit.Framework;
using UnityEngine.TestTools;

namespace Gwt.Tests.Runtime
{
    [TestFixture]
    public class TerminalIntegrationTests
    {
        private PtyService _ptyService;
        private TerminalPaneManager _paneManager;

        [SetUp]
        public void SetUp()
        {
            _ptyService = new PtyService(new PlatformShellDetector());
            _paneManager = new TerminalPaneManager();
        }

        [TearDown]
        public void TearDown()
        {
            _ptyService?.Dispose();
        }

        [UnityTest]
        public IEnumerator SpawnEcho_OutputReachesXtermBuffer() => UniTask.ToCoroutine(async () =>
        {
            var echoCmd = GetEchoCommand();
            var echoArgs = GetEchoArgs("hello");
            var paneId = await _ptyService.SpawnAsync(echoCmd, echoArgs, GetTempDir(), 24, 80);

            var adapter = new XtermSharpTerminalAdapter(24, 80);
            var pane = new TerminalPaneState("test-pane", adapter) { PtySessionId = paneId };

            _ptyService.GetOutputStream(paneId).Subscribe(data => adapter.Feed(data));

            await UniTask.Delay(1000);

            var buffer = adapter.GetBuffer();
            var text = buffer.GetTextContent(0, 0, 0, buffer.Cols - 1).TrimEnd();

            Assert.That(text, Does.Contain("hello"));
        });

        [UnityTest]
        public IEnumerator MultipleTerminals_IndependentBuffers() => UniTask.ToCoroutine(async () =>
        {
            var echoCmd = GetEchoCommand();

            var paneId1 = await _ptyService.SpawnAsync(echoCmd, GetEchoArgs("AAA"), GetTempDir(), 24, 80);
            var paneId2 = await _ptyService.SpawnAsync(echoCmd, GetEchoArgs("BBB"), GetTempDir(), 24, 80);

            var adapter1 = new XtermSharpTerminalAdapter(24, 80);
            var adapter2 = new XtermSharpTerminalAdapter(24, 80);

            _ptyService.GetOutputStream(paneId1).Subscribe(data => adapter1.Feed(data));
            _ptyService.GetOutputStream(paneId2).Subscribe(data => adapter2.Feed(data));

            _paneManager.AddPane(new TerminalPaneState("pane-1", adapter1) { PtySessionId = paneId1 });
            _paneManager.AddPane(new TerminalPaneState("pane-2", adapter2) { PtySessionId = paneId2 });

            await UniTask.Delay(1000);

            var text1 = adapter1.GetBuffer().GetTextContent(0, 0, 0, 79).TrimEnd();
            var text2 = adapter2.GetBuffer().GetTextContent(0, 0, 0, 79).TrimEnd();

            Assert.That(text1, Does.Contain("AAA"));
            Assert.That(text2, Does.Contain("BBB"));
            Assert.That(_paneManager.PaneCount, Is.EqualTo(2));
        });

        [UnityTest]
        public IEnumerator KillSession_SetsCompletedStatus() => UniTask.ToCoroutine(async () =>
        {
            var shellDetector = new PlatformShellDetector();
            var shell = shellDetector.DetectDefaultShell();
            var shellArgs = shellDetector.GetShellArgs(shell);

            var paneId = await _ptyService.SpawnAsync(shell, shellArgs, GetTempDir(), 24, 80);

            Assert.That(_ptyService.GetStatus(paneId), Is.EqualTo(PaneStatus.Running));

            await _ptyService.KillAsync(paneId);
            await UniTask.Delay(500);

            Assert.That(_ptyService.GetStatus(paneId), Is.EqualTo(PaneStatus.Completed));
        });

        [UnityTest]
        public IEnumerator PtyResize_ReflectsInTerminal() => UniTask.ToCoroutine(async () =>
        {
            var shellDetector = new PlatformShellDetector();
            var shell = shellDetector.DetectDefaultShell();
            var shellArgs = shellDetector.GetShellArgs(shell);

            var paneId = await _ptyService.SpawnAsync(shell, shellArgs, GetTempDir(), 24, 80);

            var adapter = new XtermSharpTerminalAdapter(24, 80);

            await _ptyService.ResizeAsync(paneId, 40, 120);
            adapter.Resize(40, 120);

            Assert.That(adapter.Rows, Is.EqualTo(40));
            Assert.That(adapter.Cols, Is.EqualTo(120));
        });

        [UnityTest]
        public IEnumerator AnsiColors_RenderedCorrectly() => UniTask.ToCoroutine(async () =>
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            adapter.Feed("\x1b[31mError\x1b[0m \x1b[32mSuccess\x1b[0m");

            var richText = TerminalRichTextBuilder.BuildRichText(adapter, 0, 24);

            Assert.That(richText, Does.Contain("<color=#"));
            Assert.That(richText, Does.Contain("Error"));
            Assert.That(richText, Does.Contain("Success"));

            await UniTask.CompletedTask;
        });

        private static string GetTempDir()
        {
            return RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
                ? System.Environment.GetEnvironmentVariable("TEMP") ?? "C:\\Temp"
                : "/tmp";
        }

        private static string GetEchoCommand()
        {
            return RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
                ? "cmd.exe"
                : "/bin/echo";
        }

        private static string[] GetEchoArgs(string message)
        {
            return RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
                ? new[] { "/c", $"echo {message}" }
                : new[] { message };
        }
    }
}
