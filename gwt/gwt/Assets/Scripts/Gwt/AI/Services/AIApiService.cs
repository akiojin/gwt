using Cysharp.Threading.Tasks;
using System.Collections.Generic;
using System.Text;
using System.Threading;
using Gwt.Core.Models;
using UnityEngine;
using UnityEngine.Networking;

namespace Gwt.AI.Services
{
    [System.Serializable]
    public class AIRequestMessage
    {
        public string role;
        public string content;
    }

    [System.Serializable]
    public class AIRequest
    {
        public string model;
        public List<AIRequestMessage> messages;
        public float temperature = 0.7f;
        public int max_tokens = 4096;
    }

    [System.Serializable]
    public class AIResponseChoice
    {
        public AIRequestMessage message;
        public string finish_reason;
    }

    [System.Serializable]
    public class AIResponseUsage
    {
        public int prompt_tokens;
        public int completion_tokens;
        public int total_tokens;
    }

    [System.Serializable]
    public class AIApiResponse
    {
        public string id;
        public List<AIResponseChoice> choices;
        public AIResponseUsage usage;
    }

    [System.Serializable]
    public class AIModelInfo
    {
        public string id;
        public string owned_by;
    }

    [System.Serializable]
    public class AIModelListResponse
    {
        public List<AIModelInfo> data;
    }

    public class AIApiService
    {
        private async UniTask<string> SendChatRequestAsync(
            string systemPrompt, string userMessage,
            ResolvedAISettings settings, CancellationToken ct)
        {
            var request = new AIRequest
            {
                model = settings.Model,
                messages = new List<AIRequestMessage>
                {
                    new AIRequestMessage { role = "system", content = systemPrompt },
                    new AIRequestMessage { role = "user", content = userMessage }
                },
                temperature = 0.7f,
                max_tokens = 4096
            };

            var json = JsonUtility.ToJson(request);
            var url = settings.Endpoint.TrimEnd('/') + "/chat/completions";

            using var webRequest = new UnityWebRequest(url, "POST");
            var bodyBytes = Encoding.UTF8.GetBytes(json);
            webRequest.uploadHandler = new UploadHandlerRaw(bodyBytes);
            webRequest.downloadHandler = new DownloadHandlerBuffer();
            webRequest.SetRequestHeader("Content-Type", "application/json");
            webRequest.SetRequestHeader("Authorization", $"Bearer {settings.ApiKey}");

            await webRequest.SendWebRequest().ToUniTask(cancellationToken: ct);

            if (webRequest.result != UnityWebRequest.Result.Success)
            {
                var errorMsg = webRequest.responseCode switch
                {
                    401 => "Authentication failed. Check your API key.",
                    429 => "Rate limit exceeded. Please try again later.",
                    >= 500 => $"Server error ({webRequest.responseCode}). Please try again later.",
                    _ => $"Request failed: {webRequest.error}"
                };
                Debug.LogError($"[AIApiService] {errorMsg}");
                return null;
            }

            var responseText = webRequest.downloadHandler.text;
            if (string.IsNullOrEmpty(responseText))
            {
                Debug.LogError("[AIApiService] Empty response from API");
                return null;
            }

            var response = JsonUtility.FromJson<AIApiResponse>(responseText);
            if (response?.choices == null || response.choices.Count == 0)
            {
                Debug.LogError("[AIApiService] No choices in API response");
                return null;
            }

            return response.choices[0].message?.content;
        }

        public async UniTask<string> SuggestBranchNameAsync(
            string description, ResolvedAISettings settings, CancellationToken ct)
        {
            var systemPrompt = "You are a git branch naming assistant. Given a task description, suggest a concise branch name following the convention: type/short-description (e.g., feat/add-login, fix/null-pointer). Return ONLY the branch name, nothing else.";
            return await SendChatRequestAsync(systemPrompt, description, settings, ct);
        }

        public async UniTask<string> GenerateCommitMessageAsync(
            string diff, ResolvedAISettings settings, CancellationToken ct)
        {
            var systemPrompt = "You are a commit message generator. Given a git diff, generate a concise Conventional Commits message (feat:/fix:/docs:/refactor:/chore: etc.). Return ONLY the commit message, nothing else.";
            return await SendChatRequestAsync(systemPrompt, $"Generate a commit message for this diff:\n\n{diff}", settings, ct);
        }

