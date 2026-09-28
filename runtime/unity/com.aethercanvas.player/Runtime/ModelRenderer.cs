using System;
using System.Collections.Generic;
using UnityEngine;
using UnityEngine.Experimental.Rendering;
using UnityEngine.Rendering;

namespace AetherCanvas
{
    /// <summary>
    /// Draws a <see cref="Player"/> into a render texture with a command
    /// buffer, the way every Aether renderer draws: premultiplied textures,
    /// the draw list back to front, the four blend modes with fixed blend
    /// states, and clipping through a mask target. Works with any render
    /// pipeline, since it renders into its own target. <see cref="AetherModel"/>
    /// shows the result; use this class directly to draw somewhere else.
    /// </summary>
    /// <remarks>
    /// Colour stays in the model's own (sRGB-encoded) space from the textures
    /// to <see cref="Output"/>, as in the editor. In a Linear colour-space
    /// project, sRGB textures are re-encoded on load, and the
    /// <c>AetherCanvas/Display</c> shader converts the output for the scene.
    /// </remarks>
    public sealed class ModelRenderer : IDisposable
    {
        // Passes of Hidden/AetherCanvas/Part.
        const int PassNormal = 0;
        const int PassMultiplyColour = 1;
        const int PassMultiplyAlpha = 2;
        const int PassScreen = 3;
        const int PassAdd = 4;
        const int PassMask = 5;
        const int PassPremultiply = 6;

        static readonly int MainTexId = Shader.PropertyToID("_MainTex");
        static readonly int OpacityId = Shader.PropertyToID("_AetherOpacity");
        static readonly int MultiplyId = Shader.PropertyToID("_AetherMultiply");
        static readonly int ScreenId = Shader.PropertyToID("_AetherScreen");
        static readonly int UseMaskId = Shader.PropertyToID("_AetherUseMask");
        static readonly int MaskId = Shader.PropertyToID("_AetherMask");
        static readonly int DecodeId = Shader.PropertyToID("_AetherDecode");

        readonly int width;
        readonly int height;
        readonly Material material;
        readonly RenderTexture[] pages;
        readonly int[] partPage;
        readonly Mesh[] meshes;
        readonly Vector3[][] vertices;
        readonly int[] updated;
        readonly List<MaterialPropertyBlock> blocks = new List<MaterialPropertyBlock>();
        readonly CommandBuffer commands = new CommandBuffer { name = "Aether Canvas" };
        RenderTexture mask;
        int frame;

        /// <summary>The model as drawn by the last <see cref="Render"/>: premultiplied alpha.</summary>
        public RenderTexture Output { get; private set; }

        /// <summary>Render-texture pixels per model pixel.</summary>
        public float Resolution { get; private set; }

