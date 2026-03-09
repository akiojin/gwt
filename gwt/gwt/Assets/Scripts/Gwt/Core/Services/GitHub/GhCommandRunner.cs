using System;
using System.Diagnostics;
using System.Text;
using System.Threading;
using Cysharp.Threading.Tasks;

namespace Gwt.Core.Services.GitHub
{
    public class GhCommandRunner
    {
        public async UniTask<(string stdout, string stderr, int exitCode)> RunAsync(
            string args, string workingDir, CancellationToken ct = default, int timeoutMs = 30000)
        {
            var psi = new ProcessStartInfo
            {
                FileName = "gh",
                Arguments = args,
                WorkingDirectory = workingDir,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true,
                StandardOutputEncoding = Encoding.UTF8,
                StandardErrorEncoding = Encoding.UTF8
            };

            using var process = new Process { StartInfo = psi };
            var stdoutBuf = new StringBuilder();
            var stderrBuf = new StringBuilder();

            process.OutputDataReceived += (_, e) =>
            {
                if (e.Data != null) stdoutBuf.AppendLine(e.Data);
            };
            process.ErrorDataReceived += (_, e) =>
            {
                if (e.Data != null) stderrBuf.AppendLine(e.Data);
            };

            process.Start();
            process.BeginOutputReadLine();
            process.BeginErrorReadLine();

            using var timeoutCts = new CancellationTokenSource(timeoutMs);
            using var linked = CancellationTokenSource.CreateLinkedTokenSource(ct, timeoutCts.Token);

            try
            {
                await UniTask.RunOnThreadPool(() =>
                {
                    while (!process.HasExited)
                    {
                        linked.Token.ThrowIfCancellationRequested();
                        process.WaitForExit(200);
                    }
                }, cancellationToken: linked.Token);
            }
            catch (OperationCanceledException)
            {
                try { if (!process.HasExited) process.Kill(); } catch { }
                throw;
            }

            process.WaitForExit();
            return (stdoutBuf.ToString().TrimEnd(), stderrBuf.ToString().TrimEnd(), process.ExitCode);
        }

        public async UniTask<string> RunJsonAsync(
            string args, string workingDir, CancellationToken ct = default)
        {
            var (stdout, stderr, exitCode) = await RunAsync(args, workingDir, ct);

            if (exitCode != 0)
            {
                throw new GhCliException(
                    $"gh command failed (exit {exitCode}): {stderr}",
                    exitCode, stderr);
            }

            return stdout;
        }
    }

    public class GhCliException : Exception
    {
        public int ExitCode { get; }
        public string Stderr { get; }

        public GhCliException(string message, int exitCode, string stderr)
            : base(message)
        {
            ExitCode = exitCode;
            Stderr = stderr;
        }
    }
}