        public async UniTask<string> GeneratePrDescriptionAsync(
            string commits, string diff, ResolvedAISettings settings, CancellationToken ct)
        {
            var systemPrompt = "You are a pull request description generator. Given commits and a diff, generate a clear PR title and description in markdown format.";
            return await SendChatRequestAsync(systemPrompt, $"Commits:\n{commits}\n\nDiff:\n{diff}", settings, ct);
        }

        public async UniTask<string> SummarizeIssueAsync(
            string issueBody, ResolvedAISettings settings, CancellationToken ct)
        {
            var systemPrompt = "Summarize the following GitHub issue in 2-3 sentences, focusing on the problem and expected outcome.";
            return await SendChatRequestAsync(systemPrompt, issueBody, settings, ct);
        }

        public async UniTask<string> ReviewCodeAsync(
            string diff, ResolvedAISettings settings, CancellationToken ct)
        {
            var systemPrompt = "Review the following code diff. Identify potential bugs, security issues, and improvement suggestions. Be concise.";
            return await SendChatRequestAsync(systemPrompt, diff, settings, ct);
        }

        public async UniTask<string> GenerateTestsAsync(
            string code, ResolvedAISettings settings, CancellationToken ct)
        {
            var systemPrompt = "Generate unit tests for the following code. Use NUnit framework with [Test] attributes. Focus on edge cases and important behaviors.";
            return await SendChatRequestAsync(systemPrompt, code, settings, ct);
        }

        public async UniTask<string> LeadJudgmentAsync(
            string context, string question, ResolvedAISettings settings, CancellationToken ct)
        {
            var systemPrompt = "You are the Lead of a development studio. Analyze the context and provide a clear, actionable decision. Be decisive and concise.";
            return await SendChatRequestAsync(systemPrompt, $"Context:\n{context}\n\nQuestion:\n{question}", settings, ct);
        }

        public async UniTask<string> ChatAsync(
            List<AIRequestMessage> messages, ResolvedAISettings settings, CancellationToken ct)
        {
            var request = new AIRequest
            {
                model = settings.Model,
                messages = messages,
                temperature = 0.7f,
                max_tokens = 4096
            };

            var json = JsonUtility.ToJson(request);
            var url = settings.Endpoint.TrimEnd('/') + "/chat/completions";

            using var webRequest = new UnityWebRequest(url, "POST");
            var bodyBytes = Encoding.UTF8.GetBytes(json);
            webRequest.uploadHandler = new UploadHandlerRaw(bodyBytes);
            webRequest.downloadHandler = new DownloadHandlerBuffer();
            webRequest.SetRequestHeader("Content-Type", "application/json");
            webRequest.SetRequestHeader("Authorization", $"Bearer {settings.ApiKey}");

            await webRequest.SendWebRequest().ToUniTask(cancellationToken: ct);

            if (webRequest.result != UnityWebRequest.Result.Success)
            {
                Debug.LogError($"[AIApiService] Chat request failed: {webRequest.error}");
                return null;
            }

            var responseText = webRequest.downloadHandler.text;
            if (string.IsNullOrEmpty(responseText))
            {
                Debug.LogError("[AIApiService] Empty response from API");
                return null;
            }

            var response = JsonUtility.FromJson<AIApiResponse>(responseText);
            if (response?.choices == null || response.choices.Count == 0)
            {
                Debug.LogError("[AIApiService] No choices in API response");
                return null;
            }

            return response.choices[0].message?.content;
        }

        public async UniTask<List<AIModelInfo>> ListModelsAsync(
            ResolvedAISettings settings, CancellationToken ct)
        {
            var url = settings.Endpoint.TrimEnd('/') + "/models";

            using var webRequest = UnityWebRequest.Get(url);
            webRequest.SetRequestHeader("Authorization", $"Bearer {settings.ApiKey}");

            await webRequest.SendWebRequest().ToUniTask(cancellationToken: ct);

            if (webRequest.result != UnityWebRequest.Result.Success)
            {
                Debug.LogError($"[AIApiService] ListModels failed: {webRequest.error}");
                return new List<AIModelInfo>();
            }

            var responseText = webRequest.downloadHandler.text;
            if (string.IsNullOrEmpty(responseText))
            {
                Debug.LogError("[AIApiService] Empty response from models endpoint");
                return new List<AIModelInfo>();
            }

            var response = JsonUtility.FromJson<AIModelListResponse>(responseText);
            return response?.data ?? new List<AIModelInfo>();
        }

        public static string MaskApiKey(string apiKey)
        {
            if (string.IsNullOrEmpty(apiKey) || apiKey.Length < 8) return "***";
            return apiKey[..4] + "..." + apiKey[^4..];
        }
    }
}
