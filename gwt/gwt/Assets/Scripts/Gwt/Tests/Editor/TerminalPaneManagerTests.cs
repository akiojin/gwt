using Gwt.Core.Models;
using Gwt.Core.Services.Terminal;
using NUnit.Framework;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class TerminalPaneManagerTests
    {
        private TerminalPaneManager _manager;

        [SetUp]
        public void SetUp()
        {
            _manager = new TerminalPaneManager();
        }

        [Test]
        public void EmptyManager_ActivePane_ReturnsNull()
        {
            Assert.That(_manager.ActivePane, Is.Null);
            Assert.That(_manager.PaneCount, Is.EqualTo(0));
            Assert.That(_manager.ActiveIndex, Is.EqualTo(-1));
        }

        [Test]
        public void AddPane_IncrementsPaneCount()
        {
            var pane = CreatePane("pane-1");

            _manager.AddPane(pane);

            Assert.That(_manager.PaneCount, Is.EqualTo(1));
        }

        [Test]
        public void AddPane_SetsActiveToNewPane()
        {
            var pane1 = CreatePane("pane-1");
            var pane2 = CreatePane("pane-2");

            _manager.AddPane(pane1);
            _manager.AddPane(pane2);

            Assert.That(_manager.ActiveIndex, Is.EqualTo(1));
            Assert.That(_manager.ActivePane.PaneId, Is.EqualTo("pane-2"));
        }

        [Test]
        public void RemovePane_DecrementsPaneCount()
        {
            var pane1 = CreatePane("pane-1");
            var pane2 = CreatePane("pane-2");
            _manager.AddPane(pane1);
            _manager.AddPane(pane2);

            _manager.RemovePane("pane-1");

            Assert.That(_manager.PaneCount, Is.EqualTo(1));
        }

        [Test]
        public void RemovePane_ClampsActiveIndex()
        {
            var pane1 = CreatePane("pane-1");
            var pane2 = CreatePane("pane-2");
            _manager.AddPane(pane1);
            _manager.AddPane(pane2);

            Assert.That(_manager.ActiveIndex, Is.EqualTo(1));

            _manager.RemovePane("pane-2");

            Assert.That(_manager.ActiveIndex, Is.EqualTo(0));
            Assert.That(_manager.ActivePane.PaneId, Is.EqualTo("pane-1"));
        }

        [Test]
        public void RemovePane_LastPane_SetsActiveToNegativeOne()
        {
            var pane = CreatePane("pane-1");
            _manager.AddPane(pane);

            _manager.RemovePane("pane-1");

            Assert.That(_manager.ActiveIndex, Is.EqualTo(-1));
            Assert.That(_manager.ActivePane, Is.Null);
        }

        [Test]
        public void ActivePane_ReturnsCurrentPane()
        {
            var pane = CreatePane("pane-1");
            _manager.AddPane(pane);

            Assert.That(_manager.ActivePane, Is.SameAs(pane));
        }

        [Test]
        public void NextTab_CyclesForward()
        {
            _manager.AddPane(CreatePane("pane-1"));
            _manager.AddPane(CreatePane("pane-2"));
            _manager.AddPane(CreatePane("pane-3"));

            _manager.SetActiveIndex(0);
            Assert.That(_manager.ActiveIndex, Is.EqualTo(0));

            _manager.NextTab();
            Assert.That(_manager.ActiveIndex, Is.EqualTo(1));

            _manager.NextTab();
            Assert.That(_manager.ActiveIndex, Is.EqualTo(2));

            _manager.NextTab();
            Assert.That(_manager.ActiveIndex, Is.EqualTo(0)); // Wraps around
        }

        [Test]
        public void PrevTab_CyclesBackward()
        {
            _manager.AddPane(CreatePane("pane-1"));
            _manager.AddPane(CreatePane("pane-2"));
            _manager.AddPane(CreatePane("pane-3"));

            _manager.SetActiveIndex(0);

            _manager.PrevTab();
            Assert.That(_manager.ActiveIndex, Is.EqualTo(2)); // Wraps around

            _manager.PrevTab();
            Assert.That(_manager.ActiveIndex, Is.EqualTo(1));
        }

        [Test]
        public void GetPaneByAgentSessionId_Found()
        {
            var pane = CreatePane("pane-1");
            pane.AgentSessionId = "agent-abc";
            _manager.AddPane(pane);

            var found = _manager.GetPaneByAgentSessionId("agent-abc");

            Assert.That(found, Is.SameAs(pane));
        }

        [Test]
        public void GetPaneByAgentSessionId_NotFound_ReturnsNull()
        {
            _manager.AddPane(CreatePane("pane-1"));

            var found = _manager.GetPaneByAgentSessionId("nonexistent");

            Assert.That(found, Is.Null);
        }

        [Test]
        public void SetActiveIndex_InvalidIndex_NoChange()
        {
            _manager.AddPane(CreatePane("pane-1"));

            _manager.SetActiveIndex(5);
            Assert.That(_manager.ActiveIndex, Is.EqualTo(0));

            _manager.SetActiveIndex(-1);
            Assert.That(_manager.ActiveIndex, Is.EqualTo(0));
        }

        [Test]
        public void OnPaneAdded_EventFires()
        {
            TerminalPaneState addedPane = null;
            _manager.OnPaneAdded += p => addedPane = p;

            var pane = CreatePane("pane-1");
            _manager.AddPane(pane);

            Assert.That(addedPane, Is.SameAs(pane));
        }

        [Test]
        public void OnPaneRemoved_EventFires()
        {
            string removedId = null;
            _manager.OnPaneRemoved += id => removedId = id;

            _manager.AddPane(CreatePane("pane-1"));
            _manager.RemovePane("pane-1");

            Assert.That(removedId, Is.EqualTo("pane-1"));
        }

        [Test]
        public void OnActiveIndexChanged_EventFires()
        {
            int? changedIndex = null;
            _manager.OnActiveIndexChanged += i => changedIndex = i;

            _manager.AddPane(CreatePane("pane-1"));

            Assert.That(changedIndex, Is.EqualTo(0));
        }

        private static TerminalPaneState CreatePane(string paneId)
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            return new TerminalPaneState(paneId, adapter);
        }
    }
}
