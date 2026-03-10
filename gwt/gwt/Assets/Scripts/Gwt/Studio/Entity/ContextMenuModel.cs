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
            // TODO: 実装（TDD RED 状態）
            return new List<ContextMenuItem>();
        }

        /// <summary>
        /// 空席デスクのコンテキストメニュー項目を取得する。
        /// Hire Agent / Terminal / Git / Delete Worktree
        /// </summary>
        public static List<ContextMenuItem> BuildEmptyDeskMenu()
        {
            // TODO: 実装（TDD RED 状態）
            return new List<ContextMenuItem>();
        }
    }
}
