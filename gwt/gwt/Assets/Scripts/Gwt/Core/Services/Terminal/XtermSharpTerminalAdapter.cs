using System;
using System.Collections.Concurrent;
using System.Threading;

namespace Gwt.Core.Services.Terminal
{
    /// <summary>
    /// Adapter wrapping the existing TerminalEmulator.
    /// Thread-safe: Feed() can be called from any thread;
    /// ProcessPendingData() must be called from the main thread (e.g., in Update).
    /// </summary>
    public class XtermSharpTerminalAdapter : IDisposable
    {
        private readonly TerminalEmulator _emulator;
        private readonly ConcurrentQueue<string> _pendingData = new();
        private readonly int _mainThreadId;

        public event Action BufferChanged;
        public event Action<string> TitleChanged;

        public int Rows => _emulator.Rows;
        public int Cols => _emulator.Cols;

        public XtermSharpTerminalAdapter(int rows = 24, int cols = 80)
        {
            _mainThreadId = Environment.CurrentManagedThreadId;
            _emulator = new TerminalEmulator(rows, cols);
            _emulator.BufferChanged += () => BufferChanged?.Invoke();
            _emulator.TitleChanged += t => TitleChanged?.Invoke(t);
        }

        /// <summary>
        /// Enqueue data from any thread. Call ProcessPendingData() on main thread to apply.
        /// </summary>
        public void Feed(string data)
        {
            if (Environment.CurrentManagedThreadId == _mainThreadId)
            {
                _emulator.Write(data);
                return;
            }

            _pendingData.Enqueue(data);
        }

        /// <summary>
        /// Process all pending data on the main thread. Returns true if any data was processed.
        /// </summary>
        public bool ProcessPendingData()
        {
            var processed = false;
            while (_pendingData.TryDequeue(out var data))
            {
                _emulator.Write(data);
                processed = true;
            }
            return processed;
        }

        public void Resize(int rows, int cols)
        {
            _emulator.Resize(rows, cols);
        }

        public TerminalBuffer GetBuffer()
        {
            return _emulator.Buffer;
        }

        public void Dispose()
        {
            BufferChanged = null;
            TitleChanged = null;
            _emulator.Dispose();
        }
    }
}
