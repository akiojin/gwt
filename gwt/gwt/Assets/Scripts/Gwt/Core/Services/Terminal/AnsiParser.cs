using System;
using System.Collections.Generic;
using System.Text;

namespace Gwt.Core.Services.Terminal
{
    public class AnsiParser
    {
        private readonly TerminalBuffer _buffer;
        private ParserState _state = ParserState.Ground;
        private readonly StringBuilder _paramBuffer = new();
        private readonly StringBuilder _oscBuffer = new();

        private bool _alternateScreen;
        private TerminalCell[][] _savedMainScreen;
        private int _savedMainRows;
        private int _savedMainCols;

        private int _savedCursorRow;
        private int _savedCursorCol;

        public event Action<string> TitleChanged;
        public event Action<string> UrlDetected;

        public AnsiParser(TerminalBuffer buffer)
        {
            _buffer = buffer;
        }

        public void Process(string data)
        {
            foreach (char c in data)
                ProcessChar(c);
        }

        private void ProcessChar(char c)
        {
            switch (_state)
            {
                case ParserState.Ground:
                    ProcessGround(c);
                    break;
                case ParserState.Escape:
                    ProcessEscape(c);
                    break;
                case ParserState.CsiEntry:
                    ProcessCsiEntry(c);
                    break;
                case ParserState.CsiParam:
                    ProcessCsiParam(c);
                    break;
                case ParserState.OscString:
                    ProcessOsc(c);
                    break;
            }
        }

        private void ProcessGround(char c)
        {
            switch (c)
            {
                case '\x1b': // ESC
                    _state = ParserState.Escape;
                    break;
                case '\n':
                    _buffer.CarriageReturn();
                    _buffer.Linefeed();
                    break;
                case '\r':
                    _buffer.CarriageReturn();
                    break;
                case '\t':
                    // Tab: advance to next 8-column boundary
                    int nextTab = (((_buffer.CursorCol / 8) + 1) * 8);
                    _buffer.CursorCol = Math.Min(nextTab, _buffer.Cols - 1);
                    break;
                case '\b':
                    if (_buffer.CursorCol > 0)
                        _buffer.CursorCol--;
                    break;
                case '\a': // Bell — ignore
                    break;
                default:
                    if (c >= ' ') // Printable
                        _buffer.WriteChar(c);
                    break;
            }
        }

        private void ProcessEscape(char c)
        {
            switch (c)
            {
                case '[':
                    _state = ParserState.CsiEntry;
                    _paramBuffer.Clear();
                    break;
                case ']':
                    _state = ParserState.OscString;
                    _oscBuffer.Clear();
                    break;
                case '7': // Save cursor (DECSC)
                    _savedCursorRow = _buffer.CursorRow;
                    _savedCursorCol = _buffer.CursorCol;
                    _state = ParserState.Ground;
                    break;
                case '8': // Restore cursor (DECRC)
                    _buffer.CursorRow = _savedCursorRow;
                    _buffer.CursorCol = _savedCursorCol;
                    _state = ParserState.Ground;
                    break;
                case 'D': // Index — move cursor down, scroll if at bottom
                    _buffer.Linefeed();
                    _state = ParserState.Ground;
                    break;
                case 'M': // Reverse index — move cursor up, scroll down if at top
                    if (_buffer.CursorRow == 0)
                        _buffer.ScrollDown();
                    else
                        _buffer.CursorRow--;
                    _state = ParserState.Ground;
                    break;
                case 'c': // Full reset (RIS)
                    _buffer.ClearScreen();
                    _buffer.CursorRow = 0;
                    _buffer.CursorCol = 0;
                    _buffer.ResetAttributes();
                    _state = ParserState.Ground;
                    break;
                default:
                    // Unknown escape — return to ground
                    _state = ParserState.Ground;
                    break;
            }
        }

        private void ProcessCsiEntry(char c)
        {
            if (c == '?')
            {
                _paramBuffer.Append(c);
                _state = ParserState.CsiParam;
            }
            else if (char.IsDigit(c) || c == ';')
            {
                _paramBuffer.Append(c);
                _state = ParserState.CsiParam;
            }
            else
            {
                // Immediate final character with no params
                HandleCsi(c);
                _state = ParserState.Ground;
            }
        }

