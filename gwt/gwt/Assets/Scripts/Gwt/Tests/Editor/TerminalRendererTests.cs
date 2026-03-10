using System.Text;
using Gwt.Core.Services.Terminal;
using NUnit.Framework;
using UnityEngine;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class TerminalRendererTests
    {
        [Test]
        public void BuildRichText_PlainText_NoTags()
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            adapter.Feed("Hello");

            var result = TerminalRichTextBuilder.BuildRichText(adapter, 0, 24);

            Assert.That(result, Does.Contain("Hello"));
            Assert.That(result, Does.Not.Contain("<color"));
        }

        [Test]
        public void BuildRichText_ColoredText_WrapsInColorTag()
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            adapter.Feed("\x1b[31mRed");

            var result = TerminalRichTextBuilder.BuildRichText(adapter, 0, 24);

            Assert.That(result, Does.Contain("<color=#"));
            Assert.That(result, Does.Contain("Red"));
            Assert.That(result, Does.Contain("</color>"));
        }

        [Test]
        public void BuildRichText_Bold()
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            adapter.Feed("\x1b[1mBold");

            var result = TerminalRichTextBuilder.BuildRichText(adapter, 0, 24);

            Assert.That(result, Does.Contain("<b>"));
            Assert.That(result, Does.Contain("Bold"));
            Assert.That(result, Does.Contain("</b>"));
        }

        [Test]
        public void BuildRichText_Italic()
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            adapter.Feed("\x1b[3mItalic");

            var result = TerminalRichTextBuilder.BuildRichText(adapter, 0, 24);

            Assert.That(result, Does.Contain("<i>"));
            Assert.That(result, Does.Contain("Italic"));
            Assert.That(result, Does.Contain("</i>"));
        }

        [Test]
        public void BuildRichText_Underline()
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            adapter.Feed("\x1b[4mUnder");

            var result = TerminalRichTextBuilder.BuildRichText(adapter, 0, 24);

            Assert.That(result, Does.Contain("<u>"));
            Assert.That(result, Does.Contain("Under"));
            Assert.That(result, Does.Contain("</u>"));
        }

        [Test]
        public void BuildRichText_SpecialChars_Escaped()
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            adapter.Feed("<tag>&amp");

            var result = TerminalRichTextBuilder.BuildRichText(adapter, 0, 24);

            Assert.That(result, Does.Contain("&lt;"));
            Assert.That(result, Does.Contain("&gt;"));
            Assert.That(result, Does.Contain("&amp;"));
            Assert.That(result, Does.Not.Contain("<tag>"));
        }

        [Test]
        public void MapAnsiColor_StandardColors_MatchesCatppuccin()
        {
            for (byte i = 0; i < 16; i++)
            {
                var color = TerminalRichTextBuilder.MapAnsiColor(i);
                Assert.That(color, Is.EqualTo(CatppuccinTheme.Colors[i]),
                    $"Color index {i} should match Catppuccin palette");
            }
        }

        [Test]
        public void MapAnsiColor_ExtendedRange_ReturnsValidColor()
        {
            var color100 = TerminalRichTextBuilder.MapAnsiColor(100);
            Assert.That(color100.a, Is.EqualTo(1f));

            var color232 = TerminalRichTextBuilder.MapAnsiColor(232);
            Assert.That(color232.a, Is.EqualTo(1f));
            Assert.That(color232.r, Is.LessThan(0.1f)); // Near black

            var color255 = TerminalRichTextBuilder.MapAnsiColor(255);
            Assert.That(color255.a, Is.EqualTo(1f));
            Assert.That(color255.r, Is.GreaterThan(0.9f)); // Near white
        }

        [Test]
        public void BuildRichText_ConsecutiveSameColor_MergesSpan()
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            adapter.Feed("\x1b[31mABC");

            var result = TerminalRichTextBuilder.BuildRichText(adapter, 0, 24);

            // Count color tags: should be exactly one open and one close
            int openCount = CountOccurrences(result, "<color=#");
            int closeCount = CountOccurrences(result, "</color>");
            Assert.That(openCount, Is.EqualTo(1), "Should merge consecutive same-color cells into one span");
            Assert.That(closeCount, Is.EqualTo(1));
        }

        [Test]
        public void AppendEscaped_NormalChar_Appended()
        {
            var sb = new StringBuilder();
            TerminalRichTextBuilder.AppendEscaped(sb, 'A');
            Assert.That(sb.ToString(), Is.EqualTo("A"));
        }

        [Test]
        public void AppendEscaped_LessThan_Escaped()
        {
            var sb = new StringBuilder();
            TerminalRichTextBuilder.AppendEscaped(sb, '<');
            Assert.That(sb.ToString(), Is.EqualTo("&lt;"));
        }

        [Test]
        public void AppendEscaped_GreaterThan_Escaped()
        {
            var sb = new StringBuilder();
            TerminalRichTextBuilder.AppendEscaped(sb, '>');
            Assert.That(sb.ToString(), Is.EqualTo("&gt;"));
        }

        [Test]
        public void AppendEscaped_Ampersand_Escaped()
        {
            var sb = new StringBuilder();
            TerminalRichTextBuilder.AppendEscaped(sb, '&');
            Assert.That(sb.ToString(), Is.EqualTo("&amp;"));
        }

        [Test]
        public void XtermSharpTerminalAdapter_Feed_WritesToBuffer()
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            adapter.Feed("Test");

            var buf = adapter.GetBuffer();
            Assert.That(buf.GetCell(0, 0).Character, Is.EqualTo('T'));
            Assert.That(buf.GetCell(0, 3).Character, Is.EqualTo('t'));
        }

        [Test]
        public void XtermSharpTerminalAdapter_Resize_UpdatesDimensions()
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            adapter.Resize(40, 120);

            Assert.That(adapter.Rows, Is.EqualTo(40));
            Assert.That(adapter.Cols, Is.EqualTo(120));
        }

        [Test]
        public void XtermSharpTerminalAdapter_BufferChanged_EventFires()
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            bool fired = false;
            adapter.BufferChanged += () => fired = true;

            adapter.Feed("A");

            Assert.That(fired, Is.True);
        }

        private static int CountOccurrences(string source, string pattern)
        {
            int count = 0;
            int i = 0;
            while ((i = source.IndexOf(pattern, i, System.StringComparison.Ordinal)) >= 0)
            {
                count++;
                i += pattern.Length;
            }
            return count;
        }
    }
}
