using Cysharp.Threading.Tasks;
using System;
using System.Diagnostics;
using System.Text;
using System.Threading;

namespace Gwt.Core.Services.Git
{
    public class GitCommandRunner
    {
        public async UniTask<(string stdout, string stderr, int exitCode)> RunAsync(
            string args, string workingDir, CancellationToken ct = default, int timeoutMs = 30000)
        {
            var psi = new ProcessStartInfo
            {
                FileName = "git",
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
            var stdoutBuilder = new StringBuilder();
            var stderrBuilder = new StringBuilder();

            process.OutputDataReceived += (_, e) =>
            {
                if (e.Data != null) stdoutBuilder.AppendLine(e.Data);
            };
            process.ErrorDataReceived += (_, e) =>
            {
                if (e.Data != null) stderrBuilder.AppendLine(e.Data);
            };

            process.Start();
            process.BeginOutputReadLine();
            process.BeginErrorReadLine();

            using var timeoutCts = new CancellationTokenSource(timeoutMs);
            using var linkedCts = CancellationTokenSource.CreateLinkedTokenSource(ct, timeoutCts.Token);

            try
            {
                while (!process.HasExited)
                {
                    linkedCts.Token.ThrowIfCancellationRequested();
                    await UniTask.Delay(50, cancellationToken: linkedCts.Token);
                }

                process.WaitForExit();
            }
            catch (OperationCanceledException)
            {
                try { if (!process.HasExited) process.Kill(); } catch { }
                throw;
            }

            return (stdoutBuilder.ToString(), stderrBuilder.ToString(), process.ExitCode);
        }
    }
}
