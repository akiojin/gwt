using System.Collections.Generic;
using Gwt.Studio.World;

namespace Gwt.Studio.Entity
{
    /// <summary>
    /// コンテキストメニューの項目種別。
    /// </summary>
    public enum ContextMenuItemType
    {
        Terminal,
        Summary,
        Git,
        PR,
        FireAgent,
        HireAgent,
        DeleteWorktree
    }

    /// <summary>
    /// コンテキストメニューの1項目。
    /// </summary>
    public class ContextMenuItem
    {
        public ContextMenuItemType Type;
        public bool IsEnabled;
    }

    /// <summary>
    /// デスクの状態に応じたコンテキストメニューを生成する。
    /// </summary>
    public static class ContextMenuBuilder
    {
        /// <summary>
        /// スタッフ着席デスクのコンテキストメニュー項目を取得する。
        /// Terminal / Summary / Git / PR / Fire Agent
        /// </summary>
        public static List<ContextMenuItem> BuildStaffedDeskMenu(bool hasSummary, bool hasPr)
        {
            return new List<ContextMenuItem>
            {
                new() { Type = ContextMenuItemType.Terminal, IsEnabled = true },
                new() { Type = ContextMenuItemType.Summary, IsEnabled = hasSummary },
                new() { Type = ContextMenuItemType.Git, IsEnabled = true },
                new() { Type = ContextMenuItemType.PR, IsEnabled = hasPr },
                new() { Type = ContextMenuItemType.FireAgent, IsEnabled = true }
            };
        }

        /// <summary>
        /// 空席デスクのコンテキストメニュー項目を取得する。
        /// Hire Agent / Terminal / Git / Delete Worktree
        /// </summary>
        public static List<ContextMenuItem> BuildEmptyDeskMenu()
        {
            return new List<ContextMenuItem>
            {
                new() { Type = ContextMenuItemType.HireAgent, IsEnabled = true },
                new() { Type = ContextMenuItemType.Terminal, IsEnabled = true },
                new() { Type = ContextMenuItemType.Git, IsEnabled = true },
                new() { Type = ContextMenuItemType.DeleteWorktree, IsEnabled = true }
            };
        }
    }
}