        private void ProcessCsiParam(char c)
        {
            if (char.IsDigit(c) || c == ';')
            {
                _paramBuffer.Append(c);
            }
            else
            {
                HandleCsi(c);
                _state = ParserState.Ground;
            }
        }

        private void ProcessOsc(char c)
        {
            if (c == '\a') // BEL terminates OSC
            {
                HandleOsc(_oscBuffer.ToString());
                _state = ParserState.Ground;
            }
            else if (c == '\x1b')
            {
                // ESC \ (ST) — we'll just terminate on ESC for simplicity
                HandleOsc(_oscBuffer.ToString());
                _state = ParserState.Ground;
            }
            else
            {
                _oscBuffer.Append(c);
            }
        }

        private void HandleCsi(char final)
        {
            string paramStr = _paramBuffer.ToString();
            bool privateMode = paramStr.StartsWith("?");
            if (privateMode)
                paramStr = paramStr.Substring(1);

            int[] parameters = ParseParameters(paramStr);

            if (privateMode)
            {
                HandlePrivateMode(final, parameters);
                return;
            }

            switch (final)
            {
                case 'A': // CUU — Cursor Up
                    _buffer.CursorRow = Math.Max(0, _buffer.CursorRow - Math.Max(1, Param(parameters, 0, 1)));
                    break;
                case 'B': // CUD — Cursor Down
                    _buffer.CursorRow = Math.Min(_buffer.Rows - 1,
                        _buffer.CursorRow + Math.Max(1, Param(parameters, 0, 1)));
                    break;
                case 'C': // CUF — Cursor Forward
                    _buffer.CursorCol = Math.Min(_buffer.Cols - 1,
                        _buffer.CursorCol + Math.Max(1, Param(parameters, 0, 1)));
                    break;
                case 'D': // CUB — Cursor Back
                    _buffer.CursorCol = Math.Max(0, _buffer.CursorCol - Math.Max(1, Param(parameters, 0, 1)));
                    break;
                case 'H': // CUP — Cursor Position
                case 'f':
                    _buffer.CursorRow = Math.Min(_buffer.Rows - 1, Math.Max(0, Param(parameters, 0, 1) - 1));
                    _buffer.CursorCol = Math.Min(_buffer.Cols - 1, Math.Max(0, Param(parameters, 1, 1) - 1));
                    break;
                case 'J': // ED — Erase in Display
                    HandleEraseDisplay(Param(parameters, 0, 0));
                    break;
                case 'K': // EL — Erase in Line
                    HandleEraseLine(Param(parameters, 0, 0));
                    break;
                case 'S': // SU — Scroll Up
                {
                    int lines = Math.Max(1, Param(parameters, 0, 1));
                    for (int i = 0; i < lines; i++)
                        _buffer.ScrollUp();
                    break;
                }
                case 'T': // SD — Scroll Down
                {
                    int lines = Math.Max(1, Param(parameters, 0, 1));
                    for (int i = 0; i < lines; i++)
                        _buffer.ScrollDown();
                    break;
                }
                case 'm': // SGR — Select Graphic Rendition
                    HandleSgr(parameters);
                    break;
                case 's': // Save cursor position
                    _savedCursorRow = _buffer.CursorRow;
                    _savedCursorCol = _buffer.CursorCol;
                    break;
                case 'u': // Restore cursor position
                    _buffer.CursorRow = _savedCursorRow;
                    _buffer.CursorCol = _savedCursorCol;
                    break;
                case 'G': // CHA — Cursor Character Absolute
                    _buffer.CursorCol = Math.Min(_buffer.Cols - 1, Math.Max(0, Param(parameters, 0, 1) - 1));
                    break;
                case 'd': // VPA — Line Position Absolute
                    _buffer.CursorRow = Math.Min(_buffer.Rows - 1, Math.Max(0, Param(parameters, 0, 1) - 1));
                    break;
                case 'X': // ECH — Erase Character
                {
                    int count = Math.Max(1, Param(parameters, 0, 1));
                    for (int i = 0; i < count && _buffer.CursorCol + i < _buffer.Cols; i++)
                    {
                        _buffer.SetCell(_buffer.CursorRow, _buffer.CursorCol + i, TerminalCell.Empty);
                    }
                    break;
                }
                case 'L': // IL — Insert Lines
                {
                    int count = Math.Max(1, Param(parameters, 0, 1));
                    for (int i = 0; i < count; i++)
                    {
                        // Shift lines down from cursor, insert blank at cursor row
                        for (int r = _buffer.Rows - 1; r > _buffer.CursorRow; r--)
                        {
                            for (int c = 0; c < _buffer.Cols; c++)
                                _buffer.SetCell(r, c, _buffer.GetCell(r - 1, c));
                        }
                        _buffer.ClearLine(_buffer.CursorRow);
                    }
                    break;
                }
                case 'M': // DL — Delete Lines
                {
                    int count = Math.Max(1, Param(parameters, 0, 1));
                    for (int i = 0; i < count; i++)
                    {
                        // Shift lines up from cursor+1, clear bottom
                        for (int r = _buffer.CursorRow; r < _buffer.Rows - 1; r++)
                        {
                            for (int c = 0; c < _buffer.Cols; c++)
                                _buffer.SetCell(r, c, _buffer.GetCell(r + 1, c));
                        }
                        _buffer.ClearLine(_buffer.Rows - 1);
                    }
                    break;
                }
            }
        }

