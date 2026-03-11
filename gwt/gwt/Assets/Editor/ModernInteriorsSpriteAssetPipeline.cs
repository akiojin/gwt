using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using UnityEditor;
using UnityEditor.U2D;
using UnityEngine;
using UnityEngine.Tilemaps;
using UnityEngine.U2D;

namespace Gwt.Editor
{
    public static class ModernInteriorsSpriteAssetPipeline
    {
        public const string GraphicsRoot = "Assets/Graphics/moderninteriors-win";
        public const string HomeDesignsRoot = GraphicsRoot + "/6_Home_Designs";
        public const string GeneratedSpriteRoot = "Assets/Generated/ModernInteriorsTilemapSprites";
        public const string GeneratedTileRoot = "Assets/Generated/ModernInteriorsTilemapTiles";
        public const string GeneratedAtlasPath = "Assets/Generated/ModernInteriorsAtlases/HomeDesigns.spriteatlas";
        public const string GeneratedCharacterAtlasPath = "Assets/Generated/ModernInteriorsAtlases/Characters.spriteatlas";
        public const string GeneratedBackgroundAtlasPath = "Assets/Generated/ModernInteriorsAtlases/Backgrounds.spriteatlas";

        private static readonly Regex CellSizePattern = new(@"(?<!\d)(16|32|48)x\1(?!\d)", RegexOptions.Compiled | RegexOptions.IgnoreCase);

        [MenuItem("GWT/Graphics/Generate Home Design Tilemap Assets")]
        public static void GenerateHomeDesignTilemapAssets()
        {
            var sheetPaths = CollectHomeDesignLayerSheetPaths();
            var generatedSprites = 0;
            var generatedTiles = 0;

            foreach (var assetPath in sheetPaths)
            {
                generatedSprites += ExportTileSpritesForSheet(assetPath, GeneratedSpriteRoot).Count;
                generatedTiles += CreateOrUpdateTilesForSheet(assetPath, GeneratedSpriteRoot, GeneratedTileRoot);
            }

            AssetDatabase.SaveAssets();
            AssetDatabase.Refresh();

            Debug.Log($"[GWT] Generated {generatedSprites} tile sprites and {generatedTiles} tile assets from {sheetPaths.Count} home design sheets");
        }

        [MenuItem("GWT/Graphics/Generate Sprite Importers And Atlases")]
        public static void GenerateSpriteImportersAndAtlases()
        {
            var spritePaths = CollectSpritePngPaths();
            foreach (var assetPath in spritePaths)
            {
                ConfigureSourceSpriteImporter(assetPath);
            }

            CreateOrUpdateAtlas(GetCharacterAtlasDefinition());
            CreateOrUpdateAtlas(GetBackgroundAtlasDefinition());

            AssetDatabase.SaveAssets();
            AssetDatabase.Refresh();

            Debug.Log($"[GWT] Configured {spritePaths.Count} sprite importers and updated character/background atlases");
        }

        public static List<string> CollectSpritePngPaths()
        {
            return AssetDatabase.FindAssets("t:Texture2D", new[] { GraphicsRoot })
                .Select(AssetDatabase.GUIDToAssetPath)
                .Where(IsSpriteCandidateAsset)
                .OrderBy(path => path, StringComparer.Ordinal)
                .ToList();
        }

        public static List<string> CollectHomeDesignLayerSheetPaths()
        {
            return AssetDatabase.FindAssets("t:Texture2D", new[] { HomeDesignsRoot })
                .Select(AssetDatabase.GUIDToAssetPath)
                .Where(IsHomeDesignLayerSheet)
                .OrderBy(path => path, StringComparer.Ordinal)
                .ToList();
        }

        public static bool IsHomeDesignLayerSheet(string assetPath)
        {
            if (string.IsNullOrWhiteSpace(assetPath) ||
                !assetPath.StartsWith(HomeDesignsRoot, StringComparison.Ordinal) ||
                !assetPath.EndsWith(".png", StringComparison.OrdinalIgnoreCase))
            {
                return false;
            }

            var fileName = Path.GetFileNameWithoutExtension(assetPath);
            if (fileName.IndexOf("preview", StringComparison.OrdinalIgnoreCase) >= 0)
                return false;

            return fileName.IndexOf("layer", StringComparison.OrdinalIgnoreCase) >= 0 &&
                   InferCellSize(assetPath).HasValue;
        }

