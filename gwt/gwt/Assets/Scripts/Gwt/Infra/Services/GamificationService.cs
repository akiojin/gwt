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

        public void AddExperience(int amount)
        {
            if (amount <= 0) return;

            _level.Experience += amount;
            UnlockBadge("first_experience", "First Experience", "Gained experience for the first time");

            while (_level.Experience >= _level.ExperienceToNextLevel)
            {
                _level.Experience -= _level.ExperienceToNextLevel;
                _level.Level += 1;
                _level.ExperienceToNextLevel = _level.Level * 100;
                UnlockBadge($"level_{_level.Level}", $"Level {_level.Level}", $"Reached studio level {_level.Level}");
            }
        }

        public bool CheckBadge(string badgeId)
        {
            return _badges.Exists(b => b.Id == badgeId && b.Unlocked);
        }

        private void UnlockBadge(string id, string name, string description)
        {
            var badge = _badges.Find(b => b.Id == id);
            if (badge == null)
            {
                _badges.Add(new Badge
                {
                    Id = id,
                    Name = name,
                    Description = description,
                    Unlocked = true
                });
                return;
            }

            badge.Unlocked = true;
        }
    }
}
