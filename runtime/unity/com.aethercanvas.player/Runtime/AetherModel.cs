using System;
using System.Collections.Generic;
using System.IO;
using UnityEngine;

namespace AetherCanvas
{
    /// <summary>
    /// Plays an Aether Canvas model in a scene. Assign the exported
    /// <c>model.json</c> and its texture pages; the model plays, draws into
    /// <see cref="Output"/>, and shows on a quad (sized by
    /// <see cref="pixelsPerUnit"/>) unless <see cref="showInScene"/> is off.
    /// In edit mode it shows the rest pose.
    /// </summary>
    [ExecuteAlways]
    [DisallowMultipleComponent]
    [AddComponentMenu("Aether Canvas/Aether Model")]
    public sealed class AetherModel : MonoBehaviour
    {
        [Tooltip("The exported model.json.")]
        public TextAsset modelJson;

        [Tooltip("The model's texture pages (texture_0, texture_1, …), matched by name or else by order. " +
                 "Import them uncompressed, without mipmaps and at full size; the package's importer does this " +
                 "for textures next to a model.json.")]
        public Texture2D[] textures = Array.Empty<Texture2D>();

        [Tooltip("Motion to start when the model loads (empty for none).")]
        public string autoplay = "";

        [Tooltip("Playback speed.")]
        public float speed = 1f;

        [Tooltip("Advance time every frame.")]
        public bool playing = true;

        [Tooltip("Model pixels per world unit.")]
        public float pixelsPerUnit = 100f;

        [Tooltip("Where the transform sits on the canvas: (0, 0) is the bottom-left corner, (1, 1) the top-right.")]
        public Vector2 pivot = new Vector2(0.5f, 0.5f);

        [Tooltip("Render-texture pixels per model pixel.")]
        [Range(0.25f, 4f)]
        public float resolution = 1f;

        [Tooltip("Draw the model on a quad in the scene. Turn off to use Output yourself (a RawImage, a material).")]
        public bool showInScene = true;

        [Tooltip("Tint and fade for the whole model.")]
        public Color tint = Color.white;

        [Tooltip("Sorting layer of the quad.")]
        public string sortingLayerName = "Default";

        [Tooltip("Order within the sorting layer.")]
        public int sortingOrder;

        [Tooltip("Lip sync from what this audio source is playing.")]
        public AudioSource lipSyncSource;

        [Tooltip("Loudness gain for lip sync.")]
        public float lipSyncGain = 1f;

        [Tooltip("Play motions and switch expressions with the keys set up in the editor's Hotkeys panel " +
                 "(Unity's Input Manager; with the Input System package, call Player.PressKey yourself).")]
        public bool hotkeys = true;

        /// <summary>A motion passed one of its timeline events.</summary>
        public event Action<MotionEvent> MotionEventReached;

        /// <summary>A hotkey fired.</summary>
        public event Action<Hotkey> HotkeyPressed;

        /// <summary>The player, once a model is loaded. Drive it directly for anything not wrapped here.</summary>
        public Player Player { get; private set; }

        /// <summary>The model as last drawn (premultiplied; show it with the AetherCanvas/Display shader).</summary>
        public RenderTexture Output => modelRenderer?.Output;

        ModelRenderer modelRenderer;
        TextAsset loadedFrom;
        bool failed;
        readonly List<Texture2D> ownedTextures = new List<Texture2D>();
        GameObject view;
        Mesh quad;
        Material display;
        MaterialPropertyBlock viewBlock;
        float[] samples;

        void OnEnable()
        {
            if (modelJson != null) LoadAsset();
        }

        void OnDisable() => Release();

        void Update()
        {
            if (modelJson != loadedFrom && modelJson != null && !failed) LoadAsset();
            if (Player == null) return;
            if (lipSyncSource != null && lipSyncSource.isPlaying)
            {
                samples ??= new float[2048];
                lipSyncSource.GetOutputData(samples, 0);
                var (level, brightness) = LipSync.Measure(samples, samples.Length, AudioSettings.outputSampleRate, lipSyncGain);
                Player.SetAudio(level, brightness);
            }
            if (hotkeys && Application.isPlaying) PollHotkeys();
            if (playing && Application.isPlaying)
            {
                Player.Tick(Time.deltaTime * speed);
                foreach (var e in Player.Events) MotionEventReached?.Invoke(e);
            }
            else
            {
                Player.Update();
            }
        }

