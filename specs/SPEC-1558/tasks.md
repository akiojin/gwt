### Phase S: セットアップ
- [ ] T001 [S] [US-1] `IMultiProjectService` インターフェース定義
- [ ] T002 [S] [US-1] データ型定義（ProjectSwitchRequest, ProjectSwitchSnapshot）

### Phase F: 基盤
- [ ] T003 [F] [US-3] プロジェクトコンテキスト管理（複数プロジェクトの全PTYプロセス同時管理、制限なし）
- [ ] T004 [F] [US-1] Additive Scene アンロード→ロードの切替メカニズム実装 (`SceneManager.LoadSceneAsync + LoadSceneMode.Additive`)

### Phase U: ユーザーストーリー実装
- [ ] T005 [U] [US-4] スタジオ状態の永続化（切替前のデスク配置・Issue マーカー・エージェント状態を保存）
- [ ] T006 [U] [US-4] スタジオ状態の復元（切替後に前回の状態を再構築）
- [ ] T007 [U] [US-1] `Cmd+`` ホットキーバインド実装
- [ ] T008 [U] [US-1,US-5] プロジェクト切替メニュー UI（最近開いたプロジェクト一覧）
- [ ] T009 [U] [US-2] フェード/トランジション演出実装（0.5秒以内）
- [ ] T010 [U] [US-3] バックグラウンド PTY プロセスのライフサイクル管理（一時停止なし、全維持）
- [ ] T011 [U] [US-3] 非アクティブプロジェクトPTY継続動作の検証テスト

### Phase FIN: 最終化
- [ ] T012 [FIN] [US-1] VContainer DI 登録
- [ ] T013 [FIN] [US-1,US-2,US-3,US-4,US-5] ユニットテスト作成
- [ ] T014 [FIN] [US-1,US-2,US-3,US-4,US-5] 統合テスト作成
