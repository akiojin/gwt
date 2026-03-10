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
        /// <summary>TTS 用ボイスキー。各候補に異なる声を割り当てる。</summary>
        public string VoiceKey;
    }
}
