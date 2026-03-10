using System.Text;
using UnityEngine;

namespace Gwt.Core.Services.Terminal
{
    public static class TerminalRichTextBuilder
    {
        public static string BuildRichText(XtermSharpTerminalAdapter adapter, int scrollOffset, int visibleRows)
        {
            var buffer = adapter.GetBuffer();
            var sb = new StringBuilder(visibleRows * adapter.Cols * 2);

            for (int row = 0; row < visibleRows && row < buffer.Rows; row++)
            {
                if (row > 0) sb.Append('\n');
                BuildRow(buffer, row, sb);
            }

            return sb.ToString();
        }

        internal static void BuildRow(TerminalBuffer buffer, int row, StringBuilder sb)
        {
            byte currentFg = 255;
            byte currentBg = 255;
            bool currentBold = false;
            bool currentItalic = false;
            bool currentUnderline = false;
            bool hasOpenColor = false;
            bool hasOpenBold = false;
            bool hasOpenItalic = false;
            bool hasOpenUnderline = false;

            int trailingSpaces = buffer.Cols - 1;
            while (trailingSpaces >= 0 && buffer.GetCell(row, trailingSpaces).Character == ' '
                   && buffer.GetCell(row, trailingSpaces).ForegroundColor == 7
                   && buffer.GetCell(row, trailingSpaces).BackgroundColor == 0)
            {
                trailingSpaces--;
            }

            for (int col = 0; col <= trailingSpaces; col++)
            {
                var cell = buffer.GetCell(row, col);
                byte fg = cell.ForegroundColor;
                byte bg = cell.BackgroundColor;

                if (fg != currentFg || bg != currentBg)
                {
                    if (hasOpenColor) sb.Append("</color>");
                    hasOpenColor = false;

                    if (fg != 7 || bg != 0)
                    {
                        var color = MapAnsiColor(fg);
                        sb.Append("<color=#");
                        sb.Append(ColorUtility.ToHtmlStringRGB(color));
                        sb.Append('>');
                        hasOpenColor = true;
                    }

                    currentFg = fg;
                    currentBg = bg;
                }

                if (cell.Bold != currentBold)
                {
                    if (cell.Bold) { sb.Append("<b>"); hasOpenBold = true; }
                    else if (hasOpenBold) { sb.Append("</b>"); hasOpenBold = false; }
                    currentBold = cell.Bold;
                }

                if (cell.Italic != currentItalic)
                {
                    if (cell.Italic) { sb.Append("<i>"); hasOpenItalic = true; }
                    else if (hasOpenItalic) { sb.Append("</i>"); hasOpenItalic = false; }
                    currentItalic = cell.Italic;
                }

                if (cell.Underline != currentUnderline)
                {
                    if (cell.Underline) { sb.Append("<u>"); hasOpenUnderline = true; }
                    else if (hasOpenUnderline) { sb.Append("</u>"); hasOpenUnderline = false; }
                    currentUnderline = cell.Underline;
                }

                AppendEscaped(sb, cell.Character);
            }

            if (hasOpenUnderline) sb.Append("</u>");
            if (hasOpenItalic) sb.Append("</i>");
            if (hasOpenBold) sb.Append("</b>");
            if (hasOpenColor) sb.Append("</color>");
        }

        internal static void AppendEscaped(StringBuilder sb, char c)
        {
            switch (c)
            {
                case '<': sb.Append("&lt;"); break;
                case '>': sb.Append("&gt;"); break;
                case '&': sb.Append("&amp;"); break;
                default: sb.Append(c); break;
            }
        }

        public static Color MapAnsiColor(byte colorIndex)
        {
            if (colorIndex < 16)
                return CatppuccinTheme.Colors[colorIndex];

            if (colorIndex >= 232)
            {
                float gray = (colorIndex - 232) / 23f;
                return new Color(gray, gray, gray, 1f);
            }

            int idx = colorIndex - 16;
            int r = idx / 36;
            int g = (idx % 36) / 6;
            int b = idx % 6;
            return new Color(r / 5f, g / 5f, b / 5f, 1f);
        }
    }
}
