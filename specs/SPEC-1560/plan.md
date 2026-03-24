### 技術コンテキスト

- Unity 6 AudioMixer（3チャンネル: BGM/SFX/UI）
- AudioSource プーリング（ObjectPool）で効果音同時再生を効率化
- ScriptableObject でサウンドイベント定義（SOAP 連携）
- TTS ダッキングは AudioMixer の Snapshot 切替で実装
- VContainer DI（Singleton）

### 実装アプローチ

1. AudioManager 基盤を Phase 1 で実装（音量管理、チャンネル制御、永続化）
2. Phase 5 でサウンドアセット（BGM、効果音、UI 音、ジングル）を制作・統合
3. サウンドアセットはAI生成（Suno/Udio等）で調達。ライセンス確認必須

### フェーズ分割

```
Phase S: セットアップ
  └─ IAudioService インターフェース定義、AudioMixer 設定

Phase F: 基盤
  └─ 音量管理（マスター + チャンネル別）、音量設定永続化

Phase U: ユーザーストーリー実装
  └─ BGM 再生・ループ・クロスフェード
  └─ CI 状態連動 BGM 切替
  └─ 効果音再生システム（AudioSource プーリング）
  └─ UI 音再生システム
  └─ TTS ダッキング
  └─ RPGジングルシステム

Phase FIN: 最終化
  └─ サウンドアセット AI 生成・統合、統合テスト
```

### リスク

- ~~レトロチップチューンの BGM 調達: Unity Asset Store or 外部委託のリードタイムが不明~~ → AI生成で調達。リードタイムは短縮されるが、品質・ライセンスの確認が必要
- TTS とのダッキングタイミング調整: TTS エンジンの遅延に依存
