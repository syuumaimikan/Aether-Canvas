// The Aether Canvas runtime for C#: a thin, safe wrapper over the C API in
// crates/aether-player/include/aether_player.h. It has no Unity dependency,
// so it also serves plain .NET hosts, and the tests run it with .NET.

using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace AetherCanvas
{
    /// <summary>How a part combines with what is under it (premultiplied).</summary>
    public enum BlendKind : uint
    {
        Normal = 0,
        Multiply = 1,
        Screen = 2,
        Add = 3,
    }

    /// <summary>Stages of the rig's per-frame evaluation that can be switched off.</summary>
    public enum Stage : uint
    {
        Motions = 0,
        Behaviours = 1,
        Drivers = 2,
        Physics = 3,
        Jiggle = 4,
    }

    /// <summary>One draw call. The layout matches <c>AetherDrawItem</c> (44 bytes).</summary>
    [StructLayout(LayoutKind.Sequential)]
    public struct DrawItem
    {
        /// <summary>The part to draw.</summary>
        public uint Part;
        /// <summary>The part whose coverage clips this one, or -1.</summary>
        public int Mask;
        public BlendKind Blend;
        /// <summary>Final opacity.</summary>
        public float Opacity;
        /// <summary>Opacity to draw the mask part's coverage with.</summary>
        public float MaskOpacity;
        /// <summary>Multiply tint, on straight colour.</summary>
        public float MultiplyR, MultiplyG, MultiplyB;
        /// <summary>Screen tint, applied after multiply.</summary>
        public float ScreenR, ScreenG, ScreenB;

        public bool HasMask => Mask >= 0;
    }

    /// <summary>A timeline event a motion passed during the last tick.</summary>
    public readonly struct MotionEvent
    {
        public readonly string Name;
        public readonly string Motion;

        public MotionEvent(string name, string motion)
        {
            Name = name;
            Motion = motion;
        }
    }

    /// <summary>A model could not be loaded.</summary>
    public sealed class AetherException : Exception
    {
        public AetherException(string message) : base(message) { }
    }

    /// <summary>
    /// Plays one exported model (<c>model.json</c>). Runs the editor's own rig
    /// code: keyforms, deformers, bones and IK, physics, drivers, motions,
    /// expressions, blinking, breathing, look-at, lip sync and face tracking.
    /// Not thread-safe; dispose it to free the native player.
    /// </summary>
    /// <summary>
    /// A key that plays a motion or switches an expression, set up in the
    /// editor's Hotkeys panel.
    /// </summary>
    public readonly struct Hotkey
    {
        /// <summary>Its index in <see cref="Player.Hotkeys"/>.</summary>
        public readonly int Index;
        /// <summary>Its name, or the motion or expression it plays.</summary>
        public readonly string Name;
        /// <summary>The keys, written like "Shift+1".</summary>
        public readonly string Keys;

        internal Hotkey(int index, string name, string keys)
        {
            Index = index;
            Name = name;
            Keys = keys;
        }

        /// <summary>The key without its modifiers ("1" for "Shift+1").</summary>
        public string Key
        {
            get
            {
                if (Keys.EndsWith("++")) return "+";
                var plus = Keys.LastIndexOf('+');
                return plus < 0 ? Keys : Keys.Substring(plus + 1);
            }
        }

        /// <inheritdoc/>
        public override string ToString() => $"{Keys}: {Name}";
    }

    public sealed class Player : IDisposable
    {
        IntPtr handle;
        readonly string[] parameters;
        readonly string[] motions;
        readonly string[] expressions;
        readonly Hotkey[] hotkeys;
        readonly string[] parts;
        readonly string[] textureFiles;
        readonly List<MotionEvent> events = new List<MotionEvent>();

        Player(IntPtr handle)
        {
            this.handle = handle;
            parameters = Names(Native.aether_player_parameter_count(handle), Native.aether_player_parameter_name);
            motions = Names(Native.aether_player_motion_count(handle), Native.aether_player_motion_name);
            expressions = Names(Native.aether_player_expression_count(handle), Native.aether_player_expression_name);
            var hotkeyNames = Names(Native.aether_player_hotkey_count(handle), Native.aether_player_hotkey_name);
            var hotkeyKeys = Names((uint)hotkeyNames.Length, Native.aether_player_hotkey_keys);
            hotkeys = new Hotkey[hotkeyNames.Length];
            for (var i = 0; i < hotkeys.Length; i++) hotkeys[i] = new Hotkey(i, hotkeyNames[i], hotkeyKeys[i]);
            parts = Names(Native.aether_player_part_count(handle), Native.aether_player_part_name);
            textureFiles = Names(Native.aether_player_texture_count(handle), Native.aether_player_texture_file);
        }

        /// <summary>Load a model from the text of its <c>model.json</c>.</summary>
        /// <exception cref="AetherException">The model is not valid.</exception>
        public static Player Load(string json) => Load(Encoding.UTF8.GetBytes(json));

        /// <summary>Load a model from the bytes of its <c>model.json</c>.</summary>
        /// <exception cref="AetherException">The model is not valid.</exception>
        public static Player Load(byte[] json)
        {
            if (json == null) throw new ArgumentNullException(nameof(json));
            var handle = Native.aether_player_new(json, (UIntPtr)json.Length);
            if (handle == IntPtr.Zero)
            {
                var error = Str(Native.aether_last_error(out var len), len);
                throw new AetherException(error ?? "the model could not be loaded");
            }
            return new Player(handle);
        }

        /// <summary>The C API version the native library implements (1).</summary>
        public static uint ApiVersion => Native.aether_api_version();

        IntPtr Handle => handle != IntPtr.Zero ? handle : throw new ObjectDisposedException(nameof(Player));

        // ------------------------------------------------------------ canvas

        /// <summary>Canvas width in model pixels.</summary>
        public int Width => (int)Native.aether_player_width(Handle);

        /// <summary>Canvas height in model pixels.</summary>
        public int Height => (int)Native.aether_player_height(Handle);

        /// <summary>Texture page files, relative to <c>model.json</c>.</summary>
        public IReadOnlyList<string> TextureFiles => textureFiles;

        // -------------------------------------------------------- parameters

        /// <summary>Parameter names, in panel order.</summary>
        public IReadOnlyList<string> Parameters => parameters;

        /// <summary>A parameter's index, or -1.</summary>
        public int FindParameter(string name) => Array.IndexOf(parameters, name);

        /// <summary>A parameter's minimum, maximum and default.</summary>
        public (float min, float max, float defaultValue) ParameterRange(int index)
        {
            var range = new float[3];
            Native.aether_player_parameter_range(Handle, (uint)index, range);
            return (range[0], range[1], range[2]);
        }

        /// <summary>A parameter's value as last evaluated.</summary>
        public float GetParameter(int index) => Native.aether_player_parameter_value(Handle, (uint)index);

        /// <summary>A parameter's value as last evaluated, or NaN for an unknown name.</summary>
        public float GetParameter(string name)
        {
            var index = FindParameter(name);
            return index >= 0 ? GetParameter(index) : float.NaN;
        }

        /// <summary>Set a parameter's base value; motions, physics and behaviours layer on top.</summary>
        public void SetParameter(int index, float value) => Native.aether_player_set_parameter(Handle, (uint)index, value);

        /// <summary>Set a parameter by name. False for an unknown name.</summary>
        public bool SetParameter(string name, float value)
        {
            var index = FindParameter(name);
            if (index < 0) return false;
            SetParameter(index, value);
            return true;
        }

        // ------------------------------------------ motions and expressions

        /// <summary>Motion names.</summary>
        public IReadOnlyList<string> Motions => motions;

        /// <summary>A motion's length in seconds.</summary>
        public float MotionDuration(int index) => Native.aether_player_motion_duration(Handle, (uint)index);

        /// <summary>Start a motion; <paramref name="additive"/> layers it on top. False for an unknown name.</summary>
        public bool PlayMotion(string name, bool additive = false)
        {
            var index = Array.IndexOf(motions, name);
            return index >= 0 && Native.aether_player_play_motion(Handle, (uint)index, additive ? 1u : 0u) != 0;
        }

        /// <summary>Fade every motion out.</summary>
        public void StopMotions() => Native.aether_player_stop_motions(Handle);

        /// <summary>True while a motion plays or fades.</summary>
        public bool IsPlaying => Native.aether_player_is_playing(Handle) != 0;

        /// <summary>Expression names.</summary>
        public IReadOnlyList<string> Expressions => expressions;

        /// <summary>Fade to an expression; null or "" fades out of all. False for an unknown name.</summary>
        public bool SetExpression(string name)
        {
            var index = string.IsNullOrEmpty(name) ? -1 : Array.IndexOf(expressions, name);
            if (index < 0 && !string.IsNullOrEmpty(name)) return false;
            Native.aether_player_set_expression(Handle, index);
            return true;
        }

        /// <summary>Switch an expression on or off, leaving the others. False for an unknown name.</summary>
        public bool ToggleExpression(string name)
        {
            var index = Array.IndexOf(expressions, name);
            if (index < 0) return false;
            Native.aether_player_toggle_expression(Handle, (uint)index);
            return true;
        }

        /// <summary>True while an expression is switched on.</summary>
        public bool IsExpressionActive(string name)
        {
            var index = Array.IndexOf(expressions, name);
            return index >= 0 && Native.aether_player_expression_active(Handle, (uint)index) != 0;
        }

        // ----------------------------------------------------------- hotkeys

        /// <summary>The model's hotkeys.</summary>
        public IReadOnlyList<Hotkey> Hotkeys => hotkeys;

        /// <summary>Carry out a hotkey as if its key were pressed. False for a bad index.</summary>
        public bool TriggerHotkey(int index) =>
            index >= 0 && Native.aether_player_trigger_hotkey(Handle, (uint)index) != 0;

        /// <summary>
        /// A key was pressed: trigger the hotkey bound to it. <paramref name="key"/>
        /// names it as printed ("1", "A", "F5", "Space"); Unity's KeyCode names
        /// ("Alpha1", "Keypad1") work too. Returns the hotkey's index, or -1.
        /// </summary>
        public int PressKey(string key, bool ctrl = false, bool shift = false, bool alt = false)
        {
            if (string.IsNullOrEmpty(key)) return -1;
            var bytes = Encoding.UTF8.GetBytes(key);
            var modifiers = (ctrl ? 1u : 0u) | (shift ? 2u : 0u) | (alt ? 4u : 0u);
            return Native.aether_player_press_key(Handle, bytes, (UIntPtr)bytes.Length, modifiers);
        }

        // ------------------------------------------------------------ inputs

        /// <summary>Head and eyes follow a point: -1..1 on each axis, y up.</summary>
        public void LookAt(float x, float y) => Native.aether_player_look_at(Handle, x, y);

        /// <summary>Look straight ahead.</summary>
        public void LookAhead() => Native.aether_player_look_ahead(Handle);

        /// <summary>Lip sync: voice loudness (0..1) and brightness (-1..1); see <see cref="LipSync"/>.</summary>
        public void SetAudio(float level, float brightness) => Native.aether_player_set_audio(Handle, level, brightness);

        /// <summary>Switch a stage of the rig's evaluation on or off.</summary>
        public void SetStage(Stage stage, bool enabled) =>
            Native.aether_player_set_stage(Handle, (uint)stage, enabled ? 1u : 0u);

        // -------------------------------------------------------------- time

        /// <summary>Advance time: motions, behaviours, physics, then the pose.</summary>
        public void Tick(float seconds)
        {
            Native.aether_player_tick(Handle, seconds);
            events.Clear();
            var count = Native.aether_player_event_count(handle);
            for (uint i = 0; i < count; i++)
            {
                var name = Str(Native.aether_player_event_name(handle, i, out var len), len);
                var motion = Native.aether_player_event_motion(handle, i);
                events.Add(new MotionEvent(name, motion >= 0 && motion < motions.Length ? motions[motion] : null));
            }
        }

        /// <summary>Recompute the pose after parameter changes, without advancing time.</summary>
        public void Update() => Native.aether_player_update(Handle);

        /// <summary>Back to the default pose; motions stopped, simulations at rest.</summary>
        public void Reset()
        {
            Native.aether_player_reset(Handle);
            events.Clear();
        }

        /// <summary>Motion events passed during the last <see cref="Tick"/>.</summary>
        public IReadOnlyList<MotionEvent> Events => events;

        // ------------------------------------------------------ face tracking

        static string[] blendShapes;

        /// <summary>The 52 ARKit/MediaPipe blend shape names, in the C API's order.</summary>
        public static IReadOnlyList<string> BlendShapes
        {
            get
            {
                if (blendShapes == null)
                {
                    var count = Native.aether_blendshape_count();
                    var names = new string[count];
                    for (uint i = 0; i < count; i++) names[i] = Str(Native.aether_blendshape_name(i, out var len), len);
                    blendShapes = names;
                }
                return blendShapes;
            }
        }

        /// <summary>
        /// Feed a face-tracker sample: head angles in degrees in the tracked
        /// person's frame (yaw toward their left, pitch up, roll toward their
        /// left shoulder) and blend shape weights in <see cref="BlendShapes"/>
        /// order (fewer count as zero).
        /// </summary>
        public void TrackFace(float yaw, float pitch, float roll, float[] shapes)
        {
            Native.aether_player_track_face(Handle, yaw, pitch, roll, shapes ?? Array.Empty<float>(),
                (uint)(shapes?.Length ?? 0));
        }

        /// <summary>Feed a face-tracker sample with blend shapes by name (<c>"jawOpen"</c>, …).</summary>
        public void TrackFace(float yaw, float pitch, float roll, IEnumerable<KeyValuePair<string, float>> shapes)
        {
            var names = BlendShapes;
            var weights = new float[names.Count];
            if (shapes != null)
            {
                foreach (var pair in shapes)
                {
                    for (var i = 0; i < names.Count; i++)
                    {
                        if (names[i] == pair.Key)
                        {
                            weights[i] = pair.Value;
                            break;
                        }
                    }
                }
            }
            TrackFace(yaw, pitch, roll, weights);
        }

        /// <summary>Make the latest tracked face the neutral one.</summary>
        public void CalibrateTracking() => Native.aether_player_track_calibrate(Handle);

        /// <summary>Stop following the tracker; auto-blink resumes.</summary>
        public void StopTracking() => Native.aether_player_track_stop(Handle);

        /// <summary>True while tracking samples arrive.</summary>
        public bool IsTracking => Native.aether_player_is_tracking(Handle) != 0;

        /// <summary>
        /// Tracking settings. <paramref name="mirror"/> moves the model like a
        /// mirror image (the default). NaN keeps a value unchanged.
        /// </summary>
        public void SetTrackingSettings(bool mirror, float smoothing = float.NaN, float headGain = float.NaN,
            float bodyFollow = float.NaN, float mouthGain = float.NaN)
        {
            Native.aether_player_track_settings(Handle, mirror ? 1u : 0u, smoothing, headGain, bodyFollow, mouthGain);
        }

        // ------------------------------------------------ geometry and drawing

        /// <summary>Part names; parts are drawn from these indices.</summary>
        public IReadOnlyList<string> Parts => parts;

        /// <summary>The texture page a part samples.</summary>
        public int PartTexture(int part) => (int)Native.aether_player_part_texture(Handle, (uint)part);

        /// <summary>How many vertices a part's mesh has.</summary>
        public int PartVertexCount(int part) => (int)Native.aether_player_part_vertex_count(Handle, (uint)part);

        /// <summary>A part's texture coordinates, u and v interleaved (0..1, v down).</summary>
        public float[] PartUvs(int part)
        {
            var uvs = new float[PartVertexCount(part) * 2];
            var ptr = Native.aether_player_part_uvs(Handle, (uint)part);
            if (ptr != IntPtr.Zero && uvs.Length > 0) Marshal.Copy(ptr, uvs, 0, uvs.Length);
            return uvs;
        }

        /// <summary>A part's triangles, three vertex indices each.</summary>
        public int[] PartIndices(int part)
        {
            var indices = new int[Native.aether_player_part_index_count(Handle, (uint)part)];
            var ptr = Native.aether_player_part_indices(handle, (uint)part);
            if (ptr != IntPtr.Zero && indices.Length > 0) Marshal.Copy(ptr, indices, 0, indices.Length);
            return indices;
        }

        /// <summary>
        /// A part's current vertex positions in model pixels (y down), x and y
        /// interleaved, without copying. Valid until the next tick, update or reset.
        /// </summary>
        public unsafe ReadOnlySpan<float> Positions(int part)
        {
            var ptr = Native.aether_player_part_positions(Handle, (uint)part);
            return ptr == IntPtr.Zero ? default : new ReadOnlySpan<float>((void*)ptr, PartVertexCount(part) * 2);
        }

        /// <summary>
        /// This frame's draw calls, back to front, without copying. Valid until
        /// the next tick, update or reset.
        /// </summary>
        public unsafe ReadOnlySpan<DrawItem> DrawList
        {
            get
            {
                var ptr = Native.aether_player_draw_items(Handle);
                var count = (int)Native.aether_player_draw_count(handle);
                return ptr == IntPtr.Zero ? default : new ReadOnlySpan<DrawItem>((void*)ptr, count);
            }
        }

        /// <summary>The topmost part under a point in model pixels, or -1.</summary>
        public int HitTestIndex(float x, float y) => Native.aether_player_hit_test(Handle, x, y);

        /// <summary>The name of the topmost part under a point in model pixels, or null.</summary>
        public string HitTest(float x, float y)
        {
            var index = HitTestIndex(x, y);
            return index >= 0 && index < parts.Length ? parts[index] : null;
        }

        // ---------------------------------------------------------- lifetime

        public void Dispose()
        {
            if (handle != IntPtr.Zero)
            {
                Native.aether_player_free(handle);
                handle = IntPtr.Zero;
            }
            GC.SuppressFinalize(this);
        }

        ~Player()
        {
            if (handle != IntPtr.Zero) Native.aether_player_free(handle);
        }

        // ----------------------------------------------------------- helpers

        delegate IntPtr NameFn(IntPtr player, uint index, out UIntPtr len);

        string[] Names(uint count, NameFn name)
        {
            var names = new string[count];
            for (uint i = 0; i < count; i++) names[i] = Str(name(handle, i, out var len), len);
            return names;
        }

        static string Str(IntPtr ptr, UIntPtr len)
        {
            if (ptr == IntPtr.Zero) return null;
            var bytes = new byte[(int)len];
            Marshal.Copy(ptr, bytes, 0, bytes.Length);
            return Encoding.UTF8.GetString(bytes);
        }
    }

    /// <summary>The C API. See <c>aether_player.h</c>.</summary>
    static class Native
    {
#if UNITY_IOS && !UNITY_EDITOR
        const string Lib = "__Internal";
#else
        const string Lib = "aether_player";
#endif
        const CallingConvention C = CallingConvention.Cdecl;

        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_api_version();
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_last_error(out UIntPtr len);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_new(byte[] json, UIntPtr len);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_free(IntPtr p);

        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_width(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_height(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_texture_count(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_texture_file(IntPtr p, uint index, out UIntPtr len);

        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_parameter_count(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_parameter_name(IntPtr p, uint index, out UIntPtr len);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_parameter_range(IntPtr p, uint index, [Out] float[] range);
        [DllImport(Lib, CallingConvention = C)] public static extern float aether_player_parameter_value(IntPtr p, uint index);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_set_parameter(IntPtr p, uint index, float value);

        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_motion_count(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_motion_name(IntPtr p, uint index, out UIntPtr len);
        [DllImport(Lib, CallingConvention = C)] public static extern float aether_player_motion_duration(IntPtr p, uint index);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_play_motion(IntPtr p, uint index, uint additive);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_stop_motions(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_is_playing(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_expression_count(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_expression_name(IntPtr p, uint index, out UIntPtr len);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_set_expression(IntPtr p, int index);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_toggle_expression(IntPtr p, uint index);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_expression_active(IntPtr p, uint index);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_hotkey_count(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_hotkey_name(IntPtr p, uint index, out UIntPtr len);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_hotkey_keys(IntPtr p, uint index, out UIntPtr len);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_trigger_hotkey(IntPtr p, uint index);
        [DllImport(Lib, CallingConvention = C)] public static extern int aether_player_press_key(IntPtr p, byte[] key, UIntPtr len, uint modifiers);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_look_at(IntPtr p, float x, float y);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_look_ahead(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_set_audio(IntPtr p, float level, float brightness);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_set_stage(IntPtr p, uint stage, uint enabled);

        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_tick(IntPtr p, float dt);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_update(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_reset(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_event_count(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_event_name(IntPtr p, uint index, out UIntPtr len);
        [DllImport(Lib, CallingConvention = C)] public static extern int aether_player_event_motion(IntPtr p, uint index);

        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_blendshape_count();
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_blendshape_name(uint index, out UIntPtr len);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_track_face(IntPtr p, float yaw, float pitch, float roll, float[] shapes, uint count);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_track_calibrate(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_track_stop(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_is_tracking(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern void aether_player_track_settings(IntPtr p, uint mirror, float smoothing, float headGain, float bodyFollow, float mouthGain);

        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_part_count(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_part_name(IntPtr p, uint index, out UIntPtr len);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_part_texture(IntPtr p, uint index);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_part_vertex_count(IntPtr p, uint index);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_part_uvs(IntPtr p, uint index);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_part_index_count(IntPtr p, uint index);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_part_indices(IntPtr p, uint index);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_part_positions(IntPtr p, uint index);
        [DllImport(Lib, CallingConvention = C)] public static extern uint aether_player_draw_count(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern IntPtr aether_player_draw_items(IntPtr p);
        [DllImport(Lib, CallingConvention = C)] public static extern int aether_player_hit_test(IntPtr p, float x, float y);
    }
}
