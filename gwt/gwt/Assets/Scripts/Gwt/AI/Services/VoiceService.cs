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
        private bool _isRecording;
        private bool _isSpeaking;
        private string _lastTranscript = string.Empty;

        public bool IsAvailable => true;
        public bool IsRecording => _isRecording;

        public UniTask<string> StartRecordingAsync(CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();
            _isRecording = true;
            _lastTranscript = string.Empty;
            return UniTask.FromResult(_lastTranscript);
        }

        public void StopRecording()
        {
            _isRecording = false;
        }

        public UniTask SpeakAsync(string text, string voiceId, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();
            _isSpeaking = !string.IsNullOrWhiteSpace(text);
            _isSpeaking = false;
            return UniTask.CompletedTask;
        }

        public void StopSpeaking()
        {
            _isSpeaking = false;
        }
    }
}
