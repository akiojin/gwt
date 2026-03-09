using Cysharp.Threading.Tasks;
using System.Collections.Generic;
using System.Threading;

namespace Gwt.Core.Models
{
    public interface IAIApiService
    {
        UniTask<string> SuggestBranchNameAsync(string description, ResolvedAISettings settings, CancellationToken ct = default);
        UniTask<string> GenerateCommitMessageAsync(string diff, ResolvedAISettings settings, CancellationToken ct = default);
        UniTask<string> GeneratePrDescriptionAsync(string commits, string diff, ResolvedAISettings settings, CancellationToken ct = default);
        UniTask<string> SummarizeIssueAsync(string issueBody, ResolvedAISettings settings, CancellationToken ct = default);
        UniTask<string> ReviewCodeAsync(string diff, ResolvedAISettings settings, CancellationToken ct = default);
        UniTask<string> GenerateTestsAsync(string code, ResolvedAISettings settings, CancellationToken ct = default);
        UniTask<string> ChatAsync(List<ChatMessage> messages, ResolvedAISettings settings, CancellationToken ct = default);
        UniTask<AIResponse> SendRequestAsync(string systemPrompt, string userMessage, ResolvedAISettings settings, CancellationToken ct = default);
    }
}
