using System.Threading;
using Gwt.Agent.Services;

namespace Gwt.Studio.Entity
{
    /// <summary>
    /// Developer（スタッフ）のランダム名を生成する。
    /// エージェント種別（Claude Code等）は名前とは別にラベルで表示。
    /// </summary>
    public static class RandomNameGenerator
    {
        private static readonly string[] Names =
        {
            "Alex",
            "Sam",
            "Jordan",
            "Morgan",
            "Casey",
            "Taylor",
            "Avery",
            "Riley"
        };

        private static int _nameIndex = -1;

        /// <summary>
        /// ランダムな人名を生成する。
        /// </summary>
        /// <returns>生成された名前（空文字列は不可）</returns>
        public static string Generate()
        {
            var index = Interlocked.Increment(ref _nameIndex);
            return Names[index % Names.Length];
        }

        /// <summary>
        /// エージェント種別に対応するラベル文字列を取得する。
        /// </summary>
        public static string GetAgentTypeLabel(DetectedAgentType agentType)
        {
            return agentType switch
            {
                DetectedAgentType.Claude => "Claude Code",
                DetectedAgentType.Codex => "Codex",
                DetectedAgentType.Gemini => "Gemini",
                DetectedAgentType.OpenCode => "OpenCode",
                DetectedAgentType.GithubCopilot => "Copilot",
                _ => "Custom"
            };
        }
    }
}
