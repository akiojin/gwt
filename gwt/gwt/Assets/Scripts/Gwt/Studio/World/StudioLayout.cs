using System.Collections.Generic;
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
        public bool IsRemote;

        /// <summary>
        /// デスクの状態を取得する。
        /// Remote > Staffed > Empty の優先順で判定。
        /// </summary>
        public DeskState GetState()
        {
            // TODO: 実装（TDD RED 状態）
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
            return Desks.Find(d => d.AssignedAgentId == agentId);
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
            // TODO: 実装（TDD RED 状態）
            return false;
        }

        /// <summary>
        /// 不要な行を削除してスタジオを縮小する。
        /// MinHeight 未満には縮小しない。
        /// </summary>
        /// <returns>縮小が行われた場合 true</returns>
        public bool ShrinkIfNeeded()
        {
            // TODO: 実装（TDD RED 状態）
            return false;
        }

        /// <summary>
        /// デスクを新しいグリッド位置に移動する。
        /// 移動先が空いている場合のみ成功。
        /// </summary>
        public bool MoveDesk(string branch, Vector2Int newPosition)
        {
            // TODO: 実装（TDD RED 状態）
            return false;
        }

        /// <summary>
        /// 空席デスク（Worktree あり + Agent 未起動）の一覧を取得する。
        /// </summary>
        public List<DeskSlot> GetEmptyDesks()
        {
            // TODO: 実装（TDD RED 状態）
            return new List<DeskSlot>();
        }

        /// <summary>
        /// スタッフ着席中デスクの一覧を取得する。
        /// </summary>
        public List<DeskSlot> GetStaffedDesks()
        {
            // TODO: 実装（TDD RED 状態）
            return new List<DeskSlot>();
        }
    }
}