        void LateUpdate() => Draw();

        void OnValidate()
        {
            failed = false;
            if (modelRenderer != null && !Mathf.Approximately(modelRenderer.Resolution, resolution))
                modelRenderer.SetResolution(resolution);
        }

        // ------------------------------------------------------------ loading

        void LoadAsset()
        {
            loadedFrom = modelJson;
            failed = !Load(modelJson.bytes, textures);
        }

        /// <summary>
        /// Load a model from the bytes of its <c>model.json</c> and its texture
        /// pages (matched to the model's page files by name, or else by order).
        /// False, with the reason logged, when it cannot.
        /// </summary>
        public bool Load(byte[] json, IReadOnlyList<Texture> pages)
        {
            Release();
            Player player = null;
            try
            {
                player = Player.Load(json);
                modelRenderer = new ModelRenderer(player, MatchPages(player, pages), resolution);
                Player = player;
            }
            catch (Exception e)
            {
                Debug.LogError($"Aether Canvas: could not load the model: {e.Message}", this);
                player?.Dispose();
                Release();
                return false;
            }
            if (!string.IsNullOrEmpty(autoplay) && Application.isPlaying) Player.PlayMotion(autoplay);
            Player.Update();
            Draw();
            return true;
        }

        /// <summary>
        /// Load a model from a <c>model.json</c> file and the PNGs next to it,
        /// read at run time (for example from <c>Application.streamingAssetsPath</c>
        /// on desktop). False, with the reason logged, when it cannot.
        /// </summary>
        public bool LoadFromFile(string modelJsonPath)
        {
            var pages = new List<Texture2D>();
            try
            {
                var json = File.ReadAllBytes(modelJsonPath);
                var dir = Path.GetDirectoryName(modelJsonPath) ?? "";
                using (var probe = Player.Load(json))
                {
                    foreach (var file in probe.TextureFiles)
                    {
                        // Raw (linear) RGBA32: no compression, no sRGB decode, exact.
                        var texture = new Texture2D(2, 2, TextureFormat.RGBA32, false, true)
                        {
                            name = Path.GetFileNameWithoutExtension(file),
                            wrapMode = TextureWrapMode.Clamp,
                            filterMode = FilterMode.Bilinear,
                            hideFlags = HideFlags.HideAndDontSave,
                        };
                        pages.Add(texture);
                        if (!texture.LoadImage(File.ReadAllBytes(Path.Combine(dir, file)), true))
                            throw new IOException($"{file} is not a PNG");
                    }
                }
                modelJson = null;
                loadedFrom = null;
                if (!Load(json, pages.ToArray())) throw new IOException("see the error above");
                ownedTextures.AddRange(pages);
                return true;
            }
            catch (Exception e)
            {
                Debug.LogError($"Aether Canvas: could not load {modelJsonPath}: {e.Message}", this);
                foreach (var texture in pages) DestroyOwned(texture);
                return false;
            }
        }

        static IReadOnlyList<Texture> MatchPages(Player player, IReadOnlyList<Texture> pages)
        {
            pages ??= Array.Empty<Texture>();
            var matched = new Texture[player.TextureFiles.Count];
            for (var i = 0; i < matched.Length; i++)
            {
                var name = Path.GetFileNameWithoutExtension(player.TextureFiles[i]);
                foreach (var page in pages)
                {
                    if (page != null && page.name == name)
                    {
                        matched[i] = page;
                        break;
                    }
                }
                if (matched[i] == null && i < pages.Count) matched[i] = pages[i];
                if (matched[i] == null) throw new ArgumentException($"texture page {player.TextureFiles[i]} is not assigned");
            }
            return matched;
        }

        // ------------------------------------------------------------ drawing

        void Draw()
        {
            if (Player == null || modelRenderer == null) return;
            modelRenderer.Render(Player);
            UpdateView();
        }

