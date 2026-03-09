using NUnit.Framework;
using System.Collections.Generic;
using UnityEngine;
using Gwt.Studio.World;
using Gwt.Studio.Entity;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class StudioTests
    {
        // --- StudioLayout desk management ---

        [Test]
        public void StudioLayout_AddDesk_Success()
        {
            var layout = new StudioLayout();
            var desk = new DeskSlot
            {
                GridPosition = new Vector2Int(2, 3),
                AssignedBranch = "feature/test",
                AssignedAgentId = "agent-1"
            };

            bool result = layout.AddDesk(desk);

            Assert.IsTrue(result);
            Assert.AreEqual(1, layout.Desks.Count);
        }

        [Test]
        public void StudioLayout_AddDesk_DuplicatePosition_Fails()
        {
            var layout = new StudioLayout();
            var desk1 = new DeskSlot
            {
                GridPosition = new Vector2Int(2, 3),
                AssignedBranch = "feature/a"
            };
            var desk2 = new DeskSlot
            {
                GridPosition = new Vector2Int(2, 3),
                AssignedBranch = "feature/b"
            };

            layout.AddDesk(desk1);
            bool result = layout.AddDesk(desk2);

            Assert.IsFalse(result);
            Assert.AreEqual(1, layout.Desks.Count);
        }

        [Test]
        public void StudioLayout_RemoveDesk_ByBranch()
        {
            var layout = new StudioLayout();
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(0, 0),
                AssignedBranch = "main"
            });
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(1, 0),
                AssignedBranch = "develop"
            });

            bool result = layout.RemoveDesk("main");

            Assert.IsTrue(result);
            Assert.AreEqual(1, layout.Desks.Count);
            Assert.AreEqual("develop", layout.Desks[0].AssignedBranch);
        }

        [Test]
        public void StudioLayout_RemoveDesk_NonExistent_ReturnsFalse()
        {
            var layout = new StudioLayout();
            bool result = layout.RemoveDesk("nonexistent");
            Assert.IsFalse(result);
        }

        [Test]
        public void StudioLayout_FindDeskByBranch_Found()
        {
            var layout = new StudioLayout();
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(5, 5),
                AssignedBranch = "feature/search"
            });

            var desk = layout.FindDeskByBranch("feature/search");

            Assert.IsNotNull(desk);
            Assert.AreEqual(new Vector2Int(5, 5), desk.GridPosition);
        }

        [Test]
        public void StudioLayout_FindDeskByBranch_NotFound_ReturnsNull()
        {
            var layout = new StudioLayout();
            var desk = layout.FindDeskByBranch("nonexistent");
            Assert.IsNull(desk);
        }

        [Test]
        public void StudioLayout_FindDeskByAgent_Found()
        {
            var layout = new StudioLayout();
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(3, 3),
                AssignedBranch = "feature/x",
                AssignedAgentId = "agent-42"
            });

            var desk = layout.FindDeskByAgent("agent-42");

            Assert.IsNotNull(desk);
            Assert.AreEqual("feature/x", desk.AssignedBranch);
        }

        [Test]
        public void StudioLayout_DefaultSize()
        {
            var layout = new StudioLayout();
            Assert.AreEqual(16, layout.Width);
            Assert.AreEqual(12, layout.Height);
        }

        // --- SimplePathfinder A* ---

        [Test]
        public void Pathfinder_DirectPath_NoObstacles()
        {
            var obstacles = new HashSet<Vector2Int>();
            var path = SimplePathfinder.FindPath(
                new Vector2Int(0, 0),
                new Vector2Int(3, 0),
                obstacles,
                10, 10
            );

            Assert.IsTrue(path.Count > 0);
            Assert.AreEqual(new Vector2Int(0, 0), path[0]);
            Assert.AreEqual(new Vector2Int(3, 0), path[path.Count - 1]);
        }

        [Test]
        public void Pathfinder_WithObstacles_FindsAlternate()
        {
            var obstacles = new HashSet<Vector2Int>
            {
                new(1, 0),
                new(1, 1)
            };
            var path = SimplePathfinder.FindPath(
                new Vector2Int(0, 0),
                new Vector2Int(2, 0),
                obstacles,
                5, 5
            );

            Assert.IsTrue(path.Count > 0);
            Assert.AreEqual(new Vector2Int(0, 0), path[0]);
            Assert.AreEqual(new Vector2Int(2, 0), path[path.Count - 1]);
            Assert.IsFalse(path.Contains(new Vector2Int(1, 0)));
        }

        [Test]
        public void Pathfinder_StartEqualsEnd_ReturnsSingleNode()
        {
            var path = SimplePathfinder.FindPath(
                new Vector2Int(3, 3),
                new Vector2Int(3, 3),
                new HashSet<Vector2Int>(),
                10, 10
            );

            Assert.AreEqual(1, path.Count);
            Assert.AreEqual(new Vector2Int(3, 3), path[0]);
        }

        [Test]
        public void Pathfinder_BlockedTarget_ReturnsEmpty()
        {
            var obstacles = new HashSet<Vector2Int> { new(2, 2) };
            var path = SimplePathfinder.FindPath(
                new Vector2Int(0, 0),
                new Vector2Int(2, 2),
                obstacles,
                5, 5
            );

            Assert.AreEqual(0, path.Count);
        }

        [Test]
        public void Pathfinder_Diagonal_AllowedWhenEnabled()
        {
            var path = SimplePathfinder.FindPath(
                new Vector2Int(0, 0),
                new Vector2Int(2, 2),
                new HashSet<Vector2Int>(),
                5, 5,
                allowDiagonal: true
            );

            Assert.IsTrue(path.Count > 0);
            Assert.AreEqual(new Vector2Int(2, 2), path[path.Count - 1]);
            // Diagonal path should be shorter than cardinal-only
            var cardinalPath = SimplePathfinder.FindPath(
                new Vector2Int(0, 0),
                new Vector2Int(2, 2),
                new HashSet<Vector2Int>(),
                5, 5,
                allowDiagonal: false
            );
            Assert.IsTrue(path.Count <= cardinalPath.Count);
        }

        // --- CharacterState transitions ---

        [Test]
        public void CharacterState_AllValuesExist()
        {
            Assert.AreEqual(7, System.Enum.GetValues(typeof(CharacterState)).Length);
            Assert.IsTrue(System.Enum.IsDefined(typeof(CharacterState), CharacterState.Idle));
            Assert.IsTrue(System.Enum.IsDefined(typeof(CharacterState), CharacterState.Walking));
            Assert.IsTrue(System.Enum.IsDefined(typeof(CharacterState), CharacterState.Working));
            Assert.IsTrue(System.Enum.IsDefined(typeof(CharacterState), CharacterState.WaitingInput));
            Assert.IsTrue(System.Enum.IsDefined(typeof(CharacterState), CharacterState.Stopped));
            Assert.IsTrue(System.Enum.IsDefined(typeof(CharacterState), CharacterState.Entering));
            Assert.IsTrue(System.Enum.IsDefined(typeof(CharacterState), CharacterState.Leaving));
        }

        [Test]
        public void FacingDirection_AllValuesExist()
        {
            Assert.AreEqual(4, System.Enum.GetValues(typeof(FacingDirection)).Length);
            Assert.IsTrue(System.Enum.IsDefined(typeof(FacingDirection), FacingDirection.Down));
            Assert.IsTrue(System.Enum.IsDefined(typeof(FacingDirection), FacingDirection.Up));
            Assert.IsTrue(System.Enum.IsDefined(typeof(FacingDirection), FacingDirection.Left));
            Assert.IsTrue(System.Enum.IsDefined(typeof(FacingDirection), FacingDirection.Right));
        }

        // --- AtmosphereState enum ---

        [Test]
        public void AtmosphereState_AllValuesExist()
        {
            Assert.AreEqual(3, System.Enum.GetValues(typeof(AtmosphereState)).Length);
            Assert.IsTrue(System.Enum.IsDefined(typeof(AtmosphereState), AtmosphereState.Normal));
            Assert.IsTrue(System.Enum.IsDefined(typeof(AtmosphereState), AtmosphereState.CISuccess));
            Assert.IsTrue(System.Enum.IsDefined(typeof(AtmosphereState), AtmosphereState.CIFail));
        }
    }
}
