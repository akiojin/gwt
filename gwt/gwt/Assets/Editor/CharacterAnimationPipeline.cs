using System.Collections.Generic;
using System.IO;
using System.Linq;
using UnityEditor;
using UnityEditor.Animations;
using UnityEngine;

namespace Gwt.Editor
{
    public readonly struct AnimationRowDefinition
    {
        public string AnimationName { get; }
        public int Row { get; }
        public int ColumnOffset { get; }
        public int FrameCount { get; }
        public bool Loop { get; }

        public AnimationRowDefinition(string animationName, int row, int columnOffset, int frameCount, bool loop)
        {
            AnimationName = animationName;
            Row = row;
            ColumnOffset = columnOffset;
            FrameCount = frameCount;
            Loop = loop;
        }
    }

    public static class CharacterAnimationPipeline
    {
        public const string PremadeCharactersRoot =
            "Assets/Graphics/moderninteriors-win/2_Characters/Character_Generator/0_Premade_Characters/16x16";

        public const string GeneratedAnimationsRoot = "Assets/Generated/CharacterAnimations";
        public const int CellSize = 16;
        public const int SheetColumns = 56;
        public const int SheetRows = 41;
        public const float DefaultFrameRate = 8f;

        public const int DirectionStride = SheetColumns / 4; // 14 columns per direction

        public static readonly AnimationRowDefinition[] StudioAnimations = new[]
        {
            // idle: row 1 (4 directions concatenated)
            new AnimationRowDefinition("idle_down",  1, DirectionStride * 0, 6, true),
            new AnimationRowDefinition("idle_up",    1, DirectionStride * 1, 6, true),
            new AnimationRowDefinition("idle_left",  1, DirectionStride * 2, 6, true),
            new AnimationRowDefinition("idle_right", 1, DirectionStride * 3, 6, true),
            // walk: row 2 (4 directions concatenated)
            new AnimationRowDefinition("walk_down",  2, DirectionStride * 0, 6, true),
            new AnimationRowDefinition("walk_up",    2, DirectionStride * 1, 6, true),
            new AnimationRowDefinition("walk_left",  2, DirectionStride * 2, 6, true),
            new AnimationRowDefinition("walk_right", 2, DirectionStride * 3, 6, true),
            // sit: row 4 (4 directions concatenated, variant 1)
            new AnimationRowDefinition("sit_down",   4, DirectionStride * 0, 6, true),
            new AnimationRowDefinition("sit_up",     4, DirectionStride * 1, 6, true),
            new AnimationRowDefinition("sit_left",   4, DirectionStride * 2, 6, true),
            new AnimationRowDefinition("sit_right",  4, DirectionStride * 3, 6, true),
        };

        public static List<string> CollectPremadeCharacterPaths()
        {
            if (!AssetDatabase.IsValidFolder(PremadeCharactersRoot))
                return new List<string>();

            return AssetDatabase.FindAssets("t:Texture2D", new[] { PremadeCharactersRoot })
                .Select(AssetDatabase.GUIDToAssetPath)
                .Where(p => p.EndsWith(".png"))
                .OrderBy(p => p)
                .ToList();
        }

        public static AnimationClip CreateAnimationClip(string clipName, Sprite[] frames, float frameRate, bool loop)
        {
            var clip = new AnimationClip();
            clip.name = clipName;
            clip.frameRate = frameRate;

            var keyframes = new ObjectReferenceKeyframe[frames.Length];
            for (int i = 0; i < frames.Length; i++)
            {
                keyframes[i] = new ObjectReferenceKeyframe
                {
                    time = i / frameRate,
                    value = frames[i]
                };
            }

            var binding = EditorCurveBinding.PPtrCurve(string.Empty, typeof(SpriteRenderer), "m_Sprite");
            AnimationUtility.SetObjectReferenceCurve(clip, binding, keyframes);

            var settings = AnimationUtility.GetAnimationClipSettings(clip);
            settings.loopTime = loop;
            AnimationUtility.SetAnimationClipSettings(clip, settings);

            return clip;
        }

        public static Sprite[] GetRowSprites(string spriteSheetPath, int row, int columnOffset, int frameCount)
        {
            var allSprites = AssetDatabase.LoadAllAssetRepresentationsAtPath(spriteSheetPath)
                .OfType<Sprite>()
                .ToArray();

            if (allSprites.Length == 0)
                return new Sprite[0];

            var startIndex = row * SheetColumns + columnOffset;
            var result = new List<Sprite>();
            for (int i = 0; i < frameCount && startIndex + i < allSprites.Length; i++)
            {
                result.Add(allSprites[startIndex + i]);
            }

            return result.ToArray();
        }