        void UpdateView()
        {
            if (!showInScene)
            {
                if (view != null) view.SetActive(false);
                return;
            }
            if (view == null)
            {
                view = new GameObject("Aether Canvas view") { hideFlags = HideFlags.HideAndDontSave };
                view.AddComponent<MeshFilter>();
                view.AddComponent<MeshRenderer>();
                quad = new Mesh { name = "Aether Canvas quad", hideFlags = HideFlags.HideAndDontSave };
                display = new Material(Shader.Find("AetherCanvas/Display")) { hideFlags = HideFlags.HideAndDontSave };
                viewBlock = new MaterialPropertyBlock();
            }
            view.SetActive(true);
            view.layer = gameObject.layer;
            var t = view.transform;
            t.SetParent(transform, false);
            t.localPosition = Vector3.zero;
            t.localRotation = Quaternion.identity;
            t.localScale = Vector3.one;

            // Model (0, 0) is the canvas's top-left corner.
            var bottomLeft = ModelToLocal(new Vector2(0, Player.Height));
            var topRight = ModelToLocal(new Vector2(Player.Width, 0));
            quad.vertices = new[]
            {
                new Vector3(bottomLeft.x, bottomLeft.y, 0), new Vector3(topRight.x, bottomLeft.y, 0),
                new Vector3(bottomLeft.x, topRight.y, 0), new Vector3(topRight.x, topRight.y, 0),
            };
            quad.uv = new[] { new Vector2(0, 0), new Vector2(1, 0), new Vector2(0, 1), new Vector2(1, 1) };
            quad.triangles = new[] { 0, 2, 1, 1, 2, 3 };
            quad.RecalculateBounds();
            view.GetComponent<MeshFilter>().sharedMesh = quad;
            var meshRenderer = view.GetComponent<MeshRenderer>();
            meshRenderer.sharedMaterial = display;
            meshRenderer.sortingLayerName = sortingLayerName;
            meshRenderer.sortingOrder = sortingOrder;
            meshRenderer.shadowCastingMode = UnityEngine.Rendering.ShadowCastingMode.Off;
            meshRenderer.receiveShadows = false;
            viewBlock.Clear();
            viewBlock.SetTexture("_MainTex", Output);
            viewBlock.SetColor("_Color", tint);
            meshRenderer.SetPropertyBlock(viewBlock);
        }

        // -------------------------------------------------------- coordinates

        Vector2 ModelToLocal(Vector2 model)
        {
            var ppu = Mathf.Max(pixelsPerUnit, 1e-4f);
            return new Vector2((model.x - pivot.x * Player.Width) / ppu, ((1 - pivot.y) * Player.Height - model.y) / ppu);
        }

        /// <summary>A world position in model pixels (x right, y down from the canvas's top-left).</summary>
        public Vector2 WorldToModel(Vector3 world)
        {
            if (Player == null) return Vector2.zero;
            var local = transform.InverseTransformPoint(world);
            var ppu = Mathf.Max(pixelsPerUnit, 1e-4f);
            return new Vector2(local.x * ppu + pivot.x * Player.Width, (1 - pivot.y) * Player.Height - local.y * ppu);
        }

        /// <summary>A point in model pixels as a world position.</summary>
        public Vector3 ModelToWorld(Vector2 model) =>
            Player == null ? transform.position : transform.TransformPoint(ModelToLocal(model));

        /// <summary>The name of the topmost part at a world position, or null.</summary>
        public string HitTest(Vector3 world)
        {
            if (Player == null) return null;
            var m = WorldToModel(world);
            return Player.HitTest(m.x, m.y);
        }

        /// <summary>Head and eyes follow a world position (for example the mouse, through a camera).</summary>
        public void LookAtWorld(Vector3 world)
        {
            if (Player == null) return;
            var m = WorldToModel(world);
            var half = Player.Width / 2f;
            Player.LookAt(Mathf.Clamp((m.x - half) / half, -1, 1), Mathf.Clamp(-(m.y - Player.Height / 3f) / half, -1, 1));
        }

        // ------------------------------------------------------------ hotkeys

        /// <summary>Press a key: trigger the model's hotkey for it (see <see cref="Player.PressKey"/>).</summary>
        public bool PressKey(string key, bool ctrl = false, bool shift = false, bool alt = false)
        {
            if (Player == null) return false;
            var index = Player.PressKey(key, ctrl, shift, alt);
            if (index < 0) return false;
            HotkeyPressed?.Invoke(Player.Hotkeys[index]);
            return true;
        }

