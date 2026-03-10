using System.Collections.Generic;
using System.Linq;
using UnityEngine;

namespace Gwt.Studio.World
{
    /// <summary>
    /// デスクの状態（スタッフ着席/空席/リモートブランチ）
    /// </summary>
    public enum DeskState
    {
        /// <summary>Agent 起動中、スタッフ着席</summary>
        Staffed,
        /// <summary>Worktree あり、Agent 未起動（空席）</summary>
        Empty,
        /// <summary>リモートブランチのみ（半透明デスク）</summary>
        Remote
    }

    [System.Serializable]
    public class DeskSlot
    {
        public Vector2Int GridPosition;
        public string AssignedBranch;
        public string AssignedAgentId;
        /// <summary>1 Issue : N Agent 対応。複数 Agent を同一デスク（worktree）に割り当て可能。</summary>
        public List<string> AssignedAgentIds = new();
        public bool IsRemote;

        /// <summary>
        /// デスクの状態を取得する。
        /// Remote > Staffed > Empty の優先順で判定。
        /// </summary>
        public DeskState GetState()
        {
            if (IsRemote)
                return DeskState.Remote;

            if (!string.IsNullOrEmpty(AssignedAgentId) || AssignedAgentIds.Count > 0)
                return DeskState.Staffed;

            return DeskState.Empty;
        }
    }

    [System.Serializable]
    public class StudioLayout
    {
        public const int MinWidth = 16;
        public const int MinHeight = 12;
        public const int DeskRowHeight = 4;
        public const int DesksPerRow = 4;

        /// <summary>スタジオの拡張方向。下方向（y増加方向）に行を追加する。</summary>
        public enum ExpansionDirection { Down }
        public static readonly ExpansionDirection Expansion = ExpansionDirection.Down;

        public int Width = MinWidth;
        public int Height = MinHeight;
        public List<DeskSlot> Desks = new();

        /// <summary>スタジオ入口（ドア）の位置</summary>
        public Vector2Int DoorPosition => new(Width / 2, 0);

        public DeskSlot FindDeskByBranch(string branch)
        {
            return Desks.Find(d => d.AssignedBranch == branch);
        }

        public DeskSlot FindDeskByAgent(string agentId)
        {
            return Desks.Find(d => d.AssignedAgentId == agentId || d.AssignedAgentIds.Contains(agentId));
        }

        public bool AddDesk(DeskSlot desk)
        {
            if (Desks.Exists(d => d.GridPosition == desk.GridPosition))
                return false;
            Desks.Add(desk);
            return true;
        }

        public bool RemoveDesk(string branch)
        {
            return Desks.RemoveAll(d => d.AssignedBranch == branch) > 0;
        }

        /// <summary>
        /// デスク数に基づいてスタジオを拡張する。
        /// 必要に応じて行を追加し、Height を増加させる。
        /// </summary>
        /// <returns>拡張が行われた場合 true</returns>
        public bool ExpandIfNeeded()
        {
            var requiredRows = Mathf.CeilToInt(Desks.Count / (float)DesksPerRow);
            var currentRows = Mathf.Max(1, (Height - MinHeight) / DeskRowHeight + 1);
            if (requiredRows <= currentRows)
                return false;

            Height += (requiredRows - currentRows) * DeskRowHeight;
            return true;
        }

        /// <summary>
        /// 不要な行を削除してスタジオを縮小する。
        /// MinHeight 未満には縮小しない。
        /// </summary>
        /// <returns>縮小が行われた場合 true</returns>
        public bool ShrinkIfNeeded()
        {
            var requiredRows = Mathf.Max(1, Mathf.CeilToInt(Desks.Count / (float)DesksPerRow));
            var requiredHeight = MinHeight + (requiredRows - 1) * DeskRowHeight;
            var targetHeight = Mathf.Max(MinHeight, requiredHeight);
            if (targetHeight >= Height)
                return false;

            Height = targetHeight;
            return true;
        }

        /// <summary>
        /// デスクを新しいグリッド位置に移動する。
        /// 移動先が空いている場合のみ成功。
        /// </summary>
        public bool MoveDesk(string branch, Vector2Int newPosition)
        {
            var desk = FindDeskByBranch(branch);
            if (desk == null)
                return false;

            if (desk.GridPosition == newPosition)
                return true;

            if (Desks.Any(d => d != desk && d.GridPosition == newPosition))
                return false;

            desk.GridPosition = newPosition;
            return true;
        }

        /// <summary>
        /// 空席デスク（Worktree あり + Agent 未起動）の一覧を取得する。
        /// </summary>
        public List<DeskSlot> GetEmptyDesks()
        {
            return Desks.Where(d => d.GetState() == DeskState.Empty).ToList();
        }

        /// <summary>
        /// スタッフ着席中デスクの一覧を取得する。
        /// </summary>
        public List<DeskSlot> GetStaffedDesks()
        {
            return Desks.Where(d => d.GetState() == DeskState.Staffed).ToList();
        }
    }
}
