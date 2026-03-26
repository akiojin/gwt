### Phase S: Setup
- [ ] T001 [S] [US-1] IHudService / INotificationService インターフェース定義・VContainer 登録

### Phase F: Foundation
- [ ] T002 [F] [US-1] RTS 風コンソールウィンドウ UI（ScrollRect、テキストプーリング、フィルタリング）
- [ ] T003 [F] [US-4] HUD 常時表示要素（プロジェクト名、Active Agent数、Open PR数、ステータスインジケーター、worktree 作成ボタン）
- [ ] T004 [F] [US-4] Lead 指示入力: HUD 常時表示入力フィールド（DecreeInput 相当、ミーティングルーム不要）
- [ ] T005 [F] [US-1] オーバーレイパネルシステム（エンティティ詳細、フォーム入力）
- [ ] T006 [F] [US-13] フローティングターミナルオーバーレイパネル実装

### Phase U: User Story
- [ ] T007 [U] [US-2] スタジオ内演出通知システム（World Space Canvas、!マーク、エフェクト）
- [ ] T008 [U] [US-14] Lead 質問 UI（浮遊「?」マーカー + バルーン + 選択肢ボタン、World Space Canvas）
- [ ] T009 [U] [US-2] エラー通知3段階分類・ルーティング（Error=トースト+コンソール、Warning=コンソールのみ、Info=ログのみ）
- [ ] T010 [U] [US-3] ESC ゲームスタイル設定メニュー（現行 gwt 全設定移植 + Lead キャラクター設定、ジョブタイプ設定、音声入力/出力設定、キーバインド設定、言語設定）
- [ ] T011 [U] [US-7] フォーカスベースのキーバインド切替システム（ターミナル/スタジオ）
- [ ] T012 [U] [US-15] SPEC エディタパネル（左=チャット、右=マークダウンプレビューの2ペイン構成、オーバーレイシステムに統合）
- [ ] T013 [U] [US-6] Git 詳細オーバーレイパネル（diff ビュー、コミット履歴、stash 情報）
- [ ] T014 [U] [US-10] Issue マーカー UI（マーカークリック → Issue 詳細オーバーレイ + エージェント雇用ボタン）
- [ ] T015 [U] [US-11] 半透明デスク UI（クリック → worktree 作成 + エージェント雇用フロー）
- [ ] T016 [U] [US-12] worktree 作成 UI（HUD ボタン + Lead 指示の両方対応）
- [ ] T017 [U] [US-3] 確認ダイアログシステム（PRマージ、worktree削除等）
- [ ] T018 [U] [US-5] 自前 GFM パーサー実装（Markdig 不使用、将来パッケージ化前提）
- [ ] T019 [U] [US-5] TextMeshPro リッチテキスト変換レンダラー実装（テーブル、コードブロック+シンタックスハイライト、画像、チェックボックス、リンク、リスト等）
- [ ] T020 [U] [US-9] Unity Localization パッケージ導入（Smart Strings + アセットテーブル）
- [ ] T021 [U] [US-9] Localization テーブル作成（英語 + 日本語）
- [ ] T022 [U] [US-9] 全 UI テキストの Localization テーブル経由化
- [ ] T023 [U] [US-16] デフォルト言語の OS システム言語追従実装
- [ ] T024 [U] [US-9] ESC メニュー言語切替実装
- [ ] T025 [U] [US-1] コンテキストメニュー Screen Space 描画実装
- [ ] T026 [U] [US-1] シングルウィンドウ完結の検証

### Phase FIN: Finalization
- [ ] T027 [FIN] [US-1] パフォーマンステスト（NFR-001〜003 検証）
- [ ] T028 [FIN] [US-1] US-1〜US-16 の受け入れテスト
