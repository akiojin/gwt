### Runtime Models
- `AudioChannelState`
  - `CurrentBgm: BgmType?`
  - `LastSfx: SfxType?`
  - `LastJingle: JingleType?`
  - `BgmVolume: float`
  - `SfxVolume: float`
  - `IsMuted: bool`
  - `IsDuckActive: bool`
- `AudioSettingsSnapshot`
  - `BgmEnabled: bool`
  - `SeEnabled: bool`
  - `BgmVolume: float`
  - `SeVolume: float`
- `BgmType`
  - `Field` (通常作業時フィールドBGM)
  - `FieldBright` (CI全Green時)
  - `Battle` (CI失敗時戦闘BGM)
- `JingleType`
  - `Victory` (PRマージ成功)
  - `LevelUp` (レベルアップ)
  - `BattleVictory` (CI失敗→復旧)

### Service Contract
- `ISoundService`
  - `PlayBgm(type)` は現在 BGM を更新する
  - `StopBgm()` は現在 BGM を解除する
  - `PlaySfx(type)` は SFX 再生要求を処理する
  - `PlayJingle(type)` はBGMを一時中断してジングルを再生し、完了後BGMを復帰する
  - `SetBgmVolume(volume)` / `SetSfxVolume(volume)` は永続化対象の音量を更新する
  - `IsMuted` は master mute を表す

### Persistence
- ユーザー設定は `Settings.Sound` に保存する。
- `BgmEnabled`, `SeEnabled`, `BgmVolume`, `SeVolume` を同期対象とする。

### Integration Points
- `#1551 Voice` から TTS ダッキング要求を受ける。
- `#1548 HUD/UI` から UI click/panel open/panel close を受ける。
- `#1555 Gamification` の level up event と接続し、`JingleType.LevelUp` を再生する。
- CI状態変化で `BgmType.Battle` / `BgmType.Field` を切り替える。
- PRマージ検知で `JingleType.Victory` を再生する。
