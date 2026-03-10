namespace Gwt.Studio.Entity
{
    /// <summary>
    /// Developer（スタッフ）のランダム名を生成する。
    /// エージェント種別（Claude Code等）は名前とは別にラベルで表示。
    /// </summary>
    public static class RandomNameGenerator
    {
        /// <summary>
        /// ランダムな人名を生成する。
        /// </summary>
        /// <returns>生成された名前（空文字列は不可）</returns>
        public static string Generate()
        {
            // TODO: 実装（TDD RED 状態）
            return "";
        }

        /// <summary>
        /// エージェント種別に対応するラベル文字列を取得する。
        /// </summary>
        public static string GetAgentTypeLabel(Gwt.Agent.Services.DetectedAgentType agentType)
        {
            // TODO: 実装（TDD RED 状態）
            return "";
        }
    }
}
