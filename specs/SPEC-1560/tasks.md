### Phase S: セットアップ
- [ ] T001 [S] [US-5] AudioManager サービスインターフェース定義 (IAudioService)
- [ ] T002 [S] [US-1] AudioMixer 設定（BGM/SFX/UI チャンネル）

### Phase F: 基盤
- [ ] T003 [F] [US-5] 音量管理実装（マスター + チャンネル別）+ テスト
- [ ] T004 [F] [US-5] 音量設定永続化実装 + テスト

### Phase U: ユーザーストーリー実装
- [ ] T005 [U] [US-1] BGM 再生・ループ・クロスフェード実装 + テスト
- [ ] T006 [U] [US-1] CI 状態連動 BGM 切替実装（フィールドBGM ↔ 戦闘BGM）+ テスト
- [ ] T007 [U] [US-2] 効果音再生システム実装（AudioSource プーリング）+ テスト
- [ ] T008 [U] [US-3] UI 音再生システム実装 + テスト
- [ ] T009 [U] [US-4] TTS ダッキング実装（AudioMixer Snapshot）+ テスト
- [ ] T010 [U] [US-7,US-8] RPGジングルシステム実装（勝利ジングル、レベルアップジングル等）+ テスト
- [ ] T011 [U] [US-1] サウンドイベント定義 (ScriptableObject)

### Phase FIN: 最終化
- [ ] T012 [FIN] [US-5] VContainer DI 登録 (IAudioService)
- [ ] T013 [FIN] [US-1] BGM アセット AI生成・ライセンス確認・統合
- [ ] T014 [FIN] [US-2] 効果音アセット AI生成・ライセンス確認・統合
- [ ] T015 [FIN] [US-3] UI 音アセット AI生成・ライセンス確認・統合
- [ ] T016 [FIN] [US-7,US-8] RPGジングルアセット AI生成・ライセンス確認・統合
- [ ] T017 [FIN] [US-1,US-2,US-3,US-4,US-5,US-6,US-7,US-8] 統合テスト
