// Tests for the C# binding, run with the native library:
//
//   dotnet run --project Tests~/Binding -- MODEL_DIR REFERENCE_DIR
//
// MODEL_DIR holds the demo character's model.json; REFERENCE_DIR its
// reference poses (runtime/web/build.sh --demo writes both). Exits non-zero
// if any check fails.

using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text.Json;
using AetherCanvas;

static class Program
{
    static int failures;

    static void Check(bool ok, string what)
    {
        if (ok)
        {
            Console.WriteLine($"ok - {what}");
        }
        else
        {
            failures++;
            Console.Error.WriteLine($"not ok - {what}");
        }
    }

    static int Main(string[] args)
    {
        var modelDir = args.Length > 0 ? args[0] : "../../../../web/model";
        var referenceDir = args.Length > 1 ? args[1] : "../../../../web/test/reference";
        var json = File.ReadAllBytes(Path.Combine(modelDir, "model.json"));

        Check(Player.ApiVersion == 1, "the native library speaks C API version 1");
        Check(Marshal.SizeOf<DrawItem>() == 44, "DrawItem has the C struct's size");
        Check(Player.BlendShapes.Count == 52 && Player.BlendShapes.Contains("jawOpen"), "the 52 blend shapes are listed");

        using (var player = Player.Load(json))
        {
            Basics(player);
            Poses(player, referenceDir);
            Drawing(player);
            Motions(player);
            Hotkeys(player);
            Tracking(player);
        }

        try
        {
            Player.Load("{\"format\": \"not a model\"}");
            Check(false, "an invalid model is refused");
        }
        catch (AetherException e)
        {
            Check(e.Message.Length > 0, $"an invalid model is refused with a reason ({e.Message})");
        }

        var disposed = Player.Load(json);
        disposed.Dispose();
        try
        {
            disposed.Tick(0.1f);
            Check(false, "a disposed player refuses to run");
        }
        catch (ObjectDisposedException)
        {
            Check(true, "a disposed player refuses to run");
        }

        LipSyncChecks();

        Console.WriteLine($"{failures} failed");
        return failures > 0 ? 1 : 0;
    }

    static void Hotkeys(Player player)
    {
        player.Reset();
        var listed = string.Join(", ", player.Hotkeys.Select(k => k.ToString()));
        Check(listed == "1: Greeting, Shift+1: Smile, Shift+2: Surprised, 0: Reset", $"hotkeys are listed ({listed})");
        Check(player.Hotkeys[1].Key == "1", "a hotkey's key without its modifiers");
        Check(player.PressKey("Alpha1", shift: true) == 1, "Unity's key names press hotkeys");
        player.Tick(1f);
        Check(player.IsExpressionActive("Smile") && Math.Abs(player.GetParameter("MouthForm") - 1) < 1e-6,
            "a hotkey switches its expression on");
        Check(player.ToggleExpression("Smile"), "expressions toggle by name");
        player.Tick(1f);
        Check(!player.IsExpressionActive("Smile"), "and off again");
        Check(player.PressKey("1") == 0 && player.IsPlaying, "a hotkey plays its motion");
        Check(player.PressKey("7") == -1 && player.PressKey("") == -1, "unbound keys do nothing");
        Check(player.TriggerHotkey(3) && !player.TriggerHotkey(9), "hotkeys trigger by index");
        player.Reset();
    }

    static void Basics(Player player)
    {
        Check(player.Width == 512 && player.Height == 640, "canvas size");
        Check(player.TextureFiles.SequenceEqual(new[] { "texture_0.png" }), "texture pages");
        Check(player.Parameters.Contains("AngleX") && player.FindParameter("AngleX") >= 0, "standard parameters");
        var (min, max, rest) = player.ParameterRange(player.FindParameter("AngleX"));
        Check(min == -30 && max == 30 && rest == 0, $"parameter ranges ({min}, {max}, {rest})");
        Check(player.Motions.Contains("Greeting"), "motions");
        Check(player.Parts.Contains("Face") && player.Parts.Count == 17, "parts");
        Check(player.SetParameter("AngleX", 12) && Math.Abs(player.GetParameter("AngleX") - 12) < 1e-6,
            "parameters set and read back");
        Check(!player.SetParameter("NoSuchParameter", 1) && float.IsNaN(player.GetParameter("NoSuchParameter")),
            "unknown parameters are refused and read as NaN");
        player.Reset();
    }

    // The binding must hand over exactly the native player's geometry: every
    // vertex of every part, at every reference pose.
    static void Poses(Player player, string referenceDir)
    {
        using var poses = JsonDocument.Parse(File.ReadAllText(Path.Combine(referenceDir, "poses.json")));
        var count = 0;
        foreach (var pose in poses.RootElement.EnumerateArray())
        {
            player.Reset();
            foreach (var value in pose.GetProperty("values").EnumerateObject())
                player.SetParameter(value.Name, value.Value.GetSingle());
            player.Update();
            var worst = 0.0;
            var part = 0;
            foreach (var expected in pose.GetProperty("positions").EnumerateArray())
            {
                var actual = player.Positions(part);
                var k = 0;
                foreach (var v in expected.EnumerateArray())
                {
                    worst = Math.Max(worst, k < actual.Length ? Math.Abs(actual[k] - v.GetDouble()) : double.MaxValue);
                    k++;
                }
                if (k != actual.Length) worst = double.MaxValue;
                part++;
            }
            Check(part == player.Parts.Count && worst < 1e-3,
                $"pose '{pose.GetProperty("name").GetString()}': every vertex matches (worst {worst:g3} px)");
            count++;
        }
        Check(count == 4, "four reference poses");
        player.Reset();
    }

