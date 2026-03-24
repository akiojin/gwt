### RED テスト方針
- `SoundService` のユニットテストを追加し、BGM/SFX 再生状態、ミュート時の抑制、音量クランプ、ダッキング係数の適用を先に RED で固定する。
- BGM 切り替えのテストを追加し、通常フィールドBGM/CI success バリエーション/戦闘BGM（CI fail）の状態変化時に目標 BGM 状態が更新されることを確認する。
- TTS ダッキングのテストを追加し、TTS 再生開始で BGM 音量が下がり、終了で元に戻ることを確認する。
- RPGジングルのテストを追加し、PRマージ/レベルアップ/CI復旧時にジングル再生→BGM復帰の遷移を確認する。
- AudioSource 実体には依存せず、サービス層の state machine を RED/Green する。

### 受け入れテストケース
- `PlayBgm(type)` 実行で現在 BGM 状態が更新される。
- `StopBgm()` 実行で BGM 状態が停止になる。
- `PlaySfx(type)` 実行で最新 SFX 履歴が更新される。
- `PlayJingle(type)` 実行でBGMが一時中断され、ジングル再生後にBGMが復帰する。
- `SetBgmVolume` / `SetSfxVolume` は 0..1 に clamp される。
- `IsMuted=true` 時は再生要求を受けても状態を更新しない、または出力ゲイン 0 扱いになる。
- TTS ダッキング開始/終了で実効 BGM 音量が変化する。