        /// <param name="player">The player to draw; its parts become meshes.</param>
        /// <param name="textures">The model's texture pages, in <see cref="Player.TextureFiles"/> order.</param>
        /// <param name="resolution">Render-texture pixels per model pixel.</param>
        public ModelRenderer(Player player, IReadOnlyList<Texture> textures, float resolution = 1f)
        {
            if (player == null) throw new ArgumentNullException(nameof(player));
            if (textures == null || textures.Count < player.TextureFiles.Count)
                throw new ArgumentException($"the model has {player.TextureFiles.Count} texture pages");
            var shader = Shader.Find("Hidden/AetherCanvas/Part");
            if (shader == null)
                throw new InvalidOperationException("the Hidden/AetherCanvas/Part shader is missing from the build");
            material = new Material(shader) { hideFlags = HideFlags.HideAndDontSave };
            width = player.Width;
            height = player.Height;

            // Premultiply every page once, on the GPU, into raw 8-bit targets.
            pages = new RenderTexture[player.TextureFiles.Count];
            for (var i = 0; i < pages.Length; i++)
            {
                var texture = textures[i];
                if (texture == null) throw new ArgumentException($"texture page {player.TextureFiles[i]} is missing");
                var page = new RenderTexture(texture.width, texture.height, 0, RenderTextureFormat.ARGB32,
                    RenderTextureReadWrite.Linear)
                {
                    name = player.TextureFiles[i],
                    filterMode = FilterMode.Bilinear,
                    wrapMode = TextureWrapMode.Clamp,
                    useMipMap = false,
                    hideFlags = HideFlags.HideAndDontSave,
                };
                page.Create();
                material.SetFloat(DecodeId, IsDecodedOnSampling(texture) ? 1f : 0f);
                Graphics.Blit(texture, page, material, PassPremultiply);
                pages[i] = page;
            }

            var count = player.Parts.Count;
            meshes = new Mesh[count];
            vertices = new Vector3[count][];
            partPage = new int[count];
            updated = new int[count];
            for (var p = 0; p < count; p++)
            {
                var n = player.PartVertexCount(p);
                var uv = player.PartUvs(p);
                var uvs = new Vector2[n];
                // The model's v runs down the PNG; Unity's runs up.
                for (var k = 0; k < n; k++) uvs[k] = new Vector2(uv[2 * k], 1f - uv[2 * k + 1]);
                vertices[p] = new Vector3[n];
                var mesh = new Mesh
                {
                    name = player.Parts[p],
                    hideFlags = HideFlags.HideAndDontSave,
                    indexFormat = n > 65535 ? IndexFormat.UInt32 : IndexFormat.UInt16,
                };
                mesh.MarkDynamic();
                mesh.vertices = vertices[p];
                mesh.uv = uvs;
                mesh.SetTriangles(player.PartIndices(p), 0, false);
                // Drawn by command buffer, never culled.
                mesh.bounds = new Bounds(Vector3.zero, new Vector3(1e6f, 1e6f, 1f));
                meshes[p] = mesh;
                partPage[p] = Mathf.Clamp(player.PartTexture(p), 0, pages.Length - 1);
                updated[p] = -1;
            }
            SetResolution(resolution);
        }

        /// <summary>Change the output's pixels per model pixel (recreates the targets).</summary>
        public void SetResolution(float resolution)
        {
            resolution = Mathf.Clamp(resolution, 0.05f, 8f);
            var w = Mathf.Max(1, Mathf.CeilToInt(width * resolution));
            var h = Mathf.Max(1, Mathf.CeilToInt(height * resolution));
            Resolution = resolution;
            if (Output != null && Output.width == w && Output.height == h) return;
            Release(Output);
            Release(mask);
            Output = new RenderTexture(w, h, 0, RenderTextureFormat.ARGB32, RenderTextureReadWrite.Linear)
            {
                name = "Aether Canvas output",
                filterMode = FilterMode.Bilinear,
                wrapMode = TextureWrapMode.Clamp,
                hideFlags = HideFlags.HideAndDontSave,
            };
            Output.Create();
            var maskFormat = SystemInfo.SupportsRenderTextureFormat(RenderTextureFormat.R8)
                ? RenderTextureFormat.R8
                : RenderTextureFormat.ARGB32;
            mask = new RenderTexture(w, h, 0, maskFormat, RenderTextureReadWrite.Linear)
            {
                name = "Aether Canvas mask",
                filterMode = FilterMode.Point,
                hideFlags = HideFlags.HideAndDontSave,
            };
            mask.Create();
        }

