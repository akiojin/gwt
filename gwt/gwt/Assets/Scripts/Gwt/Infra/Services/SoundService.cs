using UnityEngine;

namespace Gwt.Infra.Services
{
    public class SoundService : ISoundService
    {
        private float _bgmVolume = 0.7f;
        private float _sfxVolume = 1.0f;
        private bool _isMuted;
        private BgmType? _currentBgm;
        private SfxType? _lastSfx;

        public float BgmVolume => _bgmVolume;
        public float SfxVolume => _sfxVolume;
        public bool IsMuted { get => _isMuted; set => _isMuted = value; }

        public void PlayBgm(BgmType type)
        {
            if (_isMuted) return;
            _currentBgm = type;
            Debug.Log($"[SoundService] PlayBgm: {type} volume={_bgmVolume:0.00}");
        }

        public void StopBgm()
        {
            _currentBgm = null;
            Debug.Log("[SoundService] StopBgm");
        }

        public void PlaySfx(SfxType type)
        {
            if (_isMuted) return;
            _lastSfx = type;
            Debug.Log($"[SoundService] PlaySfx: {type} volume={_sfxVolume:0.00}");
        }

        public void SetBgmVolume(float volume) => _bgmVolume = Mathf.Clamp01(volume);
        public void SetSfxVolume(float volume) => _sfxVolume = Mathf.Clamp01(volume);
    }
}
