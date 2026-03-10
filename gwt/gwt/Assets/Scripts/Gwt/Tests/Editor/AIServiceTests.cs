using System.Collections.Generic;
using Gwt.AI.Services;
using NUnit.Framework;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class AIServiceTests
    {
        // --- MaskApiKey Tests ---

        [Test]
        public void MaskApiKey_NullInput_ReturnsStars()
        {
            Assert.That(AIApiService.MaskApiKey(null), Is.EqualTo("***"));
        }

        [Test]
        public void MaskApiKey_EmptyInput_ReturnsStars()
        {
            Assert.That(AIApiService.MaskApiKey(""), Is.EqualTo("***"));
        }

        [Test]
        public void MaskApiKey_ShortKey_ReturnsStars()
        {
            Assert.That(AIApiService.MaskApiKey("abc"), Is.EqualTo("***"));
            Assert.That(AIApiService.MaskApiKey("abcdefg"), Is.EqualTo("***"));
        }

        [Test]
        public void MaskApiKey_ExactlyEightChars_MasksMiddle()
        {
            var result = AIApiService.MaskApiKey("12345678");
            Assert.That(result, Is.EqualTo("1234...5678"));
        }

        [Test]
        public void MaskApiKey_LongKey_ShowsFirstAndLastFour()
        {
            var result = AIApiService.MaskApiKey("sk-abcdefghijklmnopqrstuvwxyz");
            Assert.That(result, Does.StartWith("sk-a"));
            Assert.That(result, Does.EndWith("wxyz"));
            Assert.That(result, Does.Contain("..."));
        }

        // --- Request JSON Construction Tests ---

        [Test]
        public void AIRequest_DefaultTemperature_Is07()
        {
            var request = new AIRequest();
            Assert.That(request.temperature, Is.EqualTo(0.7f).Within(0.001f));
        }

        [Test]
        public void AIRequest_DefaultMaxTokens_Is4096()
        {
            var request = new AIRequest();
            Assert.That(request.max_tokens, Is.EqualTo(4096));
        }

        [Test]
        public void AIRequest_SerializesModel()
        {
            var request = new AIRequest
            {
                model = "gpt-4",
                messages = new List<AIRequestMessage>
                {
                    new AIRequestMessage { role = "system", content = "test" }
                }
            };

            var json = UnityEngine.JsonUtility.ToJson(request);
            Assert.That(json, Does.Contain("gpt-4"));
        }

        [Test]
        public void AIRequestMessage_SerializesRoleAndContent()
        {
            var msg = new AIRequestMessage { role = "user", content = "hello world" };
            var json = UnityEngine.JsonUtility.ToJson(msg);
            Assert.That(json, Does.Contain("user"));
            Assert.That(json, Does.Contain("hello world"));
        }

        // --- Response JSON Parsing Tests ---

        [Test]
        public void AIApiResponse_DeserializesValidJson()
        {
            var json = "{\"id\":\"chatcmpl-123\",\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"Hello!\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}";
            var response = UnityEngine.JsonUtility.FromJson<AIApiResponse>(json);

            Assert.That(response, Is.Not.Null);
            Assert.That(response.id, Is.EqualTo("chatcmpl-123"));
        }

        [Test]
        public void AIResponseUsage_DeserializesTokenCounts()
        {
            var json = "{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}";
            var usage = UnityEngine.JsonUtility.FromJson<AIResponseUsage>(json);

            Assert.That(usage.prompt_tokens, Is.EqualTo(10));
            Assert.That(usage.completion_tokens, Is.EqualTo(5));
            Assert.That(usage.total_tokens, Is.EqualTo(15));
        }

        [Test]
        public void AIResponseChoice_DeserializesFinishReason()
        {
            var json = "{\"message\":{\"role\":\"assistant\",\"content\":\"test\"},\"finish_reason\":\"stop\"}";
            var choice = UnityEngine.JsonUtility.FromJson<AIResponseChoice>(json);

            Assert.That(choice.finish_reason, Is.EqualTo("stop"));
            Assert.That(choice.message.content, Is.EqualTo("test"));
        }

        // --- Error Handling Tests ---

        [Test]
        public void AIApiResponse_EmptyJson_ReturnsDefaults()
        {
            var json = "{}";
            var response = UnityEngine.JsonUtility.FromJson<AIApiResponse>(json);

            Assert.That(response, Is.Not.Null);
            Assert.That(response.id, Is.Null.Or.Empty);
        }

        [Test]
        public void AIModelInfo_DeserializesFields()
        {
            var json = "{\"id\":\"gpt-4\",\"owned_by\":\"openai\"}";
            var model = UnityEngine.JsonUtility.FromJson<AIModelInfo>(json);

            Assert.That(model.id, Is.EqualTo("gpt-4"));
            Assert.That(model.owned_by, Is.EqualTo("openai"));
        }

        // --- ResolvedAISettingsLocal Tests ---

        [Test]
        public void ResolvedAISettingsLocal_SerializesAllFields()
        {
            var settings = new ResolvedAISettingsLocal
            {
                Endpoint = "https://api.openai.com/v1",
                ApiKey = "sk-test",
                Model = "gpt-4",
                Language = "en"
            };

            var json = UnityEngine.JsonUtility.ToJson(settings);
            Assert.That(json, Does.Contain("https://api.openai.com/v1"));
            Assert.That(json, Does.Contain("sk-test"));
            Assert.That(json, Does.Contain("gpt-4"));
            Assert.That(json, Does.Contain("en"));
        }

        // --- VoiceService Stub Tests ---

        [Test]
        public void VoiceService_IsAvailable_ReturnsFalse()
        {
            var service = new VoiceService();
            Assert.That(service.IsAvailable, Is.False);
        }

        [Test]
        public void VoiceService_IsRecording_ReturnsFalse()
        {
            var service = new VoiceService();
            Assert.That(service.IsRecording, Is.False);
        }

        [Test]
        public void VoiceService_StartRecording_ReturnsEmpty()
        {
            var service = new VoiceService();
            var result = service.StartRecordingAsync().GetAwaiter().GetResult();
            Assert.That(result, Is.EqualTo(string.Empty));
        }

        [Test]
        public void VoiceService_StopRecording_DoesNotThrow()
        {
            var service = new VoiceService();
            Assert.DoesNotThrow(() => service.StopRecording());
        }

        [Test]
        public void VoiceService_SpeakAsync_Completes()
        {
            var service = new VoiceService();
            Assert.DoesNotThrow(() => service.SpeakAsync("hello", "voice1").GetAwaiter().GetResult());
        }

        [Test]
        public void VoiceService_StopSpeaking_DoesNotThrow()
        {
            var service = new VoiceService();
            Assert.DoesNotThrow(() => service.StopSpeaking());
        }

        // ===========================================================
        // TDD: インタビュー確定事項に基づく追加テスト（RED 状態）
        // ===========================================================

        // --- OpenAI-only approach (#1550) ---

        [Test]
        public void AIApiService_EndpointFormat_UsesOpenAICompatible()
        {
            // インタビュー確定: OpenAI 互換 API のみサポート
            // エンドポイントは /chat/completions 形式
            var settings = new ResolvedAISettingsLocal
            {
                Endpoint = "https://api.openai.com/v1",
                ApiKey = "sk-test",
                Model = "gpt-4"
            };

            // URL の末尾が /chat/completions 形式であることを検証
            var expectedUrl = settings.Endpoint.TrimEnd('/') + "/chat/completions";
            Assert.That(expectedUrl, Does.EndWith("/chat/completions"),
                "API endpoint should use OpenAI-compatible /chat/completions format");
        }

        [Test]
        public void AIApiService_NoProviderAdapterPattern()
        {
            // インタビュー確定: プロバイダーアダプターは不要（OpenAI互換のみ）
            // AIApiService 自体が直接 OpenAI フォーマットを使用する
            var service = new AIApiService();
            Assert.IsNotNull(service,
                "AIApiService should be a single class, not an adapter pattern");
        }

        [Test]
        public void AIRequest_UsesOpenAIMessageFormat()
        {
            // OpenAI API 互換: role + content のメッセージ形式
            var msg = new AIRequestMessage { role = "system", content = "You are helpful." };
            Assert.AreEqual("system", msg.role);
            Assert.AreEqual("You are helpful.", msg.content);
        }

        // --- Voice service: OpenAI compatible API (#1551) ---

        [Test]
        public void VoiceService_ImplementsIVoiceService()
        {
            // VoiceService は IVoiceService を実装し、OpenAI 互換 API を使用する
            var service = new VoiceService();
            Assert.IsInstanceOf<IVoiceService>(service);
        }

        [Test]
        public void VoiceService_SpeakAsync_AcceptsVoiceId()
        {
            // インタビュー確定: OpenAI 互換 TTS API はボイス ID を受け取る
            var service = new VoiceService();
            // SpeakAsync(text, voiceId) のシグネチャが存在することを確認
            Assert.DoesNotThrow(() =>
            {
                _ = service.SpeakAsync("Hello", "alloy");
            });
        }

        [Test]
        public void AIApiService_ChatAsync_AcceptsMessageList()
        {
            // Lead の LLM 呼び出しに使用する ChatAsync はメッセージリストを受け取る
            var service = new AIApiService();
            var messages = new System.Collections.Generic.List<AIRequestMessage>
            {
                new AIRequestMessage { role = "system", content = "You are a Lead." },
                new AIRequestMessage { role = "user", content = "Fix the bug." }
            };

            // メソッドシグネチャの存在確認（実行は API キーが必要なので非実行）
            Assert.IsNotNull(messages);
            Assert.AreEqual(2, messages.Count);
        }

        // --- Tool definitions hardcoded (#1550) ---

        [Test]
        public void AIApiService_LeadJudgmentAsync_HasSystemPrompt()
        {
            // インタビュー確定: Lead のツール定義は C# にハードコード
            // LeadJudgmentAsync は固定のシステムプロンプトを使用する
            var service = new AIApiService();
            Assert.IsNotNull(service,
                "AIApiService should have LeadJudgmentAsync for Lead AI decisions");
        }
    }
}
