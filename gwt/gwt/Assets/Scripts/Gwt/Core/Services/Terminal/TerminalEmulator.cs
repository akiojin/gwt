using System;

namespace Gwt.Core.Services.Terminal
{
    public class TerminalEmulator : IDisposable
    {
        public TerminalBuffer Buffer { get; }
        public AnsiParser Parser { get; }
        public int Rows => Buffer.Rows;
        public int Cols => Buffer.Cols;

        public event Action<string> TitleChanged;
        public event Action BufferChanged;

        public TerminalEmulator(int rows = 24, int cols = 80)
        {
            Buffer = new TerminalBuffer(rows, cols);
            Parser = new AnsiParser(Buffer);
            Parser.TitleChanged += t => TitleChanged?.Invoke(t);
        }

        public void Write(string data)
        {
            Parser.Process(data);
            BufferChanged?.Invoke();
        }

        public void Resize(int rows, int cols)
        {
            Buffer.Resize(rows, cols);
        }

        public string GetSelectedText(int startRow, int startCol, int endRow, int endCol)
        {
            return Buffer.GetTextContent(startRow, startCol, endRow, endCol);
        }

        public void Dispose()
        {
            TitleChanged = null;
            BufferChanged = null;
        }
    }
}