        public static AnimatorController CreateCharacterAnimatorController(
            string outputPath, AnimationClip idleClip, AnimationClip walkClip, AnimationClip sitClip)
        {
            var directory = Path.GetDirectoryName(outputPath);
            if (!string.IsNullOrEmpty(directory))
                EnsureFolder(directory);

            var controller = AnimatorController.CreateAnimatorControllerAtPath(outputPath);

            controller.AddParameter("isWalking", AnimatorControllerParameterType.Bool);
            controller.AddParameter("isSitting", AnimatorControllerParameterType.Bool);

            var rootStateMachine = controller.layers[0].stateMachine;

            var idleState = rootStateMachine.AddState("Idle");
            idleState.motion = idleClip;

            var walkState = rootStateMachine.AddState("Walk");
            walkState.motion = walkClip;

            var sitState = rootStateMachine.AddState("Sit");
            sitState.motion = sitClip;

            var toWalk = idleState.AddTransition(walkState);
            toWalk.AddCondition(AnimatorConditionMode.If, 0, "isWalking");
            toWalk.hasExitTime = false;
            toWalk.duration = 0;

            var toIdleFromWalk = walkState.AddTransition(idleState);
            toIdleFromWalk.AddCondition(AnimatorConditionMode.IfNot, 0, "isWalking");
            toIdleFromWalk.hasExitTime = false;
            toIdleFromWalk.duration = 0;

            var toSit = idleState.AddTransition(sitState);
            toSit.AddCondition(AnimatorConditionMode.If, 0, "isSitting");
            toSit.hasExitTime = false;
            toSit.duration = 0;

            var toIdleFromSit = sitState.AddTransition(idleState);
            toIdleFromSit.AddCondition(AnimatorConditionMode.IfNot, 0, "isSitting");
            toIdleFromSit.hasExitTime = false;
            toIdleFromSit.duration = 0;

            rootStateMachine.defaultState = idleState;

            return controller;
        }

        [MenuItem("GWT/Graphics/Generate Character Animations")]
        public static void GenerateAllCharacterAnimations()
        {
            var characterPaths = CollectPremadeCharacterPaths();
            if (characterPaths.Count == 0)
            {
                Debug.LogWarning("[CharacterAnimationPipeline] No premade characters found.");
                return;
            }

            // Ensure character spritesheets are configured as Multiple with grid slicing
            foreach (var sheetPath in characterPaths)
            {
                ModernInteriorsSpriteAssetPipeline.ConfigureSourceSpriteImporter(sheetPath);
            }

            EnsureFolder(GeneratedAnimationsRoot);

            foreach (var sheetPath in characterPaths)
            {
                var characterName = Path.GetFileNameWithoutExtension(sheetPath);
                var characterDir = $"{GeneratedAnimationsRoot}/{characterName}";
                EnsureFolder(characterDir);

                var idleFrames = GetRowSprites(sheetPath, StudioAnimations[0].Row, StudioAnimations[0].ColumnOffset, StudioAnimations[0].FrameCount);
                if (idleFrames.Length == 0)
                {
                    Debug.LogWarning($"[CharacterAnimationPipeline] No sprites found for {characterName}, skipping.");
                    continue;
                }

                foreach (var anim in StudioAnimations)
                {
                    var frames = GetRowSprites(sheetPath, anim.Row, anim.ColumnOffset, anim.FrameCount);
                    if (frames.Length == 0) continue;

                    var clip = CreateAnimationClip(anim.AnimationName, frames, DefaultFrameRate, anim.Loop);
                    var clipPath = $"{characterDir}/{anim.AnimationName}.anim";
                    AssetDatabase.CreateAsset(clip, clipPath);
                }

                var idleClip = AssetDatabase.LoadAssetAtPath<AnimationClip>($"{characterDir}/idle_down.anim");
                var walkClip = AssetDatabase.LoadAssetAtPath<AnimationClip>($"{characterDir}/walk_down.anim");
                var sitClip = AssetDatabase.LoadAssetAtPath<AnimationClip>($"{characterDir}/sit_down.anim");

                if (idleClip != null && walkClip != null && sitClip != null)
                {
                    var controllerPath = $"{characterDir}/{characterName}.controller";
                    CreateCharacterAnimatorController(controllerPath, idleClip, walkClip, sitClip);
                }

                Debug.Log($"[CharacterAnimationPipeline] Generated animations for {characterName}");
            }

            AssetDatabase.SaveAssets();
            AssetDatabase.Refresh();
            Debug.Log("[CharacterAnimationPipeline] All character animations generated.");
        }

        private static void EnsureFolder(string assetPath)
        {
            var segments = assetPath.Split('/');
            var current = segments[0];
            for (var i = 1; i < segments.Length; i++)
            {
                var next = $"{current}/{segments[i]}";
                if (!AssetDatabase.IsValidFolder(next))
                    AssetDatabase.CreateFolder(current, segments[i]);
                current = next;
            }
        }
    }
}
