using UnityEngine;

namespace Gwt.Infra.Services
{
    public class SoundService : ISoundService
    {
        private float _bgmVolume = 0.7f;
        private float _sfxVolume = 1.0f;
        private bool _isMuted;

        public float BgmVolume => _bgmVolume;
        public float SfxVolume => _sfxVolume;
        public bool IsMuted { get => _isMuted; set => _isMuted = value; }

        public void PlayBgm(BgmType type) => Debug.Log($"[SoundService] PlayBgm: {type} (stub)");
        public void StopBgm() => Debug.Log("[SoundService] StopBgm (stub)");
        public void PlaySfx(SfxType type) => Debug.Log($"[SoundService] PlaySfx: {type} (stub)");
        public void SetBgmVolume(float volume) => _bgmVolume = Mathf.Clamp01(volume);
        public void SetSfxVolume(float volume) => _sfxVolume = Mathf.Clamp01(volume);
    }
}
