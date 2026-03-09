using System.Collections.Generic;
using UnityEngine;

namespace Gwt.Studio.World
{
    [System.Serializable]
    public class DeskSlot
    {
        public Vector2Int GridPosition;
        public string AssignedBranch;
        public string AssignedAgentId;
        public bool IsRemote;
    }

    [System.Serializable]
    public class StudioLayout
    {
        public int Width = 16;
        public int Height = 12;
        public List<DeskSlot> Desks = new();

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
    }
}
