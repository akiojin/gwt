### 技術コンテキスト

- Unity 6 + 2D URP + Pixel Perfect Camera
- A* Pathfinding Pro（GridGraph、Tilemap連動）
- VContainer（DI）
- UniTask（非同期処理）
- LimeZu Modern Interiors アセット（キャラクター・家具スプライト）
- New Input System（旧Input API不可）
- IL2CPP（Script Backend）

### 実装アプローチ

エンティティシステムを `IEntityService` として設計し、gwt のバックエンドデータ（エージェント状態、worktree 情報、Issue/PR データ）を 2D スプライトオブジェクトにマッピングする。インタラクションは `IInteractionService` で一元管理し、スプライト選択→オーバーレイ UI パネル表示のフローを統一する。

コアエンティティモデルの依存チェーン（Issue → Worktree(デスク) → Agent(スタッフ)）を基盤として設計する。Leadはこのチェーンの外にいる特別なスタッフとして独立管理する。

### 設計方針

- エンティティは ScriptableObject ベースのデータ定義 + MonoBehaviour のビジュアル表現に分離
- 状態遷移は Animator Controller でスプライトアニメーションを管理し、スクリプトからトリガーで制御
- **コアエンティティモデル**: Issue → Worktree(デスク) → Agent(スタッフ) の依存チェーン。Leadはチェーン外
- **スタジオワールド**: Tilemap＋フリー配置ハイブリッド。床・壁はTilemap、デスク・家具はフリー配置（グリッドスナップ）
- **A* GridGraph**: Tilemap上にGridGraph生成。デスクドラッグ時は部分再計算（バッチ）
- **動的拡張**: デスク数に応じて行単位でTilemap拡張・縮小
- **入退場**: スタジオ入口ドアを経由。入場=ドア→デスク歩行、退場=現在位置→ドア歩行→消滅
- **家具**: 固定配置。stopped状態キャラクターがランダムに家具インタラクション
- **Pixel Perfect Camera**: Unity Pixel Perfect Camera コンポーネント使用
- **キャラクター命名**: ランダム名生成＋エージェント種別ラベル
- **空席デスク**: Fire Agent 後はデスク空席維持。Worktree 削除は空席デスクメニューの別操作
- **1リポジトリ = 1スタジオ**: モノレポ対応
- **キャラクターPrefab**: Prefab Variant パターン。ベースPrefab に共通コンポーネント、プロファイル別バリアントで SpriteRenderer のみオーバーライド。Lead専用Prefab（デスクなし巡回用）。事前 Prefab 化
- **Issue マーカー「!」**: 未連携Issueはスタジオ内自由浮遊。スタッフ雇用時にブランチ自動作成→Worktree(デスク)→「!」がデスクへ飛行→デスク付近浮遊固定。クラスタリング＋重なり回避
- **Issue-Branch連携**: 現行gwt既存機能。雇用時にIssue番号からブランチ名自動生成、依存チェーン全体を一括実行
- **Lead質問「?」**: Lead付近に浮遊。緩急度色分け（赤/黄/青）。クリックでオーバーレイ回答UI
- **Lead AI**: API直接呼び出し（CLIエージェントではない）。完全自律動作。Agentツールとしてコンテキスト提供
- **PR**: デスクに付随するステータスバッジ（アイコン＋色のみ、PR存在時のみ）
- **空席デスク**: Worktree あり + Agent未起動 = デスクスプライトのみ（キャラクターなし）
- **半透明デスク**: リモートブランチ = デスクスプライトを半透明で描画
- オブジェクト選択は 2D Collider ベースで実装（Physics2D.Raycast）、優先順位: Lead > Developer > デスク > マーカー
- オーバーレイ UI は Canvas (Screen Space - Overlay)。複数パネル同時表示・ドラッグ移動可。エンティティ別独立パネル
- **デスクコンテキストメニュー**: Screen Space 描画、ピクセルアート統一デザイン、画面端自動クランプ
- **デスク配置**: Issue起点（依存チェーン全体を一括実行）、フリー配置（自動＋ドラッグ並替え、グリッドスナップ）
- **ターミナルの区別**: スタッフクリック→ライブセッション（スタッフがいる＝ターミナルが存在する）、コンテキストメニューTerminal→プレーンターミナル（新規PTY、パネル閉じ=PTY終了）
- **Fire Agent**: 状態別段階的確認ダイアログ → プロセス停止 → デスク即座空席化 → ドアへ歩行退場アニメ装飾的並行再生
- **AI要約**: イベント駆動生成＋キャッシュ方式。吹き出しはテンプレート固定短文のみ
- **データ更新**: ハイブリッド（エージェント状態=イベント駆動、Git/PR/Issue=ポーリング30秒）
- **カメラ**: パン＋ズーム操作対応、Pixel Perfect Camera
- **機能アクセス**: ハイブリッド（HUD/サイドバー＋コマンドパレット Cmd+K）
- **ビルド**: Script Backend は IL2CPP を使用（Mono 不可）
- **入力**: New Input System を使用（旧 Input API 不可）
- **GitHub認証**: gh CLI連携（既存のgh auth認証情報を使用）
- **エージェント種別**: Claude Code / Codex / Gemini / GitHub Copilot / ユーザーカスタム（CLIパス＋引数指定）

### フェーズ分割

**Phase S（Setup）:** DI基盤、インターフェース定義、Pixel Perfect Camera、A* Pathfinding Pro導入
**Phase F（Foundation）:** スタジオワールド構築、Tilemap、キャラクターPrefab、移動システム
**Phase U（User Story）:** エンティティ配置、インタラクション、コンテキストメニュー、オーバーレイパネル、マーカー
**Phase FIN（Finalization）:** パフォーマンス最適化、サウンド、受け入れテスト

### リスク

- 多数スプライトのアニメーション負荷 → スプライトアトラス + GPU Instancing で対応
- 2D Collider の選択精度 → コライダー形状の最適化＋ヒット判定優先順位で対応
- Issue マーカーの重なり → 浮遊アニメーション + クラスタリング + 重なり回避ロジックで対応
- 50+デスクのカメラパン/ズーム → カリング＋LOD的な表示最適化で対応
- A* Pathfinding Pro の動的障害物対応 → グラフ再計算頻度の調整で対応
- IL2CPP ビルド時のリフレクション制限 → link.xml で必要な型を保護、VContainer の IL2CPP 互換性を確認
- Tilemap動的拡張のパフォーマンス → 行単位の段階的拡張で対応
- デスクドラッグ時のA* GridGraph再計算コスト → 部分更新で最小化
- Pixel Perfect Camera とズーム操作の整合性 → PPU (Pixels Per Unit) 設定の最適化
