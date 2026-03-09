using Cysharp.Threading.Tasks;
using System.Threading;

namespace Gwt.AI.Services
{
    public interface IVoiceService
    {
        bool IsAvailable { get; }
        bool IsRecording { get; }
        UniTask<string> StartRecordingAsync(CancellationToken ct = default);
        void StopRecording();
        UniTask SpeakAsync(string text, string voiceId, CancellationToken ct = default);
        void StopSpeaking();
    }

    public class VoiceService : IVoiceService
    {
        public bool IsAvailable => false;
        public bool IsRecording => false;

        public UniTask<string> StartRecordingAsync(CancellationToken ct = default)
        {
            UnityEngine.Debug.LogWarning("[VoiceService] STT not yet implemented");
            return UniTask.FromResult(string.Empty);
        }

        public void StopRecording()
        {
            UnityEngine.Debug.LogWarning("[VoiceService] STT not yet implemented");
        }

        public UniTask SpeakAsync(string text, string voiceId, CancellationToken ct = default)
        {
            UnityEngine.Debug.LogWarning("[VoiceService] TTS not yet implemented");
            return UniTask.CompletedTask;
        }

        public void StopSpeaking()
        {
            UnityEngine.Debug.LogWarning("[VoiceService] TTS not yet implemented");
        }
    }
}
