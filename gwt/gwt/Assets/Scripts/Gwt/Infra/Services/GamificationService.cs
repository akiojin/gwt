using System.Collections.Generic;
using UnityEngine;

namespace Gwt.Infra.Services
{
    public class GamificationService : IGamificationService
    {
        private readonly StudioLevel _level = new() { Level = 1, Experience = 0, ExperienceToNextLevel = 100 };
        private readonly List<Badge> _badges = new();

        public StudioLevel CurrentLevel => _level;
        public List<Badge> GetBadges() => new(_badges);
        public void AddExperience(int amount) => Debug.Log($"[GamificationService] AddExperience: {amount} (stub)");
        public bool CheckBadge(string badgeId) => false;
    }
}