    static void Drawing(Player player)
    {
        player.Update();
        var list = player.DrawList;
        Check(list.Length > 0 && list.Length <= player.Parts.Count, $"a draw list ({list.Length} items)");
        var valid = true;
        var clipped = false;
        var irisL = player.Parts.ToList().IndexOf("Iris L");
        var whiteL = player.Parts.ToList().IndexOf("Eye white L");
        foreach (var item in list)
        {
            valid &= item.Part < player.Parts.Count && item.Opacity >= 0 && item.Opacity <= 1;
            valid &= item.Blend == BlendKind.Normal;
            if (item.Part == irisL) clipped = item.Mask == whiteL && item.MaskOpacity == 1;
        }
        Check(valid, "draw items are well formed");
        Check(clipped, "the iris is clipped to the eye white");
        for (var p = 0; p < player.Parts.Count; p++)
        {
            var uvs = player.PartUvs(p);
            var indices = player.PartIndices(p);
            valid &= uvs.Length == player.PartVertexCount(p) * 2 && uvs.All(u => u >= 0 && u <= 1);
            valid &= indices.Length > 0 && indices.Length % 3 == 0 && indices.All(i => i >= 0 && i < player.PartVertexCount(p));
            valid &= player.PartTexture(p) == 0;
        }
        Check(valid, "meshes: UVs in range, whole triangles, indices in range");
        Check(player.HitTest(256, 330) == "Face", "hit testing finds the face");
        Check(player.HitTest(-5, -5) == null, "nothing outside the model");
    }

    static void Motions(Player player)
    {
        player.Reset();
        Check(player.PlayMotion("Greeting"), "a motion starts");
        Check(!player.PlayMotion("NoSuchMotion"), "unknown motions are refused");
        Check(player.MotionDuration(player.Motions.ToList().IndexOf("Greeting")) > 0, "motions have a length");
        var moved = false;
        for (var i = 0; i < 90; i++)
        {
            player.Tick(1 / 60f);
            moved |= Math.Abs(player.GetParameter("AngleX")) > 1;
        }
        Check(moved && player.IsPlaying, "the motion drives the rig");
        player.StopMotions();
        for (var i = 0; i < 90; i++) player.Tick(1 / 60f);
        Check(!player.IsPlaying, "stopped motions fade out");
        Check(player.Events.Count == 0, "no events from a motion without any");
        Check(player.SetExpression("") && !player.SetExpression("NoSuchExpression"), "expressions by name");
    }

    static void Tracking(Player player)
    {
        player.Reset();
        var shapes = new Dictionary<string, float> { ["jawOpen"] = 0.5f, ["eyeBlinkLeft"] = 0.9f };
        for (var i = 0; i < 60; i++)
        {
            player.TrackFace(20, 0, 0, shapes);
            player.Tick(1 / 60f);
        }
        Check(player.IsTracking, "tracking");
        Check(player.GetParameter("AngleX") < -15, "a face turned left turns the mirror image left");
        Check(player.GetParameter("MouthOpenY") > 0.8f, "the tracked mouth opens the model's");
        Check(player.GetParameter("EyeLOpen") < 0.1f, "a tracked wink closes the matching eye");
        player.SetTrackingSettings(mirror: false);
        for (var i = 0; i < 60; i++)
        {
            player.TrackFace(20, 0, 0, shapes);
            player.Tick(1 / 60f);
        }
        Check(player.GetParameter("AngleX") > 15, "unmirrored, the model turns the other way");
        player.StopTracking();
        Check(!player.IsTracking, "tracking stops");
    }

    static void LipSyncChecks()
    {
        const int rate = 48000;
        float[] Tone(double hz, double amplitude)
        {
            var s = new float[2048];
            for (var i = 0; i < s.Length; i++) s[i] = (float)(amplitude * Math.Sin(2 * Math.PI * hz * i / rate));
            return s;
        }

        var (level, brightness) = LipSync.Measure(new float[2048], 2048, rate);
        Check(level == 0 && brightness == 0, "silence: no voice");
        (level, brightness) = LipSync.Measure(Tone(1000, 0.5), 2048, rate);
        // 1 kHz: (log2 1000 − log2 300) / (log2 2500 − log2 300) · 2 − 1 ≈ 0.136.
        Check(level == 1 && Math.Abs(brightness - 0.136) < 0.03, $"a loud 1 kHz tone ({level}, {brightness:F3})");
        (level, brightness) = LipSync.Measure(Tone(150, 0.01), 2048, rate);
        // 0.01 amplitude: −43 dB, so (−43 + 50) / 40 ≈ 0.17; 150 Hz is dark.
        Check(Math.Abs(level - 0.17) < 0.02 && brightness == -1, $"a quiet low tone ({level:F3}, {brightness})");
    }
}
