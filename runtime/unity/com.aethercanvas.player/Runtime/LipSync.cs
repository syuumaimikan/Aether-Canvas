using System;

namespace AetherCanvas
{
    /// <summary>
    /// Loudness and brightness of live audio for <see cref="Player.SetAudio"/>:
    /// the same measures as the editor's WAV analysis and the web player's
    /// <c>lipSync()</c>, over the last 30 ms of samples.
    /// </summary>
    public static class LipSync
    {
        /// <summary>
        /// Measure mono samples (-1..1) at <paramref name="sampleRate"/>. Level is
        /// 0..1 (-50 dB to -10 dB, times <paramref name="gain"/>); brightness is
        /// -1..1 from the zero-crossing rate (300 Hz to 2.5 kHz), 0 when quiet.
        /// </summary>
        public static (float level, float brightness) Measure(float[] samples, int count, int sampleRate, float gain = 1f)
        {
            if (samples == null || count <= 0 || sampleRate <= 0) return (0f, 0f);
            count = Math.Min(count, samples.Length);
            var span = Math.Min(count, Math.Max(1, (int)Math.Round(sampleRate * 0.03)));
            var start = count - span;
            double sum = 0;
            var crossings = 0;
            for (var i = start; i < count; i++)
            {
                sum += samples[i] * samples[i];
                if (i > start && samples[i] >= 0 != samples[i - 1] >= 0) crossings++;
            }
            var rms = Math.Sqrt(sum / span);
            var db = 20 * Math.Log10(Math.Max(rms, 1e-6));
            var level = Clamp((db + 50) / 40 * gain, 0, 1);
            var zcr = crossings * (double)sampleRate / (2.0 * span);
            var bright = (Log2(Math.Max(zcr, 1)) - Log2(300)) / (Log2(2500) - Log2(300)) * 2 - 1;
            return ((float)level, level > 0.05 ? (float)Clamp(bright, -1, 1) : 0f);
        }

        static double Log2(double x) => Math.Log(x) / Math.Log(2);

        static double Clamp(double x, double min, double max) => x < min ? min : x > max ? max : x;
    }
}
