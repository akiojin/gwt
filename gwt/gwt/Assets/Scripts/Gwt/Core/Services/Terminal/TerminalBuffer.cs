using System.Collections.Generic;
using System.Text;

namespace Gwt.Core.Services.Terminal
{
    public struct TerminalCell
    {
        public char Character;
        public byte ForegroundColor;
        public byte BackgroundColor;
        public bool Bold;
        public bool Italic;
        public bool Underline;
        public bool Inverse;

        public static TerminalCell Empty => new()
        {
            Character = ' ',
            ForegroundColor = 7,
            BackgroundColor = 0
        };
    }

    public class TerminalBuffer
    {
        public int Rows { get; private set; }
        public int Cols { get; private set; }
        public int CursorRow { get; set; }
        public int CursorCol { get; set; }
        public int ScrollbackLines => _scrollback.Count;

        private TerminalCell[][] _screen;
        private readonly List<TerminalCell[]> _scrollback = new();
        private const int MaxScrollback = 10000;

        public byte CurrentFg = 7;
        public byte CurrentBg = 0;
        public bool Bold, Italic, Underline, Inverse;

        public TerminalBuffer(int rows, int cols)
        {
            Rows = rows;
            Cols = cols;
            _screen = CreateScreen(rows, cols);
        }

        public void Resize(int newRows, int newCols)
        {
            var newScreen = CreateScreen(newRows, newCols);
            int copyRows = System.Math.Min(Rows, newRows);
            int copyCols = System.Math.Min(Cols, newCols);

            for (int r = 0; r < copyRows; r++)
            {
                for (int c = 0; c < copyCols; c++)
                {
                    newScreen[r][c] = _screen[r][c];
                }
            }

            _screen = newScreen;
            Rows = newRows;
            Cols = newCols;
            CursorRow = System.Math.Min(CursorRow, newRows - 1);
            CursorCol = System.Math.Min(CursorCol, newCols - 1);
        }

        public TerminalCell GetCell(int row, int col) => _screen[row][col];

        public TerminalCell[] GetScrollbackLine(int index) => _scrollback[index];

        public void SetCell(int row, int col, TerminalCell cell)
        {
            _screen[row][col] = cell;
        }

        public void WriteChar(char c)
        {
            if (CursorCol >= Cols)
            {
                CursorCol = 0;
                CursorRow++;
                if (CursorRow >= Rows)
                {
                    ScrollUp();
                    CursorRow = Rows - 1;
                }
            }

            _screen[CursorRow][CursorCol] = new TerminalCell
            {
                Character = c,
                ForegroundColor = Inverse ? CurrentBg : CurrentFg,
                BackgroundColor = Inverse ? CurrentFg : CurrentBg,
                Bold = Bold,
                Italic = Italic,
                Underline = Underline,
                Inverse = Inverse
            };
            CursorCol++;
        }

        public void Linefeed()
        {
            CursorRow++;
            if (CursorRow >= Rows)
            {
                ScrollUp();
                CursorRow = Rows - 1;
            }
        }

        public void CarriageReturn()
        {
            CursorCol = 0;
        }

        public void ScrollUp()
        {
            // Move top line to scrollback
            _scrollback.Add(_screen[0]);
            if (_scrollback.Count > MaxScrollback)
                _scrollback.RemoveAt(0);

            // Shift screen up
            for (int r = 0; r < Rows - 1; r++)
            {
                _screen[r] = _screen[r + 1];
            }

            // Clear bottom line
            _screen[Rows - 1] = CreateEmptyLine(Cols);
        }

        public void ScrollDown()
        {
            // Shift screen down, clear top line
            for (int r = Rows - 1; r > 0; r--)
            {
                _screen[r] = _screen[r - 1];
            }

            _screen[0] = CreateEmptyLine(Cols);
        }

        public void ClearScreen()
        {
            for (int r = 0; r < Rows; r++)
            {
                _screen[r] = CreateEmptyLine(Cols);
            }
        }

        public void ClearLine(int row)
        {
            _screen[row] = CreateEmptyLine(Cols);
        }

        public void ClearToEndOfLine()
        {
            for (int c = CursorCol; c < Cols; c++)
            {
                _screen[CursorRow][c] = TerminalCell.Empty;
            }
        }

        public void ClearToEndOfScreen()
        {
            ClearToEndOfLine();
            for (int r = CursorRow + 1; r < Rows; r++)
            {
                _screen[r] = CreateEmptyLine(Cols);
            }
        }

        public void ClearFromStartOfLine()
        {
            for (int c = 0; c <= CursorCol && c < Cols; c++)
            {
                _screen[CursorRow][c] = TerminalCell.Empty;
            }
        }

        public void ClearFromStartOfScreen()
        {
            for (int r = 0; r < CursorRow; r++)
            {
                _screen[r] = CreateEmptyLine(Cols);
            }

            ClearFromStartOfLine();
        }

        public string GetTextContent(int startRow, int startCol, int endRow, int endCol)
        {
            var sb = new StringBuilder();

            for (int r = startRow; r <= endRow && r < Rows; r++)
            {
                int cStart = (r == startRow) ? startCol : 0;
                int cEnd = (r == endRow) ? endCol : Cols - 1;
                cStart = System.Math.Max(0, cStart);
                cEnd = System.Math.Min(Cols - 1, cEnd);

                for (int c = cStart; c <= cEnd; c++)
                {
                    sb.Append(_screen[r][c].Character);
                }

                if (r < endRow)
                    sb.AppendLine();
            }

            return sb.ToString();
        }

        public void ResetAttributes()
        {
            CurrentFg = 7;
            CurrentBg = 0;
            Bold = false;
            Italic = false;
            Underline = false;
            Inverse = false;
        }

        private static TerminalCell[][] CreateScreen(int rows, int cols)
        {
            var screen = new TerminalCell[rows][];
            for (int r = 0; r < rows; r++)
            {
                screen[r] = CreateEmptyLine(cols);
            }

            return screen;
        }

        private static TerminalCell[] CreateEmptyLine(int cols)
        {
            var line = new TerminalCell[cols];
            for (int c = 0; c < cols; c++)
            {
                line[c] = TerminalCell.Empty;
            }

            return line;
        }
    }
}
