using System;
using System.Diagnostics;
using System.Threading;

namespace Gwt.Core.Services.Pty
{
    public class PtySession : IDisposable
    {
        public string Id { get; }
        public Process Process { get; }
        public string WorkingDir { get; }
        public int Rows { get; set; }
        public int Cols { get; set; }
        public PtySessionStatus Status { get; set; }
        public int? ExitCode { get; private set; }
        public event Action<string> OutputReceived;
        public event Action<int> ProcessExited;

        private readonly CancellationTokenSource _cts = new();
        private bool _disposed;

        public PtySession(string id, Process process, string workingDir, int rows, int cols)
        {
            Id = id;
            Process = process;
            WorkingDir = workingDir;
            Rows = rows;
            Cols = cols;
            Status = PtySessionStatus.Running;
        }

        public void RaiseOutput(string data) => OutputReceived?.Invoke(data);

        public void RaiseExited(int exitCode)
        {
            ExitCode = exitCode;
            Status = PtySessionStatus.Completed;
            ProcessExited?.Invoke(exitCode);
        }

        public CancellationToken Token => _cts.Token;

        public void Dispose()
        {
            if (_disposed) return;
            _disposed = true;
            _cts.Cancel();
            _cts.Dispose();
            try { if (!Process.HasExited) Process.Kill(); } catch { }
            Process.Dispose();
        }
    }

    public enum PtySessionStatus
    {
        Running,
        Completed,
        Error
    }
}
