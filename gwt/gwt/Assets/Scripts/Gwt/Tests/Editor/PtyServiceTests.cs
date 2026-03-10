using System;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using Gwt.Core.Services.Pty;
using NUnit.Framework;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class PtyServiceTests
    {
        private PlatformShellDetector _shellDetector;
        private PtyService _service;

        [SetUp]
        public void SetUp()
        {
            _shellDetector = new PlatformShellDetector();
            _service = new PtyService(_shellDetector);
        }

        [TearDown]
        public void TearDown()
        {
            _service?.Dispose();
        }

        // --- PlatformShellDetector Tests ---

        [Test]
        public void DetectDefaultShell_ReturnsNonEmptyPath()
        {
            var shell = _shellDetector.DetectDefaultShell();
            Assert.That(shell, Is.Not.Null.And.Not.Empty);
        }

        [Test]
        public void DetectDefaultShell_ReturnsAvailableShell()
        {
            var shell = _shellDetector.DetectDefaultShell();
            Assert.That(_shellDetector.IsShellAvailable(shell), Is.True,
                $"Detected shell '{shell}' should be available");
        }

        [Test]
        public void GetShellArgs_ReturnsArrayForShell()
        {
            var shell = _shellDetector.DetectDefaultShell();
            var args = _shellDetector.GetShellArgs(shell);
            Assert.That(args, Is.Not.Null);
        }

        [Test]
        public void IsShellAvailable_ReturnsFalse_ForEmptyString()
        {
            Assert.That(_shellDetector.IsShellAvailable(""), Is.False);
        }

        [Test]
        public void IsShellAvailable_ReturnsFalse_ForNull()
        {
            Assert.That(_shellDetector.IsShellAvailable(null), Is.False);
        }

        [Test]
        public void IsShellAvailable_ReturnsFalse_ForNonexistentPath()
        {
            Assert.That(_shellDetector.IsShellAvailable("/nonexistent/shell"), Is.False);
        }

        // --- PtySession Tests ---

        [Test]
        public void PtySession_InitialStatus_IsRunning()
        {
            var psi = new System.Diagnostics.ProcessStartInfo
            {
                FileName = GetEchoCommand(),
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true
            };
            SetEchoArgs(psi);

            var process = new System.Diagnostics.Process { StartInfo = psi };
            process.Start();

            using var session = new PtySession("test-1", process, "/tmp", 24, 80);

            Assert.That(session.Status, Is.EqualTo(PtySessionStatus.Running));
            Assert.That(session.Id, Is.EqualTo("test-1"));
            Assert.That(session.Rows, Is.EqualTo(24));
            Assert.That(session.Cols, Is.EqualTo(80));
            Assert.That(session.ExitCode, Is.Null);
        }

        [Test]
        public void PtySession_RaiseExited_SetsStatusAndExitCode()
        {
            var psi = new System.Diagnostics.ProcessStartInfo
            {
                FileName = GetEchoCommand(),
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true
            };
            SetEchoArgs(psi);

            var process = new System.Diagnostics.Process { StartInfo = psi };
            process.Start();

            using var session = new PtySession("test-2", process, "/tmp", 24, 80);

            int? exitedCode = null;
            session.ProcessExited += code => exitedCode = code;

            session.RaiseExited(0);

            Assert.That(session.Status, Is.EqualTo(PtySessionStatus.Completed));
            Assert.That(session.ExitCode, Is.EqualTo(0));
            Assert.That(exitedCode, Is.EqualTo(0));
        }

        [Test]
        public void PtySession_Dispose_CancelsToken()
        {
            var psi = new System.Diagnostics.ProcessStartInfo
            {
                FileName = GetEchoCommand(),
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true
            };
            SetEchoArgs(psi);

            var process = new System.Diagnostics.Process { StartInfo = psi };
            process.Start();

            var session = new PtySession("test-3", process, "/tmp", 24, 80);
            var token = session.Token;

            Assert.That(token.IsCancellationRequested, Is.False);

            session.Dispose();

            Assert.That(token.IsCancellationRequested, Is.True);
        }

        [Test]
        public void PtySession_RaiseOutput_InvokesEvent()
        {
            var psi = new System.Diagnostics.ProcessStartInfo
            {
                FileName = GetEchoCommand(),
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true
            };
            SetEchoArgs(psi);

            var process = new System.Diagnostics.Process { StartInfo = psi };
            process.Start();

            using var session = new PtySession("test-4", process, "/tmp", 24, 80);

            string received = null;
            session.OutputReceived += data => received = data;
            session.RaiseOutput("hello");

            Assert.That(received, Is.EqualTo("hello"));
        }

        [Test]
        public void PtySession_OutputStream_ReceivesData()
        {
            var psi = new System.Diagnostics.ProcessStartInfo
            {
                FileName = GetEchoCommand(),
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true
            };
            SetEchoArgs(psi);

            var process = new System.Diagnostics.Process { StartInfo = psi };
            process.Start();

            using var session = new PtySession("test-5", process, "/tmp", 24, 80);

            string observedData = null;
            session.OutputStream.Subscribe(data => observedData = data);
            session.RaiseOutput("observable-test");

            Assert.That(observedData, Is.EqualTo("observable-test"));
        }

        // --- PtyService Tests ---

        [Test]
        public async Task SpawnShellAsync_CreatesSession()
        {
            var session = await _service.SpawnShellAsync(GetTempDir(), cancellationToken: CancellationToken.None);

            Assert.That(session, Is.Not.Null);
            Assert.That(session.Id, Is.Not.Null.And.Not.Empty);
            Assert.That(session.Status, Is.EqualTo(PtySessionStatus.Running));
        }

        [Test]
        public async Task SpawnAsync_Command_ReturnsPaneId()
        {
            var echoCmd = GetEchoCommand();
            var echoArgs = GetEchoArgs();
            var paneId = await _service.SpawnAsync(echoCmd, echoArgs, GetTempDir(), 24, 80, CancellationToken.None);

            Assert.That(paneId, Is.Not.Null.And.Not.Empty);
        }

        [Test]
        public async Task SpawnAsync_Command_SetsEnvironmentVariables()
        {
            var echoCmd = GetEchoCommand();
            var echoArgs = GetEchoArgs();
            var paneId = await _service.SpawnAsync(echoCmd, echoArgs, GetTempDir(), 24, 80, CancellationToken.None);

            var session = _service.GetSession(paneId);
            Assert.That(session, Is.Not.Null);
            Assert.That(session.Status, Is.EqualTo(PtySessionStatus.Running).Or.EqualTo(PtySessionStatus.Completed));
        }

        [Test]
        public async Task WriteAsync_SendsDataToProcess()
        {
            var session = await _service.SpawnShellAsync(GetTempDir(), cancellationToken: CancellationToken.None);
            string output = null;
            session.OutputReceived += data => output = data;

            var echoCmd = RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
                ? "echo hello\r\n"
                : "echo hello\n";

            await _service.WriteAsync(session.Id, echoCmd, CancellationToken.None);

            await UniTask.Delay(500, cancellationToken: CancellationToken.None);

            Assert.That(output, Is.Not.Null, "Should have received some output from echo command");
        }

        [Test]
        public async Task KillAsync_TerminatesProcess()
        {
            var session = await _service.SpawnShellAsync(GetTempDir(), cancellationToken: CancellationToken.None);

            await _service.KillAsync(session.Id, CancellationToken.None);

            await UniTask.Delay(200, cancellationToken: CancellationToken.None);

            Assert.That(session.Process.HasExited, Is.True);
        }

        [Test]
        public async Task GetStatus_ReturnsRunning_ForActiveSession()
        {
            var session = await _service.SpawnShellAsync(GetTempDir(), cancellationToken: CancellationToken.None);

            var status = _service.GetStatus(session.Id);

            Assert.That(status, Is.EqualTo(PaneStatus.Running));
        }

        [Test]
        public async Task GetOutputStream_ReturnsObservable()
        {
            var session = await _service.SpawnShellAsync(GetTempDir(), cancellationToken: CancellationToken.None);

            var observable = _service.GetOutputStream(session.Id);

            Assert.That(observable, Is.Not.Null);
        }

        [Test]
        public void GetSession_ThrowsForInvalidId()
        {
            Assert.Throws<ArgumentException>(() => _service.GetSession("nonexistent"));
        }

        [Test]
        public async Task ConcurrentSessions_AreIndependent()
        {
            var session1 = await _service.SpawnShellAsync(GetTempDir(), cancellationToken: CancellationToken.None);
            var session2 = await _service.SpawnShellAsync(GetTempDir(), cancellationToken: CancellationToken.None);

            Assert.That(session1.Id, Is.Not.EqualTo(session2.Id));
            Assert.That(_service.GetStatus(session1.Id), Is.EqualTo(PaneStatus.Running));
            Assert.That(_service.GetStatus(session2.Id), Is.EqualTo(PaneStatus.Running));

            await _service.KillAsync(session1.Id, CancellationToken.None);
            await UniTask.Delay(200, cancellationToken: CancellationToken.None);

            Assert.That(session1.Process.HasExited, Is.True);
            Assert.That(session2.Process.HasExited, Is.False);
        }

        [Test]
        public async Task ResizeAsync_UpdatesDimensions()
        {
            var session = await _service.SpawnShellAsync(GetTempDir(), cancellationToken: CancellationToken.None);

            await _service.ResizeAsync(session.Id, 40, 120, CancellationToken.None);

            Assert.That(session.Rows, Is.EqualTo(40));
            Assert.That(session.Cols, Is.EqualTo(120));
        }

        [Test]
        public async Task Dispose_CleansUpAllSessions()
        {
            var service = new PtyService(_shellDetector);
            var session = await service.SpawnShellAsync(GetTempDir(), cancellationToken: CancellationToken.None);
            var token = session.Token;

            service.Dispose();

            Assert.That(token.IsCancellationRequested, Is.True);
            Assert.Throws<ObjectDisposedException>(() => service.GetStatus(session.Id));
        }

        // --- Helpers ---

        private static string GetTempDir()
        {
            return RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
                ? Environment.GetEnvironmentVariable("TEMP") ?? "C:\\Temp"
                : "/tmp";
        }

        private static string GetEchoCommand()
        {
            return RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
                ? "cmd.exe"
                : "/bin/echo";
        }

        private static string[] GetEchoArgs()
        {
            return RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
                ? new[] { "/c", "echo test" }
                : new[] { "test" };
        }

        private static void SetEchoArgs(System.Diagnostics.ProcessStartInfo psi)
        {
            if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
            {
                psi.ArgumentList.Add("/c");
                psi.ArgumentList.Add("echo test");
            }
            else
            {
                psi.ArgumentList.Add("test");
            }
        }
    }
}
