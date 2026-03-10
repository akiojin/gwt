using System.IO;
using System.Linq;
using Gwt.Editor;
using NUnit.Framework;
using UnityEditor;
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

        private static string CreateTextureAsset(string fileName, int width, int height)
        {
            EnsureFolder(TempRoot);

            var assetPath = $"{TempRoot}/{fileName}";
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
}
