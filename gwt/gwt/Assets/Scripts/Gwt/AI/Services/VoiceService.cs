using Cysharp.Threading.Tasks;
using System.Threading;

namespace Gwt.AI.Services
{
    public interface IVoiceService
    {
        bool IsAvailable { get; }
        bool IsRecording { get; }
        bool IsSpeaking { get; }
        string LastTranscript { get; }
        string LastSpokenText { get; }
        string LastVoiceId { get; }
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
        private string _lastSpokenText = string.Empty;
        private string _lastVoiceId = string.Empty;

        public bool IsAvailable => true;
        public bool IsRecording => _isRecording;
        public bool IsSpeaking => _isSpeaking;
        public string LastTranscript => _lastTranscript;
        public string LastSpokenText => _lastSpokenText;
        public string LastVoiceId => _lastVoiceId;

        public UniTask<string> StartRecordingAsync(CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();
            _isRecording = true;
            if (string.IsNullOrWhiteSpace(_lastTranscript))
                _lastTranscript = string.Empty;
            return UniTask.FromResult(_lastTranscript);
        }

        public void StopRecording()
        {
            if (_isRecording && string.IsNullOrWhiteSpace(_lastTranscript))
                _lastTranscript = "Recorded voice note";
            _isRecording = false;
        }

        public UniTask SpeakAsync(string text, string voiceId, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();
            _lastSpokenText = text ?? string.Empty;
            _lastVoiceId = voiceId ?? string.Empty;
            _isSpeaking = !string.IsNullOrWhiteSpace(text);
            return UniTask.CompletedTask;
        }

        public void StopSpeaking()
        {
            _isSpeaking = false;
        }
    }
}
