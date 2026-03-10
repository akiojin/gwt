namespace Gwt.Infra.Services
{
    public enum BgmType
    {
        Normal,
        CISuccess,
        CIFail
    }

    public enum SfxType
    {
        DeskAppear,
        DeskRemove,
        IssueMarker,
        AgentHire,
        AgentFire,
        Notification,
        ButtonClick,
        PanelOpen,
        PanelClose
    }

    public interface ISoundService
    {
        void PlayBgm(BgmType type);
        void StopBgm();
        void PlaySfx(SfxType type);
        void SetBgmVolume(float volume);
        void SetSfxVolume(float volume);
        float BgmVolume { get; }
        float SfxVolume { get; }
        bool IsMuted { get; set; }
        BgmType? CurrentBgm { get; }
        SfxType? LastSfx { get; }
    }
}