        public static bool IsSpriteCandidateAsset(string assetPath)
        {
            if (string.IsNullOrWhiteSpace(assetPath) ||
                !assetPath.StartsWith(GraphicsRoot, StringComparison.Ordinal) ||
                !assetPath.EndsWith(".png", StringComparison.OrdinalIgnoreCase))
            {
                return false;
            }

            var fileName = Path.GetFileNameWithoutExtension(assetPath);
            if (fileName.IndexOf("preview", StringComparison.OrdinalIgnoreCase) >= 0)
                return false;

            return InferCellSize(assetPath).HasValue;
        }

        public static int? InferCellSize(string assetPath)
        {
            if (string.IsNullOrWhiteSpace(assetPath))
                return null;

            var match = CellSizePattern.Match(assetPath);
            if (!match.Success)
                return null;

            return int.Parse(match.Groups[1].Value);
        }

        public static bool ShouldSliceAsMultiple(int textureWidth, int textureHeight, int cellSize)
        {
            if (textureWidth <= 0) throw new ArgumentOutOfRangeException(nameof(textureWidth));
            if (textureHeight <= 0) throw new ArgumentOutOfRangeException(nameof(textureHeight));
            if (cellSize <= 0) throw new ArgumentOutOfRangeException(nameof(cellSize));

            return (textureWidth > cellSize || textureHeight > cellSize) &&
                   textureWidth % cellSize == 0 &&
                   textureHeight % cellSize == 0;
        }

        public static List<TileSlice> BuildTileSlices(string spriteNamePrefix, int textureWidth, int textureHeight, int cellSize)
        {
            if (textureWidth <= 0) throw new ArgumentOutOfRangeException(nameof(textureWidth));
            if (textureHeight <= 0) throw new ArgumentOutOfRangeException(nameof(textureHeight));
            if (cellSize <= 0) throw new ArgumentOutOfRangeException(nameof(cellSize));

            var columns = textureWidth / cellSize;
            var rows = textureHeight / cellSize;
            var slices = new List<TileSlice>(columns * rows);

            for (var row = 0; row < rows; row++)
            {
                for (var column = 0; column < columns; column++)
                {
                    slices.Add(new TileSlice(
                        $"{spriteNamePrefix}_{row:00}_{column:00}",
                        new RectInt(column * cellSize, textureHeight - ((row + 1) * cellSize), cellSize, cellSize)));
                }
            }

            return slices;
        }

        public static List<string> ExportTileSpritesForSheet(string sheetAssetPath, string outputRoot)
        {
            var cellSize = InferCellSize(sheetAssetPath);
            if (!cellSize.HasValue)
                return new List<string>();

            var sourceTexture = LoadPngFromProject(sheetAssetPath);
            var slices = BuildTileSlices(Path.GetFileNameWithoutExtension(sheetAssetPath), sourceTexture.width, sourceTexture.height, cellSize.Value);
            var outputFolder = BuildGeneratedSpriteFolder(sheetAssetPath, outputRoot);
            EnsureFolderTree(outputFolder);

            var generatedAssetPaths = new List<string>(slices.Count);
            foreach (var slice in slices)
            {
                var assetPath = $"{outputFolder}/{slice.Name}.png";
                SaveSlicePng(sourceTexture, slice.Rect, assetPath);
                generatedAssetPaths.Add(assetPath);
            }

            AssetDatabase.Refresh(ImportAssetOptions.ForceUpdate);

            foreach (var assetPath in generatedAssetPaths)
            {
                ConfigureGeneratedTileSpriteImporter(assetPath);
            }

            AssetDatabase.Refresh(ImportAssetOptions.ForceUpdate);
            UnityEngine.Object.DestroyImmediate(sourceTexture);
            return generatedAssetPaths;
        }

        public static int CreateOrUpdateTilesForSheet(string sheetAssetPath, string generatedSpriteRoot, string tileOutputRoot)
        {
            var spriteAssetPaths = ExportTileSpritesForSheet(sheetAssetPath, generatedSpriteRoot);
            if (spriteAssetPaths.Count == 0)
                return 0;

            var outputFolder = BuildTileOutputFolder(sheetAssetPath, tileOutputRoot);
            EnsureFolderTree(outputFolder);

            foreach (var spriteAssetPath in spriteAssetPaths)
            {
                var sprite = AssetDatabase.LoadAssetAtPath<Sprite>(spriteAssetPath);
                if (sprite == null)
                    continue;

                var tilePath = $"{outputFolder}/{Path.GetFileNameWithoutExtension(spriteAssetPath)}.asset";
                var tile = AssetDatabase.LoadAssetAtPath<Tile>(tilePath);
                if (tile == null)
                {
                    tile = ScriptableObject.CreateInstance<Tile>();
                    tile.name = sprite.name;
                    tile.sprite = sprite;
                    AssetDatabase.CreateAsset(tile, tilePath);
                }
                else
                {
                    tile.name = sprite.name;
                    tile.sprite = sprite;
                    EditorUtility.SetDirty(tile);
                }
            }

            AssetDatabase.SaveAssets();
            AssetDatabase.Refresh(ImportAssetOptions.ForceUpdate);
            return spriteAssetPaths.Count;
        }