        /// <summary>Draw the player's current pose into <see cref="Output"/>.</summary>
        /// <param name="player">The player this renderer was made for.</param>
        /// <param name="background">Clear colour (straight alpha); transparent by default.</param>
        public void Render(Player player, Color background = default)
        {
            frame++;
            var projection = Matrix4x4.Ortho(0, width, height, 0, -1, 1);
            commands.Clear();
            Target(Output, projection);
            commands.ClearRenderTarget(false, true,
                new Color(background.r * background.a, background.g * background.a, background.b * background.a,
                    background.a));

            var used = 0;
            var maskPart = -1;
            var maskOpacity = -1f;
            var list = player.DrawList;
            for (var i = 0; i < list.Length; i++)
            {
                var item = list[i];
                var part = (int)item.Part;
                if (item.Opacity <= 0f || part >= meshes.Length) continue;
                if (item.HasMask && item.Mask < meshes.Length &&
                    (item.Mask != maskPart || item.MaskOpacity != maskOpacity))
                {
                    // The base's coverage, at its opacity, into the mask target.
                    Target(mask, projection);
                    commands.ClearRenderTarget(false, true, Color.clear);
                    var mb = Block(used++);
                    mb.SetTexture(MainTexId, pages[partPage[item.Mask]]);
                    mb.SetFloat(OpacityId, item.MaskOpacity);
                    commands.DrawMesh(Mesh(player, item.Mask), Matrix4x4.identity, material, 0, PassMask, mb);
                    Target(Output, projection);
                    maskPart = item.Mask;
                    maskOpacity = item.MaskOpacity;
                }

                var block = Block(used++);
                block.SetTexture(MainTexId, pages[partPage[part]]);
                block.SetFloat(OpacityId, item.Opacity);
                block.SetVector(MultiplyId, new Vector4(item.MultiplyR, item.MultiplyG, item.MultiplyB, 1f));
                block.SetVector(ScreenId, new Vector4(item.ScreenR, item.ScreenG, item.ScreenB, 0f));
                block.SetFloat(UseMaskId, item.HasMask ? 1f : 0f);
                block.SetTexture(MaskId, item.HasMask ? (Texture)mask : Texture2D.blackTexture);
                var mesh = Mesh(player, part);
                switch (item.Blend)
                {
                    case BlendKind.Multiply:
                        // Premultiplied multiply is Cs·Cd + Cs·(1−αd) + Cd·(1−αs):
                        // two passes, the first leaving alpha alone.
                        commands.DrawMesh(mesh, Matrix4x4.identity, material, 0, PassMultiplyColour, block);
                        commands.DrawMesh(mesh, Matrix4x4.identity, material, 0, PassMultiplyAlpha, block);
                        break;
                    case BlendKind.Screen:
                        commands.DrawMesh(mesh, Matrix4x4.identity, material, 0, PassScreen, block);
                        break;
                    case BlendKind.Add:
                        commands.DrawMesh(mesh, Matrix4x4.identity, material, 0, PassAdd, block);
                        break;
                    default:
                        commands.DrawMesh(mesh, Matrix4x4.identity, material, 0, PassNormal, block);
                        break;
                }
            }
            Graphics.ExecuteCommandBuffer(commands);
        }

        void Target(RenderTexture target, Matrix4x4 projection)
        {
            commands.SetRenderTarget(target);
            commands.SetViewProjectionMatrices(Matrix4x4.identity, projection);
        }

        // A property block per draw call in a frame, reused across frames.
        MaterialPropertyBlock Block(int index)
        {
            while (blocks.Count <= index) blocks.Add(new MaterialPropertyBlock());
            var block = blocks[index];
            block.Clear();
            return block;
        }

        // A part's mesh with this frame's vertex positions.
        Mesh Mesh(Player player, int part)
        {
            var mesh = meshes[part];
            if (updated[part] == frame) return mesh;
            updated[part] = frame;
            var positions = player.Positions(part);
            var v = vertices[part];
            var n = Math.Min(v.Length, positions.Length / 2);
            for (var k = 0; k < n; k++) v[k] = new Vector3(positions[2 * k], positions[2 * k + 1], 0f);
            mesh.vertices = v;
            return mesh;
        }

        // In a Linear colour-space project, sampling an sRGB texture decodes it.
        static bool IsDecodedOnSampling(Texture texture) =>
            QualitySettings.activeColorSpace == ColorSpace.Linear &&
            GraphicsFormatUtility.IsSRGBFormat(texture.graphicsFormat);

        public void Dispose()
        {
            commands.Release();
            Release(Output);
            Release(mask);
            Output = null;
            mask = null;
            foreach (var page in pages) Release(page);
            foreach (var mesh in meshes) Destroy(mesh);
            Destroy(material);
        }

        static void Release(RenderTexture texture)
        {
            if (texture == null) return;
            texture.Release();
            Destroy(texture);
        }

        static void Destroy(UnityEngine.Object o)
        {
            if (o == null) return;
            if (Application.isPlaying) UnityEngine.Object.Destroy(o);
            else UnityEngine.Object.DestroyImmediate(o);
        }
    }
}
