using System;
using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text;
using System.Threading;
using Cysharp.Threading.Tasks;

namespace Gwt.Core.Services.Pty
{
    public class PtyService : IDisposable
    {
        private static readonly TimeSpan DefaultTimeout = TimeSpan.FromSeconds(30);

        private readonly IPlatformShellDetector _shellDetector;
        private readonly ConcurrentDictionary<string, PtySession> _sessions = new();
        private bool _disposed;

        public PtyService(IPlatformShellDetector shellDetector)
        {
            _shellDetector = shellDetector;
        }

        public async UniTask<PtySession> SpawnAsync(
            string workingDir,
            int rows = 24,
            int cols = 80,
            string shell = null,
            CancellationToken cancellationToken = default)
        {
            ThrowIfDisposed();

            shell ??= _shellDetector.DetectDefaultShell();
            var args = _shellDetector.GetShellArgs(shell);

            var psi = new ProcessStartInfo
            {
                FileName = shell,
                WorkingDirectory = workingDir,
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true,
                StandardOutputEncoding = Encoding.UTF8,
                StandardErrorEncoding = Encoding.UTF8
            };

            foreach (var arg in args)
                psi.ArgumentList.Add(arg);

            var process = new Process { StartInfo = psi, EnableRaisingEvents = true };
            process.Start();

            var id = Guid.NewGuid().ToString("N");
            var session = new PtySession(id, process, workingDir, rows, cols);

            _sessions[id] = session;

            // Start async read loops
            ReadStreamAsync(session, process.StandardOutput, cancellationToken).Forget();
            ReadStreamAsync(session, process.StandardError, cancellationToken).Forget();

            // Monitor process exit
            MonitorExitAsync(session, cancellationToken).Forget();

            await UniTask.CompletedTask;
            return session;
        }

        public async UniTask WriteAsync(string sessionId, string data, CancellationToken cancellationToken = default)
        {
            ThrowIfDisposed();

            var session = GetSession(sessionId);
            if (session.Status != PtySessionStatus.Running)
                throw new InvalidOperationException($"Session {sessionId} is not running.");

            await session.Process.StandardInput.WriteAsync(data.AsMemory(), cancellationToken);
            await session.Process.StandardInput.FlushAsync();
        }

        public UniTask ResizeAsync(string sessionId, int rows, int cols, CancellationToken cancellationToken = default)
        {
            ThrowIfDisposed();

            var session = GetSession(sessionId);
            session.Rows = rows;
            session.Cols = cols;

            // Actual PTY resize requires native interop (ioctl on Unix, SetConsoleScreenBufferSize on Windows).
            // Stubbed for now — will be implemented with native PTY backend.

            return UniTask.CompletedTask;
        }

        public async UniTask KillAsync(string sessionId, CancellationToken cancellationToken = default)
        {
            ThrowIfDisposed();

            var session = GetSession(sessionId);
            if (session.Process.HasExited)
                return;

            try
            {
                session.Process.Kill(entireProcessTree: true);
            }
            catch (InvalidOperationException)
            {
                // Process already exited
                return;
            }

            // Wait for process to actually exit with timeout
            using var timeoutCts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            timeoutCts.CancelAfter(DefaultTimeout);

            try
            {
                await session.Process.WaitForExitAsync(timeoutCts.Token);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                // Timeout — process didn't exit gracefully, already killed above
            }
        }

        public PtySessionStatus GetStatus(string sessionId)
        {
            ThrowIfDisposed();
            return GetSession(sessionId).Status;
        }

        public PtySession GetSession(string sessionId)
        {
            if (!_sessions.TryGetValue(sessionId, out var session))
                throw new ArgumentException($"Session '{sessionId}' not found.", nameof(sessionId));
            return session;
        }

        public bool TryGetSession(string sessionId, out PtySession session)
        {
            return _sessions.TryGetValue(sessionId, out session);
        }

        public void RemoveSession(string sessionId)
        {
            if (_sessions.TryRemove(sessionId, out var session))
                session.Dispose();
        }

        public void Dispose()
        {
            if (_disposed) return;
            _disposed = true;

            foreach (var kvp in _sessions)
                kvp.Value.Dispose();

            _sessions.Clear();
        }

        private static async UniTaskVoid ReadStreamAsync(
            PtySession session,
            System.IO.StreamReader reader,
            CancellationToken cancellationToken)
        {
            var buffer = new char[4096];
            try
            {
                using var linked = CancellationTokenSource.CreateLinkedTokenSource(
                    cancellationToken, session.Token);

                while (!linked.Token.IsCancellationRequested)
                {
                    var read = await reader.ReadAsync(buffer.AsMemory(), linked.Token);
                    if (read == 0) break;

                    session.RaiseOutput(new string(buffer, 0, read));
                }
            }
            catch (OperationCanceledException) { }
            catch (ObjectDisposedException) { }
        }

        private static async UniTaskVoid MonitorExitAsync(
            PtySession session,
            CancellationToken cancellationToken)
        {
            try
            {
                using var linked = CancellationTokenSource.CreateLinkedTokenSource(
                    cancellationToken, session.Token);

                await session.Process.WaitForExitAsync(linked.Token);
                session.RaiseExited(session.Process.ExitCode);
            }
            catch (OperationCanceledException) { }
            catch (ObjectDisposedException) { }
        }

        private void ThrowIfDisposed()
        {
            if (_disposed) throw new ObjectDisposedException(nameof(PtyService));
        }
    }
}
