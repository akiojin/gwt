### 技術コンテキスト

- Unity 6 + Canvas (Screen Space - Overlay / World Space)
- VContainer（DI）
- UniTask（非同期処理）
- Unity Localization パッケージ（Smart Strings + アセットテーブル）
- Pixel UI &amp; HUD / Kenney Pixel UI Pack アセット
- TextMeshPro（リッチテキスト描画）

### 実装アプローチ

UI システムを HUD レイヤー（常時表示: Lead入力フィールド、worktree作成ボタン含む）、オーバーレイレイヤー（操作時表示: Git詳細、Issue詳細、エージェント雇用、フローティングターミナル、SPECエディタパネル含む）、スタジオ内エフェクトレイヤー（トップダウン ¾ ビュー空間内: Issueマーカー、半透明デスク、Lead質問UI含む）の 3 層に分離して設計する。各層は `IHudService` / `INotificationService` を通じて制御し、VContainer で DI 統合する。

### 設計方針

- HUD は Canvas (Screen Space - Overlay) でトップダウン ¾ ビューシーンの最前面に描画
- Lead 入力フィールドは HUD に常時表示し、ミーティングルームを経由せず直接指示を送信可能にする
- コンソールウィンドウは ScrollRect + テキストプーリングで大量ログに対応
- スタジオ内エフェクト（!マーク、?マーク、Issue マーカー、半透明デスク等）は World Space Canvas でトップダウン ¾ ビュー空間内に配置
- Lead 質問 UI は浮遊「?」マーカー + バルーン + 選択肢ボタンで、Issue「!」マーカー（#1547）と同様の視覚言語を使用
- 設定メニューは Time.timeScale = 0 でゲームを一時停止して表示
- Git 詳細情報（diff、コミット、stash）はデスク（Worktree）クリック時のオーバーレイパネルに統合
- **ターミナルはスタジオビュー上にフローティングするオーバーレイパネルとして表示**
- SPEC エディタパネルは左=チャット、右=マークダウンプレビューの2ペイン構成で、オーバーレイシステムに統合
- マークダウンレンダリングは完全 GFM 対応（テーブル、コードブロック+シンタックスハイライト、画像、チェックボックス、リンク、リスト等）。**自前 GFM パーサー + TextMeshPro レンダラーで実装する（Markdig 等の外部パーサーは不使用、将来パッケージ化前提）。** GitHub の Issue/PR 本文の見た目を再現する
- キーバインドはフォーカスベースで切替（ターミナルフォーカス時: ターミナルキーバインド優先、スタジオフォーカス時: アプリショートカット優先）
- Git操作はターミナル経由で実行（ゲームライク Git UI は不要）
- **ローカライゼーションは Unity Localization パッケージ (Smart Strings + アセットテーブル) を使用**。英語 + 日本語対応、ESC メニューから切替。**デフォルト言語は OS のシステム言語設定に追従する**
- **全 UI テキストは Localization テーブル経由で取得し、ハードコード文字列を禁止する**
- 全操作をシングルウィンドウ内で完結させる
- Issue マーカークリック → オーバーレイで Issue 詳細 + エージェント雇用ボタン
- 半透明デスククリック → worktree 作成 + エージェント雇用フロー
- worktree 作成は HUD ボタンと Lead 指示の両方から可能
- **エラー通知は3段階: Error=トースト+コンソール、Warning=コンソールのみ、Info=ログのみ**
- **コンテキストメニューは Screen Space で描画する**

### フェーズ分割

**Phase S（Setup）:** DI基盤、IHudService/INotificationServiceインターフェース定義
**Phase F（Foundation）:** HUDレイヤー、コンソールウィンドウ、オーバーレイパネル基盤
**Phase U（User Story）:** 各UIコンポーネント実装、GFMパーサー、ローカライゼーション、SPECエディタ
**Phase FIN（Finalization）:** パフォーマンス検証、受け入れテスト

### リスク

- マークダウンレンダリングの品質（自前実装のため、GFM 仕様の網羅度に段階的対応が必要）
- HUD 要素のパフォーマンスへの影響 → Canvas の rebatch 最小化で対応
- ローカライゼーション対応のテキスト管理 → Unity Localization パッケージを活用
