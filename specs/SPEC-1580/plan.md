### Technical context

| ファイル | 変更内容 |
|---------|---------|
| `Assets/Editor/ModernInteriorsSpriteAssetPipeline.cs` | Office root 定数、Office/UI アトラス定義、Office インポーター対応、IsSpriteCandidateAsset 拡張 |
| `Assets/Editor/CharacterAnimationPipeline.cs` | **新規**: AnimatorController + AnimationClip 自動生成 |
| `Assets/Scripts/Gwt/Tests/Editor/ModernInteriorsSpriteAssetPipelineTests.cs` | Office/UI/Character 関連テスト追加 |

### Implementation approach

既存 `ModernInteriorsSpriteAssetPipeline` を拡張して Modern Office と UI 素材に対応。キャラクターアニメーション生成は責務分離のため新規 `CharacterAnimationPipeline` として独立。

### Phasing

1. Step 1-3: インポーター設定拡張 (Office + Interiors + UI)
2. Step 4: キャラクタースプライトシートスライス
3. Step 5: Animator Controller + Animation Clip 生成
4. Step 6: SpriteAtlas 整理 (Office + UI アトラス追加)