        public static void ConfigureSourceSpriteImporter(string assetPath)
        {
            var cellSize = InferCellSize(assetPath);
            if (!cellSize.HasValue)
                return;

            var importer = AssetImporter.GetAtPath(assetPath) as TextureImporter;
            var texture = AssetDatabase.LoadAssetAtPath<Texture2D>(assetPath);
            if (importer == null || texture == null)
                return;

            importer.textureType = TextureImporterType.Sprite;
            importer.alphaIsTransparency = true;
            importer.filterMode = FilterMode.Point;
            importer.mipmapEnabled = false;
            importer.spritePixelsPerUnit = cellSize.Value;

            if (ShouldSliceAsMultiple(texture.width, texture.height, cellSize.Value))
            {
                importer.spriteImportMode = SpriteImportMode.Multiple;
                importer.spritesheet = BuildTileSlices(
                        Path.GetFileNameWithoutExtension(assetPath),
                        texture.width,
                        texture.height,
                        cellSize.Value)
                    .Select(slice => new SpriteMetaData
                    {
                        name = slice.Name,
                        rect = new Rect(slice.Rect.x, slice.Rect.y, slice.Rect.width, slice.Rect.height),
                        alignment = (int)SpriteAlignment.Center,
                        pivot = new Vector2(0.5f, 0.5f)
                    })
                    .ToArray();
            }
            else
            {
                importer.spriteImportMode = SpriteImportMode.Single;
                importer.spritesheet = Array.Empty<SpriteMetaData>();
            }

            importer.SaveAndReimport();
        }

        public static void CreateOrUpdateAtlas(SpriteAtlasDefinition definition)
        {
            if (definition == null) throw new ArgumentNullException(nameof(definition));

            var atlasDirectory = NormalizeAssetPath(Path.GetDirectoryName(definition.OutputPath));
            EnsureFolderTree(atlasDirectory);

            if (AssetDatabase.LoadAssetAtPath<SpriteAtlas>(definition.OutputPath) != null)
                AssetDatabase.DeleteAsset(definition.OutputPath);

            var atlas = new SpriteAtlas();
            atlas.SetPackingSettings(new SpriteAtlasPackingSettings
            {
                enableRotation = false,
                enableTightPacking = false,
                padding = 2,
                blockOffset = 1
            });
            atlas.SetTextureSettings(new SpriteAtlasTextureSettings
            {
                generateMipMaps = false,
                readable = false,
                sRGB = true,
                filterMode = FilterMode.Point
            });

            AssetDatabase.CreateAsset(atlas, definition.OutputPath);

            var packables = definition.PackableFolderPaths
                .Select(path => AssetDatabase.LoadAssetAtPath<UnityEngine.Object>(path))
                .Where(asset => asset != null)
                .ToArray();
            if (packables.Length > 0)
                SpriteAtlasExtensions.Add(atlas, packables);

            EditorUtility.SetDirty(atlas);
            AssetDatabase.SaveAssets();
            AssetDatabase.ImportAsset(definition.OutputPath, ImportAssetOptions.ForceUpdate);
        }

        public static SpriteAtlasDefinition GetHomeDesignAtlasDefinition()
        {
            return new SpriteAtlasDefinition(
                "HomeDesigns",
                GeneratedAtlasPath,
                new[] { GeneratedSpriteRoot });
        }

        public static SpriteAtlasDefinition GetCharacterAtlasDefinition()
        {
            return new SpriteAtlasDefinition(
                "Characters",
                GeneratedCharacterAtlasPath,
                new[] { GraphicsRoot + "/2_Characters" });
        }

        public static SpriteAtlasDefinition GetBackgroundAtlasDefinition()
        {
            return new SpriteAtlasDefinition(
                "Backgrounds",
                GeneratedBackgroundAtlasPath,
                new[]
                {
                    GraphicsRoot + "/1_Interiors",
                    GraphicsRoot + "/3_Animated_objects",
                    GraphicsRoot + "/6_Home_Designs"
                });
        }

