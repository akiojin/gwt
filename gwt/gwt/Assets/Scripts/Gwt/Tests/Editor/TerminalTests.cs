using NUnit.Framework;
using Gwt.Core.Services.Terminal;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class TerminalBufferTests
    {
        [Test]
        public void PlainText_FillsBuffer()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("Hello");

            Assert.AreEqual('H', emu.Buffer.GetCell(0, 0).Character);
            Assert.AreEqual('e', emu.Buffer.GetCell(0, 1).Character);
            Assert.AreEqual('l', emu.Buffer.GetCell(0, 2).Character);
            Assert.AreEqual('l', emu.Buffer.GetCell(0, 3).Character);
            Assert.AreEqual('o', emu.Buffer.GetCell(0, 4).Character);
            Assert.AreEqual(0, emu.Buffer.CursorRow);
            Assert.AreEqual(5, emu.Buffer.CursorCol);
        }

        [Test]
        public void CursorMovement_CUP_MovesToPosition()
        {
            var emu = new TerminalEmulator(24, 80);
            // ESC[5;10H — move cursor to row 5, col 10 (1-based)
            emu.Write("\x1b[5;10H");

            Assert.AreEqual(4, emu.Buffer.CursorRow); // 0-based
            Assert.AreEqual(9, emu.Buffer.CursorCol); // 0-based
        }

        [Test]
        public void SGR_SetRedForeground()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[31mA");

            var cell = emu.Buffer.GetCell(0, 0);
            Assert.AreEqual('A', cell.Character);
            Assert.AreEqual(1, cell.ForegroundColor); // Red = ANSI color 1
        }

        [Test]
        public void SGR_Reset_RestoresDefaults()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[31;1m");  // Red + Bold
            emu.Write("\x1b[0m");     // Reset
            emu.Write("A");

            var cell = emu.Buffer.GetCell(0, 0);
            Assert.AreEqual('A', cell.Character);
            Assert.AreEqual(7, cell.ForegroundColor); // Default white
            Assert.AreEqual(0, cell.BackgroundColor); // Default black
            Assert.IsFalse(cell.Bold);
        }

        [Test]
        public void LineWrapping_AtColumnBoundary()
        {
            var emu = new TerminalEmulator(24, 5);
            emu.Write("ABCDE");  // Fills row 0 exactly
            emu.Write("F");      // Should wrap to row 1

            Assert.AreEqual('E', emu.Buffer.GetCell(0, 4).Character);
            Assert.AreEqual('F', emu.Buffer.GetCell(1, 0).Character);
            Assert.AreEqual(1, emu.Buffer.CursorRow);
            Assert.AreEqual(1, emu.Buffer.CursorCol);
        }

        [Test]
        public void ScrollUp_WhenWritingPastLastRow()
        {
            var emu = new TerminalEmulator(3, 5);
            emu.Write("Line1\n");
            emu.Write("Line2\n");
            emu.Write("Line3\n");
            // Line1 should have scrolled into scrollback
            // Screen should now have Line2, Line3, empty

            Assert.AreEqual(1, emu.Buffer.ScrollbackLines);
            // Line2 is now at row 0
            Assert.AreEqual('L', emu.Buffer.GetCell(0, 0).Character);
        }

        [Test]
        public void ScrollbackBuffer_AccumulatesLines()
        {
            var emu = new TerminalEmulator(2, 10);
            emu.Write("AAA\n");
            emu.Write("BBB\n");
            emu.Write("CCC\n");

            Assert.IsTrue(emu.Buffer.ScrollbackLines >= 2);
        }

        [Test]
        public void ClearScreen_ErasesAllCells()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("Hello World");
            emu.Write("\x1b[2J"); // Clear screen

            Assert.AreEqual(' ', emu.Buffer.GetCell(0, 0).Character);
            Assert.AreEqual(' ', emu.Buffer.GetCell(0, 5).Character);
        }

        [Test]
        public void ClearToEndOfLine()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("Hello World");
            emu.Write("\x1b[1;6H"); // Move to row 1, col 6 (after "Hello")
            emu.Write("\x1b[K");    // Clear to end of line

            Assert.AreEqual('H', emu.Buffer.GetCell(0, 0).Character);
            Assert.AreEqual('o', emu.Buffer.GetCell(0, 4).Character);
            Assert.AreEqual(' ', emu.Buffer.GetCell(0, 5).Character); // Cleared
            Assert.AreEqual(' ', emu.Buffer.GetCell(0, 6).Character); // Cleared
        }

        [Test]
        public void AlternateScreenBuffer_Toggle()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("Main");

            // Switch to alternate screen
            emu.Write("\x1b[?1049h");
            Assert.AreEqual(' ', emu.Buffer.GetCell(0, 0).Character); // Alternate is clear
            Assert.AreEqual(0, emu.Buffer.CursorRow);

            emu.Write("Alt");

            // Switch back to main screen
            emu.Write("\x1b[?1049l");
            Assert.AreEqual('M', emu.Buffer.GetCell(0, 0).Character); // Main restored
            Assert.AreEqual('a', emu.Buffer.GetCell(0, 1).Character);
        }

        [Test]
        public void Color256_Support()
        {
            var emu = new TerminalEmulator(24, 80);
            // ESC[38;5;196m — set foreground to color 196
            emu.Write("\x1b[38;5;196mA");

            var cell = emu.Buffer.GetCell(0, 0);
            Assert.AreEqual('A', cell.Character);
            Assert.AreEqual(196, cell.ForegroundColor);
        }

        [Test]
        public void BufferResize_PreservesContent()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("Hello");

            emu.Resize(30, 120);

            Assert.AreEqual(30, emu.Rows);
            Assert.AreEqual(120, emu.Cols);
            Assert.AreEqual('H', emu.Buffer.GetCell(0, 0).Character);
            Assert.AreEqual('o', emu.Buffer.GetCell(0, 4).Character);
        }

        [Test]
        public void BufferResize_ShrinkPreservesVisibleContent()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("AB");

            emu.Resize(10, 40);

            Assert.AreEqual(10, emu.Rows);
            Assert.AreEqual(40, emu.Cols);
            Assert.AreEqual('A', emu.Buffer.GetCell(0, 0).Character);
            Assert.AreEqual('B', emu.Buffer.GetCell(0, 1).Character);
        }

        [Test]
        public void CatppuccinTheme_ColorsAreValid()
        {
            Assert.AreEqual(16, CatppuccinTheme.Colors.Length);

            foreach (var color in CatppuccinTheme.Colors)
            {
                Assert.GreaterOrEqual(color.r, 0f);
                Assert.LessOrEqual(color.r, 1f);
                Assert.GreaterOrEqual(color.g, 0f);
                Assert.LessOrEqual(color.g, 1f);
                Assert.GreaterOrEqual(color.b, 0f);
                Assert.LessOrEqual(color.b, 1f);
                Assert.AreEqual(1f, color.a);
            }

            // Background and foreground should be valid
            Assert.AreEqual(1f, CatppuccinTheme.Background.a);
            Assert.AreEqual(1f, CatppuccinTheme.Foreground.a);
            Assert.AreEqual(1f, CatppuccinTheme.CursorColor.a);
        }

        [Test]
        public void CursorMovement_CUU_MovesUp()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[5;5H");  // Row 5, Col 5
            emu.Write("\x1b[2A");    // Move up 2

            Assert.AreEqual(2, emu.Buffer.CursorRow); // 4 - 2 = 2
        }

        [Test]
        public void CursorMovement_CUD_MovesDown()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[5;5H");
            emu.Write("\x1b[3B");    // Move down 3

            Assert.AreEqual(7, emu.Buffer.CursorRow); // 4 + 3 = 7
        }

        [Test]
        public void CursorMovement_CUF_MovesForward()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[1;1H");
            emu.Write("\x1b[10C");   // Move forward 10

            Assert.AreEqual(10, emu.Buffer.CursorCol); // 0 + 10 = 10
        }

        [Test]
        public void CursorMovement_CUB_MovesBack()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[1;20H");
            emu.Write("\x1b[5D");    // Move back 5

            Assert.AreEqual(14, emu.Buffer.CursorCol); // 19 - 5 = 14
        }

        [Test]
        public void SGR_BoldAttribute()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[1mB");

            var cell = emu.Buffer.GetCell(0, 0);
            Assert.AreEqual('B', cell.Character);
            Assert.IsTrue(cell.Bold);
        }

        [Test]
        public void SGR_ItalicAttribute()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[3mI");

            var cell = emu.Buffer.GetCell(0, 0);
            Assert.AreEqual('I', cell.Character);
            Assert.IsTrue(cell.Italic);
        }

        [Test]
        public void SGR_UnderlineAttribute()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[4mU");

            var cell = emu.Buffer.GetCell(0, 0);
            Assert.AreEqual('U', cell.Character);
            Assert.IsTrue(cell.Underline);
        }

        [Test]
        public void SGR_InverseAttribute()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[7mR");

            var cell = emu.Buffer.GetCell(0, 0);
            Assert.AreEqual('R', cell.Character);
            Assert.IsTrue(cell.Inverse);
            // Inverse swaps fg/bg during write
            Assert.AreEqual(0, cell.ForegroundColor); // Was bg (0)
            Assert.AreEqual(7, cell.BackgroundColor); // Was fg (7)
        }

        [Test]
        public void SGR_BrightColors()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[91mA"); // Bright red foreground
            emu.Write("\x1b[102mB"); // Bright green background

            var cellA = emu.Buffer.GetCell(0, 0);
            Assert.AreEqual(9, cellA.ForegroundColor); // Bright red = 8 + 1

            var cellB = emu.Buffer.GetCell(0, 1);
            Assert.AreEqual(10, cellB.BackgroundColor); // Bright green = 8 + 2
        }

        [Test]
        public void SGR_BackgroundColor()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[44mA"); // Blue background

            var cell = emu.Buffer.GetCell(0, 0);
            Assert.AreEqual(4, cell.BackgroundColor);
        }

        [Test]
        public void NewlineAndCarriageReturn()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("AB\r\nCD");

            Assert.AreEqual('A', emu.Buffer.GetCell(0, 0).Character);
            Assert.AreEqual('B', emu.Buffer.GetCell(0, 1).Character);
            Assert.AreEqual('C', emu.Buffer.GetCell(1, 0).Character);
            Assert.AreEqual('D', emu.Buffer.GetCell(1, 1).Character);
        }

        [Test]
        public void OSC_WindowTitle()
        {
            var emu = new TerminalEmulator(24, 80);
            string receivedTitle = null;
            emu.TitleChanged += t => receivedTitle = t;

            emu.Write("\x1b]0;My Terminal\a");

            Assert.AreEqual("My Terminal", receivedTitle);
        }

        [Test]
        public void GetTextContent_ExtractsText()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("Hello World");

            string text = emu.GetSelectedText(0, 0, 0, 4);
            Assert.AreEqual("Hello", text);
        }

        [Test]
        public void ScrollDown_ShiftsScreenDown()
        {
            var emu = new TerminalEmulator(3, 10);
            emu.Write("AAA\nBBB\nCCC");

            emu.Write("\x1b[T"); // Scroll down 1

            // Top line should be empty after scroll down
            Assert.AreEqual(' ', emu.Buffer.GetCell(0, 0).Character);
            // AAA should now be at row 1
            Assert.AreEqual('A', emu.Buffer.GetCell(1, 0).Character);
        }

        [Test]
        public void CursorSaveRestore()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[5;10H"); // Move to 5,10
            emu.Write("\x1b[s");     // Save
            emu.Write("\x1b[1;1H");  // Move to 1,1
            emu.Write("\x1b[u");     // Restore

            Assert.AreEqual(4, emu.Buffer.CursorRow);
            Assert.AreEqual(9, emu.Buffer.CursorCol);
        }

        [Test]
        public void Color256_BackgroundSupport()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[48;5;42mA");

            var cell = emu.Buffer.GetCell(0, 0);
            Assert.AreEqual(42, cell.BackgroundColor);
        }

        [Test]
        public void EraseLine_EntireLine()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("Hello World");
            emu.Write("\x1b[1;6H");  // Position in middle
            emu.Write("\x1b[2K");    // Erase entire line

            for (int c = 0; c < 80; c++)
            {
                Assert.AreEqual(' ', emu.Buffer.GetCell(0, c).Character);
            }
        }

        [Test]
        public void MultipleSGR_InOneSequence()
        {
            var emu = new TerminalEmulator(24, 80);
            emu.Write("\x1b[1;3;31mA"); // Bold + Italic + Red

            var cell = emu.Buffer.GetCell(0, 0);
            Assert.IsTrue(cell.Bold);
            Assert.IsTrue(cell.Italic);
            Assert.AreEqual(1, cell.ForegroundColor);
        }

        [Test]
        public void BufferChanged_EventFires()
        {
            var emu = new TerminalEmulator(24, 80);
            int fireCount = 0;
            emu.BufferChanged += () => fireCount++;

            emu.Write("A");
            emu.Write("B");

            Assert.AreEqual(2, fireCount);
        }
    }
}
