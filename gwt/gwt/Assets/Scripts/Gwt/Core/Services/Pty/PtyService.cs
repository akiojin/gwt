using System;
using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;

namespace Gwt.Core.Services.Pty
{
    public class PtyService : IPtyService, IDisposable
    {
        private static readonly TimeSpan DefaultTimeout = TimeSpan.FromSeconds(30);

        private readonly IPlatformShellDetector _shellDetector;
        private readonly ConcurrentDictionary<string, PtySession> _sessions = new();
        private bool _disposed;

        public PtyService(IPlatformShellDetector shellDetector)
        {
            _shellDetector = shellDetector;
        }

        public async UniTask<string> SpawnAsync(
            string command, string[] args, string workingDir,
            int rows, int cols, CancellationToken ct = default)
        {
            ThrowIfDisposed();

            var psi = new ProcessStartInfo
            {
                FileName = command,
                WorkingDirectory = workingDir,
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true,
                StandardOutputEncoding = Encoding.UTF8,
                StandardErrorEncoding = Encoding.UTF8
            };

            psi.Environment["TERM"] = "xterm-256color";
            psi.Environment["FORCE_COLOR"] = "1";

            if (args != null)
            {
                foreach (var arg in args)
                    psi.ArgumentList.Add(arg);
            }

            var process = new Process { StartInfo = psi, EnableRaisingEvents = true };
            try
            {
                process.Start();
            }
            catch
            {
                process.Dispose();
                throw;
            }

            var id = Guid.NewGuid().ToString("N");
            var session = new PtySession(id, process, workingDir, rows, cols);
            _sessions[id] = session;

            ReadStreamAsync(session, process.StandardOutput, ct).Forget();
            ReadStreamAsync(session, process.StandardError, ct).Forget();
            MonitorExitAsync(session, ct).Forget();

            await UniTask.CompletedTask;
            return id;
        }

        public async UniTask<PtySession> SpawnShellAsync(
            string workingDir,
            int rows = 24,
            int cols = 80,
            string shell = null,
            CancellationToken cancellationToken = default)
        {
            ThrowIfDisposed();

            shell ??= _shellDetector.DetectDefaultShell();
            var shellArgs = _shellDetector.GetShellArgs(shell);

            var paneId = await SpawnAsync(shell, shellArgs, workingDir, rows, cols, cancellationToken);
            return GetSession(paneId);
        }

        public async UniTask WriteAsync(string paneId, string data, CancellationToken ct = default)
        {
            ThrowIfDisposed();

            var session = GetSession(paneId);
            if (session.Status != PtySessionStatus.Running)
                throw new InvalidOperationException($"Session {paneId} is not running.");

            await session.Process.StandardInput.WriteAsync(data.AsMemory(), ct);
            await session.Process.StandardInput.FlushAsync();
        }

        public UniTask ResizeAsync(string paneId, int rows, int cols, CancellationToken ct = default)
        {
            ThrowIfDisposed();

            var session = GetSession(paneId);
            session.Rows = rows;
            session.Cols = cols;

            return UniTask.CompletedTask;
        }

        public async UniTask KillAsync(string paneId, CancellationToken ct = default)
        {
            ThrowIfDisposed();

            var session = GetSession(paneId);
            if (session.Process.HasExited)
                return;

            try
            {
                session.Process.Kill();
            }
            catch (InvalidOperationException)
            {
                return;
            }

            using var timeoutCts = CancellationTokenSource.CreateLinkedTokenSource(ct);
            timeoutCts.CancelAfter(DefaultTimeout);

            try
            {
                await UniTask.WaitUntil(() => session.Process.HasExited, cancellationToken: timeoutCts.Token);
            }
            catch (OperationCanceledException) when (!ct.IsCancellationRequested)
            {
                // Timeout — process didn't exit gracefully
            }
        }

        public IObservable<string> GetOutputStream(string paneId)
        {
            ThrowIfDisposed();
            return GetSession(paneId).OutputStream;
        }

        public PaneStatus GetStatus(string paneId)
        {
            ThrowIfDisposed();
            var session = GetSession(paneId);
            return session.Status switch
            {
                PtySessionStatus.Running => PaneStatus.Running,
                PtySessionStatus.Completed => PaneStatus.Completed,
                PtySessionStatus.Error => PaneStatus.Error,
                _ => PaneStatus.Error
            };
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

                await UniTask.WaitUntil(() => session.Process.HasExited, cancellationToken: linked.Token);
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
