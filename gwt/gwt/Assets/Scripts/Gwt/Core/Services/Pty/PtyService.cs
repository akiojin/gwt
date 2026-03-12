using System;
using System.Collections.Concurrent;
using System.Diagnostics;
using System.Linq;
using System.Runtime.InteropServices;
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
        private readonly string _scriptPath;
        private bool _disposed;

        public PtyService(IPlatformShellDetector shellDetector)
        {
            _shellDetector = shellDetector;
            _scriptPath = DetectScriptPath();
        }

        public async UniTask<string> SpawnAsync(
            string command, string[] args, string workingDir,
            int rows, int cols, CancellationToken ct = default)
        {
            ThrowIfDisposed();

            var (psi, usesPseudoTerminal) = CreateStartInfo(command, args, workingDir, rows, cols);

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
            var session = new PtySession(id, process, workingDir, rows, cols, usesPseudoTerminal);
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

            await session.StandardInputLock.WaitAsync(ct);
            try
            {
                if (!TryGetStandardInput(session.Process, out var writer))
                    return;

                try
                {
                    await writer.WriteAsync(data.AsMemory(), ct);
                    await writer.FlushAsync();
                }
                catch (Exception e) when (IsIgnorableStandardInputException(e))
                {
                    return;
                }
            }
            finally
            {
                session.StandardInputLock.Release();
            }
        }

        public async UniTask ResizeAsync(string paneId, int rows, int cols, CancellationToken ct = default)
        {
            ThrowIfDisposed();

            var session = GetSession(paneId);
            session.Rows = rows;
            session.Cols = cols;

            if (!session.UsesPseudoTerminal || session.Process.HasExited)
                return;

            await session.StandardInputLock.WaitAsync(ct);
            try
            {
                if (!TryGetStandardInput(session.Process, out var writer))
                    return;

                try
                {
                    await writer.WriteAsync($"stty rows {rows} cols {cols}\n".AsMemory(), ct);
                    await writer.FlushAsync();
                }
                catch (Exception e) when (IsIgnorableStandardInputException(e))
                {
                    return;
                }
            }
            finally
            {
                session.StandardInputLock.Release();
            }
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

                    var chunk = SanitizeOutput(new string(buffer, 0, read));
                    if (!string.IsNullOrEmpty(chunk))
                        session.RaiseOutput(chunk);
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

        private (ProcessStartInfo startInfo, bool usesPseudoTerminal) CreateStartInfo(
            string command,
            string[] args,
            string workingDir,
            int rows,
            int cols)
        {
            if (CanUsePseudoTerminal())
                return (CreateUnixPtyStartInfo(command, args, workingDir, rows, cols), true);

            return (CreateRedirectedStartInfo(command, args, workingDir), false);
        }

        private ProcessStartInfo CreateRedirectedStartInfo(string command, string[] args, string workingDir)
        {
            var psi = CreateBaseStartInfo(command, workingDir);
            if (args != null)
            {
                foreach (var arg in args)
                    psi.ArgumentList.Add(arg);
            }

            return psi;
        }

        private ProcessStartInfo CreateUnixPtyStartInfo(
            string command,
            string[] args,
            string workingDir,
            int rows,
            int cols)
        {
            var psi = CreateBaseStartInfo(_scriptPath, workingDir);
            psi.ArgumentList.Add("-q");
            psi.ArgumentList.Add("/dev/null");
            psi.ArgumentList.Add("/bin/sh");
            psi.ArgumentList.Add("-lc");
            psi.ArgumentList.Add(BuildUnixLaunchCommand(command, args, rows, cols));
            return psi;
        }

        private static ProcessStartInfo CreateBaseStartInfo(string command, string workingDir)
        {
            var psi = new ProcessStartInfo
            {
                FileName = command,
                WorkingDirectory = string.IsNullOrWhiteSpace(workingDir) ? Environment.CurrentDirectory : workingDir,
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
            return psi;
        }

        private static string BuildUnixLaunchCommand(string command, string[] args, int rows, int cols)
        {
            var parts = new System.Collections.Generic.List<string>
            {
                "export TERM=xterm-256color",
                "export FORCE_COLOR=1"
            };

            if (rows > 0 && cols > 0)
                parts.Add($"stty rows {rows} cols {cols} >/dev/null 2>&1 || true");

            var escapedCommand = EscapeShellArgument(command);
            var escapedArgs = args == null || args.Length == 0
                ? string.Empty
                : " " + string.Join(" ", args.Select(EscapeShellArgument));
            parts.Add($"exec {escapedCommand}{escapedArgs}");
            return string.Join("; ", parts);
        }

        private bool CanUsePseudoTerminal()
        {
            return !RuntimeInformation.IsOSPlatform(OSPlatform.Windows) && !string.IsNullOrWhiteSpace(_scriptPath);
        }

        private static string DetectScriptPath()
        {
            if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
                return string.Empty;

            var candidates = new[] { "/usr/bin/script", "/bin/script" };
            return candidates.FirstOrDefault(System.IO.File.Exists) ?? string.Empty;
        }

        private static string EscapeShellArgument(string value)
        {
            var input = value ?? string.Empty;
            return $"'{input.Replace("'", "'\"'\"'")}'";
        }

        private static bool TryGetStandardInput(Process process, out System.IO.StreamWriter writer)
        {
            writer = null;
            if (process == null)
                return false;

            try
            {
                writer = process.StandardInput;
                return writer != null;
            }
            catch (InvalidOperationException)
            {
                return false;
            }
        }

        private static bool IsIgnorableStandardInputException(Exception exception)
        {
            if (exception is ObjectDisposedException)
                return true;

            if (exception is InvalidOperationException && IsIgnorableStandardInputMessage(exception.Message))
                return true;

            return exception?.InnerException != null && IsIgnorableStandardInputException(exception.InnerException);
        }

        private static bool IsIgnorableStandardInputMessage(string message)
        {
            if (string.IsNullOrWhiteSpace(message))
                return false;

            return message.IndexOf("StandardIn has not been redirected", StringComparison.OrdinalIgnoreCase) >= 0 ||
                message.IndexOf("Standard input has not been redirected", StringComparison.OrdinalIgnoreCase) >= 0 ||
                message.IndexOf("input stream is not writable", StringComparison.OrdinalIgnoreCase) >= 0 ||
                message.IndexOf("cannot write to a closed", StringComparison.OrdinalIgnoreCase) >= 0;
        }

        private static string SanitizeOutput(string output)
        {
            return string.IsNullOrEmpty(output)
                ? string.Empty
                : output.Replace("\u0004\b\b", string.Empty);
        }
    }
}
