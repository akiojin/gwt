using System;

namespace Gwt.Core.Services.Terminal
{
    /// <summary>
    /// Adapter wrapping the existing TerminalEmulator.
    /// Designed to be swapped with XtermSharp Terminal when the package is verified for Unity.
    /// </summary>
    public class XtermSharpTerminalAdapter : IDisposable
    {
        private readonly TerminalEmulator _emulator;

        public event Action BufferChanged;
        public event Action<string> TitleChanged;

        public int Rows => _emulator.Rows;
        public int Cols => _emulator.Cols;

        public XtermSharpTerminalAdapter(int rows = 24, int cols = 80)
        {
            _emulator = new TerminalEmulator(rows, cols);
            _emulator.BufferChanged += () => BufferChanged?.Invoke();
            _emulator.TitleChanged += t => TitleChanged?.Invoke(t);
        }

        public void Feed(string data)
        {
            _emulator.Write(data);
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