        private void HandlePrivateMode(char final, int[] parameters)
        {
            if (parameters.Length == 0) return;

            int mode = parameters[0];
            switch (final)
            {
                case 'h': // DECSET
                    if (mode == 1049) // Alternate screen buffer
                    {
                        if (!_alternateScreen)
                        {
                            _alternateScreen = true;
                            // Save main screen
                            _savedMainRows = _buffer.Rows;
                            _savedMainCols = _buffer.Cols;
                            _savedMainScreen = new TerminalCell[_buffer.Rows][];
                            for (int r = 0; r < _buffer.Rows; r++)
                            {
                                _savedMainScreen[r] = new TerminalCell[_buffer.Cols];
                                for (int c = 0; c < _buffer.Cols; c++)
                                    _savedMainScreen[r][c] = _buffer.GetCell(r, c);
                            }
                            _savedCursorRow = _buffer.CursorRow;
                            _savedCursorCol = _buffer.CursorCol;
                            _buffer.ClearScreen();
                            _buffer.CursorRow = 0;
                            _buffer.CursorCol = 0;
                        }
                    }
                    break;
                case 'l': // DECRST
                    if (mode == 1049) // Restore main screen
                    {
                        if (_alternateScreen)
                        {
                            _alternateScreen = false;
                            if (_savedMainScreen != null)
                            {
                                int restoreRows = Math.Min(_savedMainRows, _buffer.Rows);
                                int restoreCols = Math.Min(_savedMainCols, _buffer.Cols);
                                _buffer.ClearScreen();
                                for (int r = 0; r < restoreRows; r++)
                                {
                                    for (int c = 0; c < restoreCols; c++)
                                        _buffer.SetCell(r, c, _savedMainScreen[r][c]);
                                }
                                _buffer.CursorRow = Math.Min(_savedCursorRow, _buffer.Rows - 1);
                                _buffer.CursorCol = Math.Min(_savedCursorCol, _buffer.Cols - 1);
                                _savedMainScreen = null;
                            }
                        }
                    }
                    break;
            }
        }

        private void HandleEraseDisplay(int mode)
        {
            switch (mode)
            {
                case 0: // Erase from cursor to end of screen
                    _buffer.ClearToEndOfScreen();
                    break;
                case 1: // Erase from start to cursor
                    _buffer.ClearFromStartOfScreen();
                    break;
                case 2: // Erase entire screen
                case 3: // Erase entire screen + scrollback (treat same as 2)
                    _buffer.ClearScreen();
                    break;
            }
        }

        private void HandleEraseLine(int mode)
        {
            switch (mode)
            {
                case 0: // Erase from cursor to end of line
                    _buffer.ClearToEndOfLine();
                    break;
                case 1: // Erase from start to cursor
                    _buffer.ClearFromStartOfLine();
                    break;
                case 2: // Erase entire line
                    _buffer.ClearLine(_buffer.CursorRow);
                    break;
            }
        }

