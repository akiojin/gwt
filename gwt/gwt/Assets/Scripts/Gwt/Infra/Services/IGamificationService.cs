using System.Collections.Generic;

namespace Gwt.Infra.Services
{
    [System.Serializable]
    public class Badge
    {
        public string Id;
        public string Name;
        public string Description;
        public bool Unlocked;
    }

    [System.Serializable]
    public class StudioLevel
    {
        public int Level;
        public int Experience;
        public int ExperienceToNextLevel;
    }

    public interface IGamificationService
    {
        StudioLevel CurrentLevel { get; }
        List<Badge> GetBadges();
        void AddExperience(int amount);
        bool CheckBadge(string badgeId);
    }
}
