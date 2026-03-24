### Background

`Assets/Graphics/moderninteriors-win` および `Assets/Graphics/Modern_Office_Revamped_v1.2` 配下に LimeZu アセット（Modern Interiors + Modern Office）が配置されている。現在のパイプライン (`ModernInteriorsSpriteAssetPipeline.cs`) は Home Design のタイル生成と基本インポーター設定のみ対応。キャラクターアニメーション、オフィス家具、UI 素材のパイプラインが未整備。

gwt Unity 6 版のスタジオビジュアル (#1546/#1547) を実現するには、これらのアセットを Unity で正しくインポート・スライス・アニメーション化する必要がある。

### User scenarios

- P0 US1: 開発者が Modern Office 家具スプライト (339点) を Unity で即座に参照・配置できる
- P0 US2: 開発者がキャラクタースプライトシート (20体) を idle/walk/sit アニメーションとして使用できる
- P0 US3: 開発者が UI アイコンシート (288×256) を個別スプライトとして参照できる
- P1 US4: Animator Controller と Animation Clip がキャラクターごとに自動生成される
- P1 US5: Office / UI 用の SpriteAtlas が自動生成される
- P1 US6: Editor メニューから再実行可能で冪等に更新できる

### Functional requirements

- FR-001: `moderninteriors-win` 配下の PNG について cell size を推定し Sprite importer 設定を適用
- FR-002: cell size より大きい PNG は grid slice により Multiple Sprite として設定
- FR-003: character / background 系 SpriteAtlas の生成・更新
- FR-004: 生成処理は Unity Editor 内で再実行可能・冪等
- FR-005: `Modern_Office_Revamped_v1.2` 配下の家具スプライト (32×48 px singles) を Single インポート (pixelsPerUnit=16)
- FR-006: `Modern_Office_16x16.png` (256×848) を 16×16 グリッドスライス
- FR-007: `Room_Builder_Office_16x16.png` を 16×16 グリッドスライス
- FR-008: `UI_16x16.png` (288×256) を 16×16 グリッドスライス
- FR-009: `UI_thinking_emotes_animation_16x16.png` (160×160) を 16×16 グリッドスライス
- FR-010: キャラクタースプライトシート (896×656 @ 16x16) を 16×16 グリッドスライス
- FR-011: キャラクターごとの AnimationClip 生成 (idle, walk, sit, phone)
- FR-012: キャラクターごとの AnimatorController 生成 (idle→walk→sit 状態遷移)
- FR-013: Office / UI 用 SpriteAtlas 生成

### Non-functional requirements

- NFR-001: 元の PNG ファイル内容は変更しない
- NFR-002: 既存の未関連アセット・コードに影響を広げない
- NFR-003: 実装は Editor スクリプトと Editor テスト中心に閉じる
- NFR-004: 16x16 ベース解像度のみ対応 (Spec #1539 準拠)

### Success criteria

- SC-001: Modern Office 家具 339 点が Sprite として利用可能
- SC-002: キャラクター 20 体のアニメーション (idle/walk/sit) が再生可能
- SC-003: UI アイコンが個別スプライトとして参照可能
- SC-004: Office / UI 用 SpriteAtlas が生成される
- SC-005: Editor テストと Unity compile check が全パス
