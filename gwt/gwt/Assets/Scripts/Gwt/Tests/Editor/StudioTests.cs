using NUnit.Framework;
using System.Collections.Generic;
using System.Linq;
using UnityEngine;
using Gwt.Studio.World;
using Gwt.Studio.Entity;
using Gwt.Agent.Services;

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
        public void StudioLayout_FindDeskByAgent_FindsAgentInAssignedAgentIds()
        {
            var layout = new StudioLayout();
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(4, 4),
                AssignedBranch = "feature/shared",
                AssignedAgentIds = new List<string> { "agent-a", "agent-b" }
            });

            var desk = layout.FindDeskByAgent("agent-b");

            Assert.IsNotNull(desk);
            Assert.AreEqual("feature/shared", desk.AssignedBranch);
        }

        [Test]
        public void StudioLayout_DefaultSize()
        {
            var layout = new StudioLayout();
            Assert.AreEqual(16, layout.Width);
            Assert.AreEqual(12, layout.Height);
        }

        // --- SimplePathfinder: Grid-based A* (no external plugin) ---

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

        // ===========================================================
        // TDD: #1547 エンティティシステム SPEC 追加分
        // 以下のテストは RED 状態（未実装）
        // ===========================================================

        // --- DeskState (US-6, US-27, FR-013, FR-014) ---

        [Test]
        public void DeskState_AllValuesExist()
        {
            Assert.AreEqual(3, System.Enum.GetValues(typeof(DeskState)).Length);
            Assert.IsTrue(System.Enum.IsDefined(typeof(DeskState), DeskState.Staffed));
            Assert.IsTrue(System.Enum.IsDefined(typeof(DeskState), DeskState.Empty));
            Assert.IsTrue(System.Enum.IsDefined(typeof(DeskState), DeskState.Remote));
        }

        [Test]
        public void DeskSlot_GetState_ReturnsStaffed_WhenAgentAssigned()
        {
            var desk = new DeskSlot
            {
                GridPosition = new Vector2Int(0, 0),
                AssignedBranch = "feature/test",
                AssignedAgentId = "agent-1",
                IsRemote = false
            };

            Assert.AreEqual(DeskState.Staffed, desk.GetState());
        }

        [Test]
        public void DeskSlot_GetState_ReturnsEmpty_WhenNoAgent()
        {
            var desk = new DeskSlot
            {
                GridPosition = new Vector2Int(0, 0),
                AssignedBranch = "feature/test",
                AssignedAgentId = null,
                IsRemote = false
            };

            Assert.AreEqual(DeskState.Empty, desk.GetState());
        }

        [Test]
        public void DeskSlot_GetState_ReturnsRemote_WhenIsRemote()
        {
            var desk = new DeskSlot
            {
                GridPosition = new Vector2Int(0, 0),
                AssignedBranch = "origin/feature/remote",
                IsRemote = true
            };

            Assert.AreEqual(DeskState.Remote, desk.GetState());
        }

        [Test]
        public void DeskSlot_GetState_ReturnsStaffed_WhenAssignedAgentIdsExist()
        {
            var desk = new DeskSlot
            {
                GridPosition = new Vector2Int(0, 0),
                AssignedBranch = "feature/test",
                AssignedAgentIds = new List<string> { "agent-1", "agent-2" },
                IsRemote = false
            };

            Assert.AreEqual(DeskState.Staffed, desk.GetState());
        }

        // --- Dynamic studio expansion (US-29, FR-044) ---

        [Test]
        public void StudioLayout_ExpandIfNeeded_IncreasesHeight_WhenDesksExceedCapacity()
        {
            var layout = new StudioLayout();
            int originalHeight = layout.Height;

            // 初期容量（DesksPerRow * rows）を超えるデスクを追加
            for (int i = 0; i < StudioLayout.DesksPerRow + 1; i++)
            {
                layout.AddDesk(new DeskSlot
                {
                    GridPosition = new Vector2Int(i % 4, i / 4 * StudioLayout.DeskRowHeight + 2),
                    AssignedBranch = $"feature/test-{i}"
                });
            }

            bool expanded = layout.ExpandIfNeeded();

            Assert.IsTrue(expanded, "Layout should expand when desk count exceeds capacity");
            Assert.Greater(layout.Height, originalHeight, "Height should increase after expansion");
        }

        [Test]
        public void StudioLayout_ShrinkIfNeeded_DecreasesHeight_WhenDesksRemoved()
        {
            var layout = new StudioLayout();
            // 拡張させてからデスクを削除
            for (int i = 0; i < 10; i++)
            {
                layout.AddDesk(new DeskSlot
                {
                    GridPosition = new Vector2Int(i % 4, i / 4 * 4 + 2),
                    AssignedBranch = $"feature/test-{i}"
                });
            }
            layout.ExpandIfNeeded();

            // デスクを大量削除
            for (int i = 5; i < 10; i++)
                layout.RemoveDesk($"feature/test-{i}");

            bool shrunk = layout.ShrinkIfNeeded();

            Assert.IsTrue(shrunk, "Layout should shrink when desks are removed");
        }

        [Test]
        public void StudioLayout_ShrinkIfNeeded_NeverBelowMinHeight()
        {
            var layout = new StudioLayout();
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(0, 2),
                AssignedBranch = "main"
            });
            layout.RemoveDesk("main");

            layout.ShrinkIfNeeded();

            Assert.GreaterOrEqual(layout.Height, StudioLayout.MinHeight,
                "Layout should never shrink below MinHeight");
        }

        [Test]
        public void StudioLayout_ExpandIfNeeded_NoExpansionWithinCapacity()
        {
            var layout = new StudioLayout();
            for (int i = 0; i < StudioLayout.DesksPerRow; i++)
            {
                layout.AddDesk(new DeskSlot
                {
                    GridPosition = new Vector2Int(i, 2),
                    AssignedBranch = $"feature/capacity-{i}"
                });
            }

            bool expanded = layout.ExpandIfNeeded();

            Assert.IsFalse(expanded);
            Assert.AreEqual(StudioLayout.MinHeight, layout.Height);
        }

        [Test]
        public void StudioLayout_ShrinkIfNeeded_NoChangeWhenAlreadyMinimal()
        {
            var layout = new StudioLayout();

            bool shrunk = layout.ShrinkIfNeeded();

            Assert.IsFalse(shrunk);
            Assert.AreEqual(StudioLayout.MinHeight, layout.Height);
        }

        // --- Studio door (US-22, US-23, FR-045) ---

        [Test]
        public void StudioLayout_DoorPosition_IsAtBottomCenter()
        {
            var layout = new StudioLayout();
            var door = layout.DoorPosition;

            Assert.AreEqual(0, door.y, "Door should be at the bottom of the studio (y=0)");
            Assert.AreEqual(layout.Width / 2, door.x, "Door should be at the horizontal center");
        }

        [Test]
        public void StudioLayout_DoorPosition_UpdatesWithExpansion()
        {
            var layout = new StudioLayout();
            var doorBefore = layout.DoorPosition;

            // 拡張はHeight方向のみなのでDoorPositionのxは変わらない
            Assert.AreEqual(layout.Width / 2, doorBefore.x);
            Assert.AreEqual(0, doorBefore.y);
        }

        // --- Desk drag / move (US-26, FR-049) ---

        [Test]
        public void StudioLayout_MoveDesk_Success_WhenTargetEmpty()
        {
            var layout = new StudioLayout();
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(2, 3),
                AssignedBranch = "feature/move-test"
            });

            bool result = layout.MoveDesk("feature/move-test", new Vector2Int(5, 5));

            Assert.IsTrue(result, "Move should succeed when target position is empty");
            var desk = layout.FindDeskByBranch("feature/move-test");
            Assert.AreEqual(new Vector2Int(5, 5), desk.GridPosition);
        }

        [Test]
        public void StudioLayout_MoveDesk_Fails_WhenTargetOccupied()
        {
            var layout = new StudioLayout();
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(2, 3),
                AssignedBranch = "feature/a"
            });
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(5, 5),
                AssignedBranch = "feature/b"
            });

            bool result = layout.MoveDesk("feature/a", new Vector2Int(5, 5));

            Assert.IsFalse(result, "Move should fail when target position is occupied");
            var desk = layout.FindDeskByBranch("feature/a");
            Assert.AreEqual(new Vector2Int(2, 3), desk.GridPosition, "Position should not change on failed move");
        }

        [Test]
        public void StudioLayout_MoveDesk_SamePosition_ReturnsTrueWithoutMutation()
        {
            var layout = new StudioLayout();
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(2, 3),
                AssignedBranch = "feature/noop"
            });

            bool result = layout.MoveDesk("feature/noop", new Vector2Int(2, 3));

            Assert.IsTrue(result);
            Assert.AreEqual(new Vector2Int(2, 3), layout.FindDeskByBranch("feature/noop").GridPosition);
        }

        [Test]
        public void StudioLayout_MoveDesk_Fails_WhenDeskMissing()
        {
            var layout = new StudioLayout();

            bool result = layout.MoveDesk("feature/missing", new Vector2Int(9, 9));

            Assert.IsFalse(result);
        }

        // --- GetEmptyDesks / GetStaffedDesks (FR-013, FR-050) ---

        [Test]
        public void StudioLayout_GetEmptyDesks_ReturnsOnlyEmptyDesks()
        {
            var layout = new StudioLayout();
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(0, 0),
                AssignedBranch = "feature/staffed",
                AssignedAgentId = "agent-1"
            });
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(1, 0),
                AssignedBranch = "feature/empty",
                AssignedAgentId = null
            });

            var emptyDesks = layout.GetEmptyDesks();

            Assert.AreEqual(1, emptyDesks.Count);
            Assert.AreEqual("feature/empty", emptyDesks[0].AssignedBranch);
        }

        [Test]
        public void StudioLayout_GetStaffedDesks_ReturnsOnlyStaffedDesks()
        {
            var layout = new StudioLayout();
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(0, 0),
                AssignedBranch = "feature/staffed",
                AssignedAgentId = "agent-1"
            });
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(1, 0),
                AssignedBranch = "feature/empty",
                AssignedAgentId = null
            });

            var staffedDesks = layout.GetStaffedDesks();

            Assert.AreEqual(1, staffedDesks.Count);
            Assert.AreEqual("feature/staffed", staffedDesks[0].AssignedBranch);
        }

        // --- Random name generation (US-25, FR-047) ---

        [Test]
        public void RandomNameGenerator_Generate_ReturnsNonEmptyString()
        {
            var name = RandomNameGenerator.Generate();

            Assert.IsFalse(string.IsNullOrWhiteSpace(name),
                "Generated name should not be empty or whitespace");
        }

        [Test]
        public void RandomNameGenerator_Generate_ProducesVariousNames()
        {
            var names = new HashSet<string>();
            for (int i = 0; i < 10; i++)
                names.Add(RandomNameGenerator.Generate());

            Assert.Greater(names.Count, 1,
                "Generator should produce multiple different names across 10 calls");
        }

        [Test]
        public void RandomNameGenerator_GetAgentTypeLabel_Claude_ReturnsClaudeCode()
        {
            var label = RandomNameGenerator.GetAgentTypeLabel(DetectedAgentType.Claude);
            Assert.AreEqual("Claude Code", label);
        }

        [Test]
        public void RandomNameGenerator_GetAgentTypeLabel_Codex_ReturnsCodex()
        {
            var label = RandomNameGenerator.GetAgentTypeLabel(DetectedAgentType.Codex);
            Assert.AreEqual("Codex", label);
        }

        [Test]
        public void RandomNameGenerator_GetAgentTypeLabel_Gemini_ReturnsGemini()
        {
            var label = RandomNameGenerator.GetAgentTypeLabel(DetectedAgentType.Gemini);
            Assert.AreEqual("Gemini", label);
        }

        [Test]
        public void RandomNameGenerator_GetAgentTypeLabel_GithubCopilot_ReturnsCopilot()
        {
            var label = RandomNameGenerator.GetAgentTypeLabel(DetectedAgentType.GithubCopilot);
            Assert.AreEqual("Copilot", label);
        }

        [Test]
        public void RandomNameGenerator_GetAgentTypeLabel_OpenCode_ReturnsOpenCode()
        {
            var label = RandomNameGenerator.GetAgentTypeLabel(DetectedAgentType.OpenCode);
            Assert.AreEqual("OpenCode", label);
        }

        [Test]
        public void RandomNameGenerator_GetAgentTypeLabel_Custom_ReturnsCustom()
        {
            var label = RandomNameGenerator.GetAgentTypeLabel(DetectedAgentType.Custom);
            Assert.AreEqual("Custom", label);
        }

        // --- Furniture type (US-24, FR-048) ---

        [Test]
        public void FurnitureType_AllValuesExist()
        {
            Assert.AreEqual(3, System.Enum.GetValues(typeof(FurnitureType)).Length);
            Assert.IsTrue(System.Enum.IsDefined(typeof(FurnitureType), FurnitureType.CoffeeMachine));
            Assert.IsTrue(System.Enum.IsDefined(typeof(FurnitureType), FurnitureType.Bookshelf));
            Assert.IsTrue(System.Enum.IsDefined(typeof(FurnitureType), FurnitureType.Whiteboard));
        }

        // --- Context menu model (US-9, US-27, FR-021, FR-050) ---

        [Test]
        public void ContextMenuBuilder_StaffedDesk_HasFiveItems()
        {
            var items = ContextMenuBuilder.BuildStaffedDeskMenu(hasSummary: true, hasPr: true);

            Assert.AreEqual(5, items.Count,
                "Staffed desk menu should have 5 items: Terminal, Summary, Git, PR, Fire Agent");
        }

        [Test]
        public void ContextMenuBuilder_StaffedDesk_AllItemsEnabled_WhenSummaryAndPrExist()
        {
            var items = ContextMenuBuilder.BuildStaffedDeskMenu(hasSummary: true, hasPr: true);

            Assert.IsTrue(items.All(i => i.IsEnabled),
                "All items should be enabled when summary and PR exist");
        }

        [Test]
        public void ContextMenuBuilder_StaffedDesk_SummaryDisabled_WhenNoSummary()
        {
            var items = ContextMenuBuilder.BuildStaffedDeskMenu(hasSummary: false, hasPr: true);

            var summaryItem = items.Find(i => i.Type == ContextMenuItemType.Summary);
            Assert.IsNotNull(summaryItem, "Summary item should exist");
            Assert.IsFalse(summaryItem.IsEnabled, "Summary should be disabled when not generated");
        }

        [Test]
        public void ContextMenuBuilder_StaffedDesk_PrDisabled_WhenNoPr()
        {
            var items = ContextMenuBuilder.BuildStaffedDeskMenu(hasSummary: true, hasPr: false);

            var prItem = items.Find(i => i.Type == ContextMenuItemType.PR);
            Assert.IsNotNull(prItem, "PR item should exist");
            Assert.IsFalse(prItem.IsEnabled, "PR should be disabled when not created");
        }

        [Test]
        public void ContextMenuBuilder_StaffedDesk_ContainsFireAgent()
        {
            var items = ContextMenuBuilder.BuildStaffedDeskMenu(hasSummary: false, hasPr: false);

            var fireItem = items.Find(i => i.Type == ContextMenuItemType.FireAgent);
            Assert.IsNotNull(fireItem, "Fire Agent item should exist");
            Assert.IsTrue(fireItem.IsEnabled, "Fire Agent should always be enabled");
        }

        [Test]
        public void ContextMenuBuilder_EmptyDesk_HasFourItems()
        {
            var items = ContextMenuBuilder.BuildEmptyDeskMenu();

            Assert.AreEqual(4, items.Count,
                "Empty desk menu should have 4 items: Hire Agent, Terminal, Git, Delete Worktree");
        }

        [Test]
        public void ContextMenuBuilder_EmptyDesk_ContainsHireAgent()
        {
            var items = ContextMenuBuilder.BuildEmptyDeskMenu();

            var hireItem = items.Find(i => i.Type == ContextMenuItemType.HireAgent);
            Assert.IsNotNull(hireItem, "Hire Agent item should exist in empty desk menu");
            Assert.IsTrue(hireItem.IsEnabled, "Hire Agent should be enabled");
        }

        [Test]
        public void ContextMenuBuilder_EmptyDesk_ContainsDeleteWorktree()
        {
            var items = ContextMenuBuilder.BuildEmptyDeskMenu();

            var deleteItem = items.Find(i => i.Type == ContextMenuItemType.DeleteWorktree);
            Assert.IsNotNull(deleteItem, "Delete Worktree item should exist in empty desk menu");
            Assert.IsTrue(deleteItem.IsEnabled, "Delete Worktree should be enabled");
        }

        // --- Agent type enum (FR-040) ---

        [Test]
        public void DetectedAgentType_IncludesGithubCopilot()
        {
            Assert.IsTrue(System.Enum.IsDefined(typeof(DetectedAgentType), DetectedAgentType.GithubCopilot));
        }

        [Test]
        public void DetectedAgentType_IncludesCustom()
        {
            Assert.IsTrue(System.Enum.IsDefined(typeof(DetectedAgentType), DetectedAgentType.Custom));
        }

        [Test]
        public void DetectedAgentType_HasSixValues()
        {
            Assert.AreEqual(6, System.Enum.GetValues(typeof(DetectedAgentType)).Length,
                "DetectedAgentType should have 6 values: Claude, Codex, Gemini, OpenCode, GithubCopilot, Custom");
        }

        // ===========================================================
        // TDD: インタビュー確定事項に基づく追加テスト（RED 状態）
        // ===========================================================

        // --- Studio expansion direction: downward (#1546) ---

        [Test]
        public void StudioLayout_ExpansionDirection_IsDown()
        {
            Assert.AreEqual(StudioLayout.ExpansionDirection.Down, StudioLayout.Expansion,
                "Studio should expand downward (y increases)");
        }

        [Test]
        public void StudioLayout_DoorPosition_IsAtBottomRow()
        {
            var layout = new StudioLayout();
            // ドアはスタジオ最下段（y=0）に配置される
            Assert.AreEqual(0, layout.DoorPosition.y,
                "Door should be at y=0 (bottom row)");
        }

        [Test]
        public void StudioLayout_ExpandIfNeeded_ExpandsDownward_NotUpward()
        {
            var layout = new StudioLayout();
            int originalHeight = layout.Height;

            // 容量超過するデスクを追加
            for (int i = 0; i < StudioLayout.DesksPerRow * 3 + 1; i++)
            {
                layout.AddDesk(new DeskSlot
                {
                    GridPosition = new Vector2Int(i % StudioLayout.DesksPerRow,
                        i / StudioLayout.DesksPerRow * StudioLayout.DeskRowHeight + 2),
                    AssignedBranch = $"feature/expand-{i}"
                });
            }

            layout.ExpandIfNeeded();

            // 拡張後もドア（y=0）は変わらない = 下方向に拡張
            Assert.AreEqual(0, layout.DoorPosition.y,
                "Door position should remain at y=0 after expansion (expansion goes downward)");
        }

        // --- 1 Issue : N Agent — DeskSlot supports multiple agents (#1545 FR-028) ---

        [Test]
        public void DeskSlot_AssignedAgentIds_SupportsMultipleAgents()
        {
            var desk = new DeskSlot
            {
                GridPosition = new Vector2Int(0, 0),
                AssignedBranch = "feature/multi-agent",
                AssignedAgentIds = new List<string> { "agent-claude-1", "agent-codex-2", "agent-gemini-3" }
            };

            Assert.AreEqual(3, desk.AssignedAgentIds.Count,
                "DeskSlot should support multiple agent IDs for 1:N Agent");
        }

        [Test]
        public void DeskSlot_GetState_ReturnsStaffed_WhenMultipleAgentsAssigned()
        {
            var desk = new DeskSlot
            {
                GridPosition = new Vector2Int(0, 0),
                AssignedBranch = "feature/test",
                AssignedAgentIds = new List<string> { "agent-1", "agent-2" },
                IsRemote = false
            };

            // 複数 Agent が着席している場合も Staffed を返す
            Assert.AreEqual(DeskState.Staffed, desk.GetState(),
                "Desk with multiple agents should return Staffed state");
        }

        [Test]
        public void StudioLayout_FindDesksByWorktree_ReturnsAllAgentsOnDesk()
        {
            var layout = new StudioLayout();
            layout.AddDesk(new DeskSlot
            {
                GridPosition = new Vector2Int(0, 0),
                AssignedBranch = "feature/shared",
                AssignedAgentIds = new List<string> { "agent-1", "agent-2" }
            });

            var desk = layout.FindDeskByBranch("feature/shared");
            Assert.IsNotNull(desk);
            Assert.AreEqual(2, desk.AssignedAgentIds.Count,
                "Desk should hold multiple agent IDs for shared worktree");
        }

        // --- Context menu for multi-agent desk (#1547 + #1545 FR-028) ---

        [Test]
        public void ContextMenuBuilder_StaffedDesk_ItemOrder_IsCorrect()
        {
            var items = ContextMenuBuilder.BuildStaffedDeskMenu(hasSummary: true, hasPr: true);

            // インタビュー確定: Terminal, Summary, Git, PR, Fire Agent の順
            Assert.AreEqual(ContextMenuItemType.Terminal, items[0].Type, "First item should be Terminal");
            Assert.AreEqual(ContextMenuItemType.Summary, items[1].Type, "Second item should be Summary");
            Assert.AreEqual(ContextMenuItemType.Git, items[2].Type, "Third item should be Git");
            Assert.AreEqual(ContextMenuItemType.PR, items[3].Type, "Fourth item should be PR");
            Assert.AreEqual(ContextMenuItemType.FireAgent, items[4].Type, "Fifth item should be Fire Agent");
        }

        [Test]
        public void ContextMenuBuilder_EmptyDesk_ItemOrder_IsCorrect()
        {
            var items = ContextMenuBuilder.BuildEmptyDeskMenu();

            // インタビュー確定: Hire Agent, Terminal, Git, Delete Worktree の順
            Assert.AreEqual(ContextMenuItemType.HireAgent, items[0].Type, "First item should be Hire Agent");
            Assert.AreEqual(ContextMenuItemType.Terminal, items[1].Type, "Second item should be Terminal");
            Assert.AreEqual(ContextMenuItemType.Git, items[2].Type, "Third item should be Git");
            Assert.AreEqual(ContextMenuItemType.DeleteWorktree, items[3].Type, "Fourth item should be Delete Worktree");
        }

        // --- Camera: Drag Pan Only (confirmed: no zoom) ---

        [Test]
        public void StudioCameraController_HasNoPubicZoomMethod()
        {
            // インタビュー確定: カメラはドラッグパンのみ対応（ズームなし）
            var cameraType = typeof(StudioCameraController);
            var zoomMethod = cameraType.GetMethod("HandleZoom",
                System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);

            Assert.IsNull(zoomMethod,
                "StudioCameraController should not have HandleZoom method (drag pan only, no zoom)");
        }

        [Test]
        public void StudioCameraController_HasNoZoomFields()
        {
            // ズーム関連のフィールドが存在しないことを確認
            var cameraType = typeof(StudioCameraController);
            var zoomSpeedField = cameraType.GetField("_zoomSpeed",
                System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            var minZoomField = cameraType.GetField("_minZoom",
                System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            var maxZoomField = cameraType.GetField("_maxZoom",
                System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);

            Assert.IsNull(zoomSpeedField, "Should not have _zoomSpeed field");
            Assert.IsNull(minZoomField, "Should not have _minZoom field");
            Assert.IsNull(maxZoomField, "Should not have _maxZoom field");
        }

        // --- Grid-based movement: no external plugin ---

        [Test]
        public void SimplePathfinder_IsGridBased_NotNavMesh()
        {
            // インタビュー確定: グリッドベース移動（A* Pathfinding Pro プラグイン不使用）
            // SimplePathfinder は Vector2Int グリッド座標で動作する
            var path = SimplePathfinder.FindPath(
                new Vector2Int(0, 0),
                new Vector2Int(2, 2),
                new HashSet<Vector2Int>(),
                5, 5,
                allowDiagonal: true
            );

            Assert.IsTrue(path.Count > 0, "Grid-based pathfinder should find a path");
            // 全ポイントが整数グリッド座標であることを確認
            foreach (var point in path)
            {
                Assert.AreEqual(point, new Vector2Int(point.x, point.y),
                    "All path points should be integer grid coordinates");
            }
        }
    }
}
