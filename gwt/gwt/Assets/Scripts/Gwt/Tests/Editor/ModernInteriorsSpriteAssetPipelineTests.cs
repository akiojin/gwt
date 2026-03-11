using System.IO;
using System.Linq;
using Gwt.Editor;
using NUnit.Framework;
using UnityEditor;
using UnityEditor.Animations;
using UnityEngine;
using UnityEngine.Tilemaps;
using UnityEngine.U2D;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class ModernInteriorsSpriteAssetPipelineTests
    {
        private const string TempRoot = "Assets/__CodexTemp/ModernInteriorsSpriteAssetPipelineTests";
        private const string GeneratedSpriteRoot = TempRoot + "/GeneratedSprites";
        private const string GeneratedTileRoot = TempRoot + "/GeneratedTiles";

        [TearDown]
        public void TearDown()
        {
            if (AssetDatabase.IsValidFolder(TempRoot))
            {
                AssetDatabase.DeleteAsset(TempRoot);
                AssetDatabase.Refresh();
            }
        }

        [Test]
        public void InferCellSize_UsesPathAndFileNameHints()
        {
            Assert.AreEqual(32,
                ModernInteriorsSpriteAssetPipeline.InferCellSize(
                    "Assets/Graphics/moderninteriors-win/4_User_Interface_Elements/UI_32x32.png"));
            Assert.AreEqual(48,
                ModernInteriorsSpriteAssetPipeline.InferCellSize(
                    "Assets/Graphics/moderninteriors-win/6_Home_Designs/TV_Studio_Designs/48x48/Tv_Studio_Design_layer_1_48x48.png"));
            Assert.IsNull(
                ModernInteriorsSpriteAssetPipeline.InferCellSize(
                    "Assets/Graphics/moderninteriors-win/READ_ME.txt"));
        }

        [Test]
        public void ShouldSliceAsMultiple_DistinguishesSingleAndSheet()
        {
            Assert.IsFalse(ModernInteriorsSpriteAssetPipeline.ShouldSliceAsMultiple(16, 16, 16));
            Assert.IsTrue(ModernInteriorsSpriteAssetPipeline.ShouldSliceAsMultiple(32, 16, 16));
            Assert.IsTrue(ModernInteriorsSpriteAssetPipeline.ShouldSliceAsMultiple(48, 48, 16));
        }

        [Test]
        public void IsLikelySheetAsset_UsesPathAndFileNameHints()
        {
            Assert.IsTrue(ModernInteriorsSpriteAssetPipeline.IsLikelySheetAsset(
                "Assets/Graphics/moderninteriors-win/6_Home_Designs/TV_Studio_Designs/48x48/Tv_Studio_Design_layer_1_48x48.png"));
            Assert.IsTrue(ModernInteriorsSpriteAssetPipeline.IsLikelySheetAsset(
                "Assets/Graphics/moderninteriors-win/4_User_Interface_Elements/UI_32x32.png"));
            Assert.IsFalse(ModernInteriorsSpriteAssetPipeline.IsLikelySheetAsset(
                "Assets/Graphics/moderninteriors-win/1_Interiors/16x16/Theme_Sorter_Shadowless_Singles/Bedroom_Singles_Shadowless_45.png"));
        }

        [Test]
        public void BuildTileSlices_CreatesExpectedCells()
        {
            var slices = ModernInteriorsSpriteAssetPipeline.BuildTileSlices("TestSheet", 64, 32, 16);

            Assert.AreEqual(8, slices.Count);
            Assert.AreEqual(new RectInt(0, 16, 16, 16), slices[0].Rect);
            Assert.AreEqual("TestSheet_00_00", slices[0].Name);
            Assert.AreEqual(new RectInt(48, 0, 16, 16), slices[7].Rect);
            Assert.AreEqual("TestSheet_01_03", slices[7].Name);
        }

        [Test]
        public void ConfigureSourceSpriteImporter_MultipleSheet_CreatesSubSprites()
        {
            var assetPath = CreateTextureAsset("Office_layer_16x16.png", 32, 16);

            ModernInteriorsSpriteAssetPipeline.ConfigureSourceSpriteImporter(assetPath);

            var importer = AssetImporter.GetAtPath(assetPath) as TextureImporter;
            Assert.IsNotNull(importer);
            Assert.AreEqual(SpriteImportMode.Multiple, importer.spriteImportMode);

            var sprites = AssetDatabase.LoadAllAssetRepresentationsAtPath(assetPath).OfType<Sprite>().ToArray();
            Assert.AreEqual(2, sprites.Length);
        }

        [Test]
        public void ConfigureSourceSpriteImporter_SingleCellTexture_UsesSingleMode()
        {
            var assetPath = CreateTextureAsset("Chair_16x16.png", 16, 16);

            ModernInteriorsSpriteAssetPipeline.ConfigureSourceSpriteImporter(assetPath);

            var importer = AssetImporter.GetAtPath(assetPath) as TextureImporter;
            Assert.IsNotNull(importer);
            Assert.AreEqual(SpriteImportMode.Single, importer.spriteImportMode);
            Assert.IsNotNull(AssetDatabase.LoadAssetAtPath<Sprite>(assetPath));
        }

        [Test]
        public void ConfigureSourceSpriteImporter_SinglesFolder_StaysSingleMode()
        {
            var assetPath = CreateTextureAsset("Bedroom_Singles_Shadowless_45_16x16.png", 48, 48, "Singles");

            ModernInteriorsSpriteAssetPipeline.ConfigureSourceSpriteImporter(assetPath);

            var importer = AssetImporter.GetAtPath(assetPath) as TextureImporter;
            Assert.IsNotNull(importer);
            Assert.AreEqual(SpriteImportMode.Single, importer.spriteImportMode);
        }

        [Test]
        public void ExportTileSpritesForSheet_CreatesSpriteAssets()
        {
            var assetPath = CreateTextureAsset("Office_layer_16x16.png", 32, 16);

            var generated = ModernInteriorsSpriteAssetPipeline.ExportTileSpritesForSheet(assetPath, GeneratedSpriteRoot);

            Assert.AreEqual(2, generated.Count);
            var sprite = AssetDatabase.LoadAssetAtPath<Sprite>(generated[0]);
            Assert.IsNotNull(sprite);
        }

        [Test]
        public void CreateOrUpdateTilesForSheet_CreatesTileAssets()
        {
            var assetPath = CreateTextureAsset("Office_layer_16x16.png", 32, 16);

            var createdCount = ModernInteriorsSpriteAssetPipeline.CreateOrUpdateTilesForSheet(
                assetPath,
                GeneratedSpriteRoot,
                GeneratedTileRoot);

            Assert.AreEqual(2, createdCount);

            var tile = AssetDatabase.LoadAssetAtPath<Tile>(
                $"{GeneratedTileRoot}/Office_layer_16x16/Office_layer_16x16_00_00.asset");
            Assert.IsNotNull(tile);
            Assert.IsNotNull(tile.sprite);
        }

        [Test]
        public void CreateOrUpdateAtlas_CreatesAtlasAssetWithPackables()
        {
            var assetPath = CreateTextureAsset("Office_layer_16x16.png", 32, 16);
            ModernInteriorsSpriteAssetPipeline.ExportTileSpritesForSheet(assetPath, GeneratedSpriteRoot);

            var atlasPath = $"{TempRoot}/Atlases/TestAtlas.spriteatlas";
            var definition = new ModernInteriorsSpriteAssetPipeline.SpriteAtlasDefinition(
                "TestAtlas",
                atlasPath,
                new[] { GeneratedSpriteRoot });

            ModernInteriorsSpriteAssetPipeline.CreateOrUpdateAtlas(definition);

            var atlas = AssetDatabase.LoadAssetAtPath<SpriteAtlas>(atlasPath);
            Assert.IsNotNull(atlas);
        }

        [Test]
        public void AtlasDefinitions_UseExpectedRoots()
        {
            var character = ModernInteriorsSpriteAssetPipeline.GetCharacterAtlasDefinition();
            var background = ModernInteriorsSpriteAssetPipeline.GetBackgroundAtlasDefinition();

            Assert.AreEqual("Assets/Generated/ModernInteriorsAtlases/Characters.spriteatlas", character.OutputPath);
            CollectionAssert.AreEqual(new[] { "Assets/Graphics/moderninteriors-win/2_Characters" }, character.PackableFolderPaths);

            Assert.AreEqual("Assets/Generated/ModernInteriorsAtlases/Backgrounds.spriteatlas", background.OutputPath);
            CollectionAssert.AreEqual(
                new[]
                {
                    "Assets/Graphics/moderninteriors-win/1_Interiors",
                    "Assets/Graphics/moderninteriors-win/3_Animated_objects",
                    "Assets/Graphics/moderninteriors-win/6_Home_Designs"
                },
                background.PackableFolderPaths);
        }

        [Test]
        public void IsSpriteCandidateAsset_RecognizesOfficePath()
        {
            Assert.IsTrue(ModernInteriorsSpriteAssetPipeline.IsSpriteCandidateAsset(
                "Assets/Graphics/Modern_Office_Revamped_v1.2/4_Modern_Office_singles/16x16/Modern_Office_Singles_1.png"));
            Assert.IsTrue(ModernInteriorsSpriteAssetPipeline.IsSpriteCandidateAsset(
                "Assets/Graphics/Modern_Office_Revamped_v1.2/Modern_Office_16x16.png"));
        }

        [Test]
        public void InferCellSize_RecognizesOfficePath()
        {
            Assert.AreEqual(16,
                ModernInteriorsSpriteAssetPipeline.InferCellSize(
                    "Assets/Graphics/Modern_Office_Revamped_v1.2/4_Modern_Office_singles/16x16/Modern_Office_Singles_1.png"));
            Assert.AreEqual(16,
                ModernInteriorsSpriteAssetPipeline.InferCellSize(
                    "Assets/Graphics/Modern_Office_Revamped_v1.2/Modern_Office_16x16.png"));
        }

        [Test]
        public void IsLikelySheetAsset_OfficeSinglesReturnsFalse()
        {
            Assert.IsFalse(ModernInteriorsSpriteAssetPipeline.IsLikelySheetAsset(
                "Assets/Graphics/Modern_Office_Revamped_v1.2/4_Modern_Office_singles/16x16/Modern_Office_Singles_1.png"));
        }

        [Test]
        public void IsLikelySheetAsset_OfficeTilesheetReturnsTrue()
        {
            Assert.IsTrue(ModernInteriorsSpriteAssetPipeline.IsLikelySheetAsset(
                "Assets/Graphics/Modern_Office_Revamped_v1.2/Modern_Office_16x16.png"));
            Assert.IsTrue(ModernInteriorsSpriteAssetPipeline.IsLikelySheetAsset(
                "Assets/Graphics/Modern_Office_Revamped_v1.2/1_Room_Builder_Office/Room_Builder_Office_16x16.png"));
        }

        [Test]
        public void OfficeAtlasDefinition_UsesExpectedRoots()
        {
            var office = ModernInteriorsSpriteAssetPipeline.GetOfficeAtlasDefinition();
            Assert.AreEqual("Assets/Generated/ModernInteriorsAtlases/Office.spriteatlas", office.OutputPath);
            CollectionAssert.AreEqual(
                new[] { "Assets/Graphics/Modern_Office_Revamped_v1.2" },
                office.PackableFolderPaths);
        }

        [Test]
        public void UiAtlasDefinition_UsesExpectedRoots()
        {
            var ui = ModernInteriorsSpriteAssetPipeline.GetUiAtlasDefinition();
            Assert.AreEqual("Assets/Generated/ModernInteriorsAtlases/UI.spriteatlas", ui.OutputPath);
            CollectionAssert.AreEqual(
                new[] { "Assets/Graphics/moderninteriors-win/4_User_Interface_Elements" },
                ui.PackableFolderPaths);
        }

        private static string CreateTextureAsset(string fileName, int width, int height, string subFolder = null)
        {
            EnsureFolder(TempRoot);

            var assetPath = string.IsNullOrWhiteSpace(subFolder)
                ? $"{TempRoot}/{fileName}"
                : $"{TempRoot}/{subFolder}/{fileName}";
            if (!string.IsNullOrWhiteSpace(subFolder))
                EnsureFolder($"{TempRoot}/{subFolder}");
            var relativePath = assetPath.Substring("Assets/".Length)
                .Replace('/', Path.DirectorySeparatorChar);
            var fullPath = Path.Combine(Application.dataPath, relativePath);
            var directory = Path.GetDirectoryName(fullPath);
            if (!string.IsNullOrEmpty(directory))
                Directory.CreateDirectory(directory);

            var texture = new Texture2D(width, height, TextureFormat.RGBA32, false);
            var pixels = Enumerable.Repeat(new Color(1f, 1f, 1f, 1f), width * height).ToArray();
            texture.SetPixels(pixels);
            texture.Apply();

            File.WriteAllBytes(fullPath, texture.EncodeToPNG());
            Object.DestroyImmediate(texture);

            AssetDatabase.ImportAsset(assetPath, ImportAssetOptions.ForceUpdate);
            return assetPath;
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

    [TestFixture]
    public class CharacterAnimationPipelineTests
    {
        private const string TempRoot = "Assets/__CodexTemp/CharacterAnimationPipelineTests";

        [TearDown]
        public void TearDown()
        {
            if (AssetDatabase.IsValidFolder(TempRoot))
            {
                AssetDatabase.DeleteAsset(TempRoot);
                AssetDatabase.Refresh();
            }
        }

        [Test]
        public void CharacterAnimationPipeline_StudioAnimations_HasExpectedDefinitions()
        {
            var animations = CharacterAnimationPipeline.StudioAnimations;
            Assert.IsTrue(animations.Length >= 12, "Should have at least 12 animations (idle/walk/sit x 4 directions)");

            Assert.AreEqual("idle_down", animations[0].AnimationName);
            Assert.AreEqual(1, animations[0].Row);
            Assert.AreEqual(0, animations[0].ColumnOffset);
            Assert.IsTrue(animations[0].Loop);
        }

        [Test]
        public void CharacterAnimationPipeline_Constants_MatchSpriteSheetDimensions()
        {
            Assert.AreEqual(16, CharacterAnimationPipeline.CellSize);
            Assert.AreEqual(56, CharacterAnimationPipeline.SheetColumns);
            Assert.AreEqual(41, CharacterAnimationPipeline.SheetRows);
        }

        [Test]
        public void CharacterAnimationPipeline_CreateAnimationClip_SetsFramesAndLoop()
        {
            var texture = new Texture2D(48, 16, TextureFormat.RGBA32, false);
            var sprites = new[]
            {
                Sprite.Create(texture, new Rect(0, 0, 16, 16), Vector2.one * 0.5f),
                Sprite.Create(texture, new Rect(16, 0, 16, 16), Vector2.one * 0.5f),
                Sprite.Create(texture, new Rect(32, 0, 16, 16), Vector2.one * 0.5f)
            };

            var clip = CharacterAnimationPipeline.CreateAnimationClip("test_idle", sprites, 8f, true);

            Assert.IsNotNull(clip);
            Assert.AreEqual("test_idle", clip.name);
            Assert.AreEqual(8f, clip.frameRate);

            var settings = AnimationUtility.GetAnimationClipSettings(clip);
            Assert.IsTrue(settings.loopTime);

            Object.DestroyImmediate(texture);
            foreach (var s in sprites) Object.DestroyImmediate(s);
            Object.DestroyImmediate(clip);
        }

        [Test]
        public void CharacterAnimationPipeline_CreateAnimatorController_HasExpectedStates()
        {
            var tempPath = $"{TempRoot}/TestAnimator.controller";
            EnsureFolder(TempRoot);

            var texture = new Texture2D(16, 16, TextureFormat.RGBA32, false);
            var sprite = Sprite.Create(texture, new Rect(0, 0, 16, 16), Vector2.one * 0.5f);
            var dummyClip = CharacterAnimationPipeline.CreateAnimationClip("dummy", new[] { sprite }, 8f, true);
            AssetDatabase.CreateAsset(dummyClip, $"{TempRoot}/dummy.anim");

            var controller = CharacterAnimationPipeline.CreateCharacterAnimatorController(
                tempPath, dummyClip, dummyClip, dummyClip);

            Assert.IsNotNull(controller);
            Assert.AreEqual(2, controller.parameters.Length);

            var stateMachine = controller.layers[0].stateMachine;
            Assert.AreEqual(3, stateMachine.states.Length);

            Object.DestroyImmediate(texture);
            Object.DestroyImmediate(sprite);
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
