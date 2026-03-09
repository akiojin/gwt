namespace Gwt.Agent.Lead
{
    public enum LeadPersonality
    {
        Analytical,
        Creative,
        Pragmatic
    }

    [System.Serializable]
    public class LeadCandidate
    {
        public string Id;
        public string DisplayName;
        public LeadPersonality Personality;
        public string Description;
        public string SpriteKey;
    }
}
