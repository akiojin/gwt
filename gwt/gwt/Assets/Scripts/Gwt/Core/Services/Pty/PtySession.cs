using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
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
        public bool UsesPseudoTerminal { get; }
        public PtySessionStatus Status { get; set; }
        public int? ExitCode { get; private set; }
        public IObservable<string> OutputStream { get; }
        public event Action<string> OutputReceived;
        public event Action<int> ProcessExited;

        private readonly PtyOutputStream _outputStream = new();
        private readonly CancellationTokenSource _cts = new();
        private bool _disposed;

        public PtySession(string id, Process process, string workingDir, int rows, int cols, bool usesPseudoTerminal = false)
        {
            Id = id;
            Process = process;
            WorkingDir = workingDir;
            Rows = rows;
            Cols = cols;
            UsesPseudoTerminal = usesPseudoTerminal;
            Status = PtySessionStatus.Running;
            OutputStream = _outputStream;
        }

        public void RaiseOutput(string data)
        {
            _outputStream.OnNext(data);
            OutputReceived?.Invoke(data);
        }

        public void RaiseExited(int exitCode)
        {
            ExitCode = exitCode;
            Status = PtySessionStatus.Completed;
            _outputStream.OnCompleted();
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

    internal class PtyOutputStream : IObservable<string>
    {
        private readonly object _lock = new();
        private readonly List<IObserver<string>> _observers = new();

        public IDisposable Subscribe(IObserver<string> observer)
        {
            lock (_lock)
            {
                _observers.Add(observer);
            }
            return new Unsubscriber(this, observer);
        }

        public void OnNext(string value)
        {
            IObserver<string>[] snapshot;
            lock (_lock)
            {
                snapshot = _observers.ToArray();
            }
            foreach (var observer in snapshot)
                observer.OnNext(value);
        }

        public void OnCompleted()
        {
            IObserver<string>[] snapshot;
            lock (_lock)
            {
                snapshot = _observers.ToArray();
            }
            foreach (var observer in snapshot)
                observer.OnCompleted();
        }

        internal void Remove(IObserver<string> observer)
        {
            lock (_lock)
            {
                _observers.Remove(observer);
            }
        }

        private class Unsubscriber : IDisposable
        {
            private readonly PtyOutputStream _stream;
            private readonly IObserver<string> _observer;

            public Unsubscriber(PtyOutputStream stream, IObserver<string> observer)
            {
                _stream = stream;
                _observer = observer;
            }

            public void Dispose() => _stream.Remove(_observer);
        }
    }

    public static class ObservableExtensions
    {
        public static IDisposable Subscribe<T>(this IObservable<T> observable, Action<T> onNext)
        {
            return observable.Subscribe(new ActionObserver<T>(onNext));
        }

        private class ActionObserver<T> : IObserver<T>
        {
            private readonly Action<T> _onNext;
            public ActionObserver(Action<T> onNext) => _onNext = onNext;
            public void OnNext(T value) => _onNext(value);
            public void OnError(Exception error) { }
            public void OnCompleted() { }
        }
    }

    public enum PtySessionStatus
    {
        Running,
        Completed,
        Error
    }
}
