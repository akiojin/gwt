using System.Collections.Generic;
using Gwt.AI.Services;
using Gwt.Core.Models;
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

        // --- ResolvedAISettings Tests ---

        [Test]
        public void ResolvedAISettings_SerializesAllFields()
        {
            var settings = new ResolvedAISettings
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
    }
}