        private void HandleSgr(int[] parameters)
        {
            if (parameters.Length == 0)
            {
                _buffer.ResetAttributes();
                return;
            }

            for (int i = 0; i < parameters.Length; i++)
            {
                int p = parameters[i];

                switch (p)
                {
                    case 0: // Reset
                        _buffer.ResetAttributes();
                        break;
                    case 1: // Bold
                        _buffer.Bold = true;
                        break;
                    case 3: // Italic
                        _buffer.Italic = true;
                        break;
                    case 4: // Underline
                        _buffer.Underline = true;
                        break;
                    case 7: // Inverse
                        _buffer.Inverse = true;
                        break;
                    case 22: // Normal intensity (not bold)
                        _buffer.Bold = false;
                        break;
                    case 23: // Not italic
                        _buffer.Italic = false;
                        break;
                    case 24: // Not underline
                        _buffer.Underline = false;
                        break;
                    case 27: // Not inverse
                        _buffer.Inverse = false;
                        break;
                    case >= 30 and <= 37: // Standard foreground colors
                        _buffer.CurrentFg = (byte)(p - 30);
                        break;
                    case 38: // Extended foreground color
                        i = HandleExtendedColor(parameters, i, isForeground: true);
                        break;
                    case 39: // Default foreground
                        _buffer.CurrentFg = 7;
                        break;
                    case >= 40 and <= 47: // Standard background colors
                        _buffer.CurrentBg = (byte)(p - 40);
                        break;
                    case 48: // Extended background color
                        i = HandleExtendedColor(parameters, i, isForeground: false);
                        break;
                    case 49: // Default background
                        _buffer.CurrentBg = 0;
                        break;
                    case >= 90 and <= 97: // Bright foreground colors
                        _buffer.CurrentFg = (byte)(p - 90 + 8);
                        break;
                    case >= 100 and <= 107: // Bright background colors
                        _buffer.CurrentBg = (byte)(p - 100 + 8);
                        break;
                }
            }
        }

        private int HandleExtendedColor(int[] parameters, int index, bool isForeground)
        {
            if (index + 1 >= parameters.Length)
                return index;

            int colorMode = parameters[index + 1];
            if (colorMode == 5 && index + 2 < parameters.Length)
            {
                // 256 color: ESC[38;5;Nm or ESC[48;5;Nm
                byte colorIndex = (byte)Math.Min(255, Math.Max(0, parameters[index + 2]));
                if (isForeground)
                    _buffer.CurrentFg = colorIndex;
                else
                    _buffer.CurrentBg = colorIndex;
                return index + 2;
            }

            if (colorMode == 2 && index + 4 < parameters.Length)
            {
                // 24-bit color: ESC[38;2;R;G;Bm — map to nearest 256-color
                // For simplicity, store as 0 (we don't support true color mapping yet)
                return index + 4;
            }

            return index + 1;
        }

        private void HandleOsc(string data)
        {
            int semicolonIndex = data.IndexOf(';');
            if (semicolonIndex < 0) return;

            string codeStr = data.Substring(0, semicolonIndex);
            string payload = data.Substring(semicolonIndex + 1);

            if (int.TryParse(codeStr, out int code))
            {
                switch (code)
                {
                    case 0: // Set icon name and window title
                    case 2: // Set window title
                        TitleChanged?.Invoke(payload);
                        break;
                    case 8: // Hyperlink
                        // Format: 8;params;uri
                        int uriSep = payload.IndexOf(';');
                        if (uriSep >= 0)
                        {
                            string uri = payload.Substring(uriSep + 1);
                            if (!string.IsNullOrEmpty(uri))
                                UrlDetected?.Invoke(uri);
                        }
                        break;
                }
            }
        }

        private static int[] ParseParameters(string paramStr)
        {
            if (string.IsNullOrEmpty(paramStr))
                return Array.Empty<int>();

            string[] parts = paramStr.Split(';');
            var result = new List<int>();

            foreach (string part in parts)
            {
                if (int.TryParse(part, out int val))
                    result.Add(val);
                else
                    result.Add(0);
            }

            return result.ToArray();
        }

        private static int Param(int[] parameters, int index, int defaultValue)
        {
            if (index < parameters.Length)
                return parameters[index] == 0 ? defaultValue : parameters[index];
            return defaultValue;
        }

        private enum ParserState
        {
            Ground,
            Escape,
            CsiEntry,
            CsiParam,
            OscString
        }
    }
}