        void PollHotkeys()
        {
#if !ENABLE_INPUT_SYSTEM || ENABLE_LEGACY_INPUT_MANAGER
            if (!Input.anyKeyDown || Player.Hotkeys.Count == 0) return;
            var ctrl = Input.GetKey(KeyCode.LeftControl) || Input.GetKey(KeyCode.RightControl) ||
                       Input.GetKey(KeyCode.LeftCommand) || Input.GetKey(KeyCode.RightCommand);
            var shift = Input.GetKey(KeyCode.LeftShift) || Input.GetKey(KeyCode.RightShift);
            var alt = Input.GetKey(KeyCode.LeftAlt) || Input.GetKey(KeyCode.RightAlt);
            var tried = new HashSet<string>();
            foreach (var hotkey in Player.Hotkeys)
            {
                var key = hotkey.Key;
                if (!tried.Add(key)) continue;
                foreach (var code in KeyCodes(key))
                {
                    if (Input.GetKeyDown(code) && PressKey(key, ctrl, shift, alt)) return;
                }
            }
#endif
        }

        /// <summary>Unity's key codes for a hotkey's key name.</summary>
        public static IEnumerable<KeyCode> KeyCodes(string key)
        {
            if (string.IsNullOrEmpty(key)) yield break;
            if (key.Length == 1 && key[0] >= '0' && key[0] <= '9')
            {
                yield return KeyCode.Alpha0 + (key[0] - '0');
                yield return KeyCode.Keypad0 + (key[0] - '0');
                yield break;
            }
            switch (key)
            {
                case "Enter": yield return KeyCode.Return; yield return KeyCode.KeypadEnter; yield break;
                case "ArrowUp": yield return KeyCode.UpArrow; yield break;
                case "ArrowDown": yield return KeyCode.DownArrow; yield break;
                case "ArrowLeft": yield return KeyCode.LeftArrow; yield break;
                case "ArrowRight": yield return KeyCode.RightArrow; yield break;
                case "-": yield return KeyCode.Minus; yield return KeyCode.KeypadMinus; yield break;
                case "+": yield return KeyCode.Plus; yield return KeyCode.KeypadPlus; yield break;
                case "=": yield return KeyCode.Equals; yield break;
                case "[": yield return KeyCode.LeftBracket; yield break;
                case "]": yield return KeyCode.RightBracket; yield break;
                case ";": yield return KeyCode.Semicolon; yield break;
                case "'": yield return KeyCode.Quote; yield break;
                case ",": yield return KeyCode.Comma; yield break;
                case ".": yield return KeyCode.Period; yield break;
                case "/": yield return KeyCode.Slash; yield break;
                case "\\": yield return KeyCode.Backslash; yield break;
                case "`": yield return KeyCode.BackQuote; yield break;
            }
            // Letters, F1–F15, Space, Tab, Backspace, Escape, Insert, Delete,
            // Home, End, PageUp, PageDown share Unity's names.
            if (Enum.TryParse(key, out KeyCode code)) yield return code;
        }

        // ------------------------------------------------------- conveniences

        /// <summary>Set a parameter's base value. False for an unknown name.</summary>
        public bool SetParameter(string name, float value) => Player != null && Player.SetParameter(name, value);

        /// <summary>A parameter's value, or NaN.</summary>
        public float GetParameter(string name) => Player?.GetParameter(name) ?? float.NaN;

        /// <summary>Start a motion. False for an unknown name.</summary>
        public bool PlayMotion(string name, bool additive = false) => Player != null && Player.PlayMotion(name, additive);

        /// <summary>Fade to an expression; null or "" for none.</summary>
        public bool SetExpression(string name) => Player != null && Player.SetExpression(name);

        /// <summary>Switch an expression on or off, leaving the others. False for an unknown name.</summary>
        public bool ToggleExpression(string name) => Player != null && Player.ToggleExpression(name);

        /// <summary>Feed a face-tracker sample (see <see cref="Player.TrackFace(float, float, float, IEnumerable{KeyValuePair{string, float}})"/>).</summary>
        public void TrackFace(float yaw, float pitch, float roll, IEnumerable<KeyValuePair<string, float>> shapes) =>
            Player?.TrackFace(yaw, pitch, roll, shapes);

        // ----------------------------------------------------------- lifetime

        void Release()
        {
            modelRenderer?.Dispose();
            modelRenderer = null;
            Player?.Dispose();
            Player = null;
            foreach (var texture in ownedTextures) DestroyOwned(texture);
            ownedTextures.Clear();
            DestroyOwned(view);
            DestroyOwned(quad);
            DestroyOwned(display);
            view = null;
            quad = null;
            display = null;
        }

        static void DestroyOwned(UnityEngine.Object o)
        {
            if (o == null) return;
            if (Application.isPlaying) Destroy(o);
            else DestroyImmediate(o);
        }
    }
}