        private static void ConfigureGeneratedTileSpriteImporter(string assetPath)
        {
            var importer = AssetImporter.GetAtPath(assetPath) as TextureImporter;
            if (importer == null)
                return;

            importer.textureType = TextureImporterType.Sprite;
            importer.spriteImportMode = SpriteImportMode.Single;
            importer.alphaIsTransparency = true;
            importer.filterMode = FilterMode.Point;
            importer.mipmapEnabled = false;
            importer.SaveAndReimport();
        }

        private static Texture2D LoadPngFromProject(string assetPath)
        {
            var absolutePath = ToAbsoluteProjectPath(assetPath);
            var bytes = File.ReadAllBytes(absolutePath);
            var texture = new Texture2D(2, 2, TextureFormat.RGBA32, false);
            texture.LoadImage(bytes);
            return texture;
        }

        private static void SaveSlicePng(Texture2D sourceTexture, RectInt rect, string destinationAssetPath)
        {
            var destinationTexture = new Texture2D(rect.width, rect.height, TextureFormat.RGBA32, false);
            destinationTexture.SetPixels(sourceTexture.GetPixels(rect.x, rect.y, rect.width, rect.height));
            destinationTexture.Apply();

            var absolutePath = ToAbsoluteProjectPath(destinationAssetPath);
            var directory = Path.GetDirectoryName(absolutePath);
            if (!string.IsNullOrEmpty(directory))
                Directory.CreateDirectory(directory);

            File.WriteAllBytes(absolutePath, destinationTexture.EncodeToPNG());
            UnityEngine.Object.DestroyImmediate(destinationTexture);
        }

        private static string BuildGeneratedSpriteFolder(string sheetAssetPath, string outputRoot)
        {
            var normalizedOutputRoot = NormalizeAssetPath(outputRoot);
            var sheetName = Path.GetFileNameWithoutExtension(sheetAssetPath);
            var parentFolder = NormalizeAssetPath(Path.GetDirectoryName(sheetAssetPath));

            if (!string.IsNullOrEmpty(parentFolder) &&
                parentFolder.StartsWith(HomeDesignsRoot + "/", StringComparison.Ordinal))
            {
                var relativeParent = parentFolder.Substring(HomeDesignsRoot.Length + 1);
                return $"{normalizedOutputRoot}/{relativeParent}/{sheetName}";
            }

            return $"{normalizedOutputRoot}/{sheetName}";
        }

        private static string BuildTileOutputFolder(string sheetAssetPath, string outputRoot)
        {
            return BuildGeneratedSpriteFolder(sheetAssetPath, outputRoot);
        }

        private static string ToAbsoluteProjectPath(string assetPath)
        {
            var relativePath = assetPath.Substring("Assets/".Length).Replace('/', Path.DirectorySeparatorChar);
            return Path.Combine(Application.dataPath, relativePath);
        }

        private static void EnsureFolderTree(string assetPath)
        {
            if (string.IsNullOrWhiteSpace(assetPath))
                return;

            var normalizedPath = NormalizeAssetPath(assetPath);
            if (AssetDatabase.IsValidFolder(normalizedPath))
                return;

            var segments = normalizedPath.Split('/');
            var current = segments[0];
            for (var i = 1; i < segments.Length; i++)
            {
                var next = $"{current}/{segments[i]}";
                if (!AssetDatabase.IsValidFolder(next))
                    AssetDatabase.CreateFolder(current, segments[i]);

                current = next;
            }
        }

        private static string NormalizeAssetPath(string assetPath)
        {
            return string.IsNullOrWhiteSpace(assetPath)
                ? string.Empty
                : assetPath.Replace('\\', '/');
        }

        public sealed class SpriteAtlasDefinition
        {
            public SpriteAtlasDefinition(string name, string outputPath, IEnumerable<string> packableFolderPaths)
            {
                Name = name;
                OutputPath = NormalizeAssetPath(outputPath);
                PackableFolderPaths = packableFolderPaths?
                    .Select(NormalizeAssetPath)
                    .Where(path => !string.IsNullOrWhiteSpace(path))
                    .ToArray() ?? Array.Empty<string>();
            }

            public string Name { get; }
            public string OutputPath { get; }
            public IReadOnlyList<string> PackableFolderPaths { get; }
        }

        public readonly struct TileSlice
        {
            public TileSlice(string name, RectInt rect)
            {
                Name = name;
                Rect = rect;
            }

            public string Name { get; }
            public RectInt Rect { get; }
        }
    }
}
