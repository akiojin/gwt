using System;
using System.Collections.Generic;

namespace Gwt.Agent.Lead
{
    [Serializable]
    public class ProjectContext
    {
        public string ProjectRoot;
        public string DefaultBranch;
        public string CurrentBranch;
        public List<string> AvailableAgents = new();
        public List<string> ExistingBranches = new();
    }
}
