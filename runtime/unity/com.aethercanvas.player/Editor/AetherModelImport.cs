using System.IO;
using System.Linq;
using UnityEditor;
using UnityEngine;

namespace AetherCanvas.Editor
{
    /// <summary>
    /// Imports the texture pages of an exported model (PNGs next to a
    /// <c>model.json</c>) the way the runtime needs them: full size,
    /// uncompressed, without mipmaps and with raw colour, so Unity draws
    /// exactly what the editor showed. Also adds
    /// <c>GameObject ▸ Aether Canvas ▸ Model from Selected model.json</c>.
    /// </summary>
    sealed class AetherModelImport : AssetPostprocessor
    {
        void OnPreprocessTexture()
        {
            if (!IsModelPage(assetPath)) return;
            var importer = (TextureImporter)assetImporter;
            importer.textureType = TextureImporterType.Default;
            importer.sRGBTexture = false;
            importer.mipmapEnabled = false;
            importer.npotScale = TextureImporterNPOTScale.None;
            importer.textureCompression = TextureImporterCompression.Uncompressed;
            importer.alphaIsTransparency = false;
            importer.wrapMode = TextureWrapMode.Clamp;
            importer.filterMode = FilterMode.Bilinear;
            importer.maxTextureSize = 8192;
        }

        static bool IsModelPage(string path)
        {
            if (!path.EndsWith(".png", System.StringComparison.OrdinalIgnoreCase)) return false;
            var json = Path.Combine(Path.GetDirectoryName(path) ?? "", "model.json");
            try
            {
                return File.Exists(json) && File.ReadAllText(json).Contains("\"aether-model\"");
            }
            catch (IOException)
            {
                return false;
            }
        }

        [MenuItem("GameObject/Aether Canvas/Model from Selected model.json", false, 10)]
        static void CreateFromSelection(MenuCommand command)
        {
            var json = Selection.activeObject as TextAsset;
            var path = AssetDatabase.GetAssetPath(json);
            var dir = Path.GetDirectoryName(path)?.Replace('\\', '/') ?? "Assets";
            var pages = AssetDatabase.FindAssets("t:Texture2D", new[] { dir })
                .Select(AssetDatabase.GUIDToAssetPath)
                .Where(p => Path.GetDirectoryName(p)?.Replace('\\', '/') == dir)
                .OrderBy(p => p)
                .Select(AssetDatabase.LoadAssetAtPath<Texture2D>)
                .ToArray();
            var go = new GameObject(Path.GetFileName(dir));
            GameObjectUtility.SetParentAndAlign(go, command.context as GameObject);
            Undo.RegisterCreatedObjectUndo(go, "Create Aether Canvas model");
            var model = go.AddComponent<AetherModel>();
            model.textures = pages;
            model.modelJson = json;
            Selection.activeObject = go;
        }

        [MenuItem("GameObject/Aether Canvas/Model from Selected model.json", true)]
        static bool CanCreateFromSelection() =>
            Selection.activeObject is TextAsset json &&
            AssetDatabase.GetAssetPath(json).EndsWith("model.json", System.StringComparison.OrdinalIgnoreCase);
    }
}
