//! Audio analysis for lip sync.
//!
//! Only what lip sync needs: a dependency-free WAV reader (8/16/24/32-bit PCM
//! and 32-bit float, any channel count, down-mixed to mono) and an analyser
//! that turns samples into a per-frame mouth-opening level and a "brightness"
//! value that separates open vowels (a, o) from spread ones (i, e).
//!
//! Brightness comes from the zero-crossing rate, which tracks the dominant
//! frequency region of voiced speech well enough to steer a mouth-form
//! parameter, at a tiny fraction of the cost of a spectral analysis.

use crate::motion::{Easing, Track};
use aether_core::{AetherError, ParameterId, Result};

/// Decoded mono audio.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioClip {
    /// Samples in `-1..=1`.
    pub samples: Vec<f32>,
    /// Samples per second.
    pub sample_rate: u32,
}

impl AudioClip {
    /// Length in seconds.
    pub fn duration(&self) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.samples.len() as f32 / self.sample_rate as f32
        }
    }

    /// Decode a RIFF/WAVE file.
    pub fn from_wav(bytes: &[u8]) -> Result<Self> {
        let bad = |msg: &str| AetherError::UnsupportedFormat(format!("WAV: {msg}"));
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(bad("not a RIFF/WAVE file"));
        }
        let u16_at =
            |i: usize| -> Option<u16> { Some(u16::from_le_bytes([*bytes.get(i)?, *bytes.get(i + 1)?])) };
        let u32_at = |i: usize| -> Option<u32> {
            Some(u32::from_le_bytes([
                *bytes.get(i)?,
                *bytes.get(i + 1)?,
                *bytes.get(i + 2)?,
                *bytes.get(i + 3)?,
            ]))
        };
        let mut pos = 12;
        let mut format: Option<(u16, u16, u32, u16)> = None;
        let mut data: Option<&[u8]> = None;
        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let size = u32_at(pos + 4).ok_or_else(|| bad("truncated chunk"))? as usize;
            let body_start = pos + 8;
            let body_end = body_start.saturating_add(size).min(bytes.len());
            match id {
                b"fmt " => {
                    let tag = u16_at(body_start).ok_or_else(|| bad("truncated fmt"))?;
                    let channels = u16_at(body_start + 2).ok_or_else(|| bad("truncated fmt"))?;
                    let rate = u32_at(body_start + 4).ok_or_else(|| bad("truncated fmt"))?;
                    let bits = u16_at(body_start + 14).ok_or_else(|| bad("truncated fmt"))?;
                    // WAVE_FORMAT_EXTENSIBLE keeps the real tag in the sub-format.
                    let tag = if tag == 0xFFFE {
                        u16_at(body_start + 24).unwrap_or(1)
                    } else {
                        tag
                    };
                    format = Some((tag, channels, rate, bits));
                }
                b"data" => data = Some(&bytes[body_start..body_end]),
                _ => {}
            }
            pos = body_start + size + (size & 1);
        }
        let (tag, channels, rate, bits) = format.ok_or_else(|| bad("missing fmt chunk"))?;
        let data = data.ok_or_else(|| bad("missing data chunk"))?;
        if channels == 0 || rate == 0 {
            return Err(bad("zero channels or sample rate"));
        }
        let bytes_per_sample = (bits as usize).div_ceil(8);
        let frame = bytes_per_sample * channels as usize;
        if frame == 0 {
            return Err(bad("zero-sized samples"));
        }
        let decode = |chunk: &[u8]| -> Option<f32> {
            Some(match (tag, bits) {
                (1, 8) => (chunk[0] as f32 - 128.0) / 128.0,
                (1, 16) => i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0,
                (1, 24) => {
                    let v = i32::from_le_bytes([0, chunk[0], chunk[1], chunk[2]]) >> 8;
                    v as f32 / 8_388_608.0
                }
                (1, 32) => {
                    i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f32 / 2_147_483_648.0
                }
                (3, 32) => f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                _ => return None,
            })
        };
        let mut samples = Vec::with_capacity(data.len() / frame);
        for f in data.chunks_exact(frame) {
            let mut sum = 0.0;
            for c in 0..channels as usize {
                let chunk = &f[c * bytes_per_sample..(c + 1) * bytes_per_sample];
                sum += decode(chunk)
                    .ok_or_else(|| bad(&format!("unsupported encoding (format {tag}, {bits} bits)")))?;
            }
            let v = sum / channels as f32;
            samples.push(if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 });
        }
        Ok(Self {
            samples,
            sample_rate: rate,
        })
    }

    /// Encode as 16-bit mono PCM (used by tests and for exporting).
    pub fn to_wav(&self) -> Vec<u8> {
        let data_len = self.samples.len() * 2;
        let mut out = Vec::with_capacity(44 + data_len);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&(self.sample_rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for s in &self.samples {
            out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        out
    }

    /// Per-frame `(level, brightness)` at `fps` frames per second.
    ///
    /// `level` is a loudness in `0..=1` (RMS mapped through a soft knee so
    /// quiet speech still opens the mouth); `brightness` is in `-1..=1`.
    pub fn analyze(&self, fps: f32) -> Vec<(f32, f32)> {
        let fps = fps.clamp(1.0, 240.0);
        let hop = (self.sample_rate as f32 / fps).max(1.0) as usize;
        let window = hop.max((self.sample_rate as f32 * 0.03) as usize).max(1);
        let frames = (self.samples.len() as f32 / hop as f32).ceil() as usize;
        let mut out = Vec::with_capacity(frames);
        for f in 0..frames {
            let start = f * hop;
            let end = (start + window).min(self.samples.len());
            if start >= end {
                out.push((0.0, 0.0));
                continue;
            }
            let slice = &self.samples[start..end];
            let rms = (slice.iter().map(|s| s * s).sum::<f32>() / slice.len() as f32).sqrt();
            // -50 dB → 0, -10 dB → 1.
            let db = 20.0 * rms.max(1e-6).log10();
            let level = ((db + 50.0) / 40.0).clamp(0.0, 1.0);
            let crossings = slice
                .windows(2)
                .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
                .count();
            let zcr_hz = crossings as f32 * self.sample_rate as f32 / (2.0 * slice.len() as f32);
            // ~300 Hz reads as round ("o"), ~2.5 kHz as spread ("i").
            let brightness =
                ((zcr_hz.max(1.0).log2() - 300f32.log2()) / (2500f32.log2() - 300f32.log2()) * 2.0 - 1.0)
                    .clamp(-1.0, 1.0);
            out.push((level, if level > 0.05 { brightness } else { 0.0 }));
        }
        out
    }

    /// Bake lip sync into motion tracks: mouth opening, and optionally mouth
    /// form. Keys closer to a straight line than `tolerance` are dropped so
    /// the tracks stay editable.
    pub fn bake_lip_sync(
        &self,
        fps: f32,
        mouth_open: ParameterId,
        mouth_form: Option<ParameterId>,
        gain: f32,
        tolerance: f32,
    ) -> Vec<Track> {
        let frames = self.analyze(fps);
        let dt = 1.0 / fps.clamp(1.0, 240.0);
        let mut open = Vec::with_capacity(frames.len());
        let mut form = Vec::with_capacity(frames.len());
        let mut smoothed = 0.0f32;
        for (i, (level, bright)) in frames.iter().enumerate() {
            // Fast attack, slower release: mouths snap open and relax shut.
            let target = (level * gain).clamp(0.0, 1.0);
            let k = if target > smoothed { 0.7 } else { 0.35 };
            smoothed += (target - smoothed) * k;
            open.push((i as f32 * dt, smoothed));
            form.push((i as f32 * dt, *bright));
        }
        let mut tracks = vec![simplify_track(mouth_open, &open, tolerance)];
        if let Some(param) = mouth_form {
            tracks.push(simplify_track(param, &form, tolerance));
        }
        tracks
    }
}

/// Turn dense samples into a track, keeping only keys that matter
/// (Ramer–Douglas–Peucker on the value curve).
pub fn simplify_track(param: ParameterId, samples: &[(f32, f32)], tolerance: f32) -> Track {
    let mut keep = vec![false; samples.len()];
    if !samples.is_empty() {
        keep[0] = true;
        keep[samples.len() - 1] = true;
    }
    let mut stack = vec![(0usize, samples.len().saturating_sub(1))];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (ta, va) = samples[a];
        let (tb, vb) = samples[b];
        let mut worst = (0.0f32, a);
        for (i, &(t, v)) in samples.iter().enumerate().take(b).skip(a + 1) {
            let expected = if tb > ta {
                va + (vb - va) * (t - ta) / (tb - ta)
            } else {
                va
            };
            let err = (v - expected).abs();
            if err > worst.0 {
                worst = (err, i);
            }
        }
        if worst.0 > tolerance {
            keep[worst.1] = true;
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    let mut track = Track::new(param);
    for (i, &(t, v)) in samples.iter().enumerate() {
        if keep[i] {
            let index = track.set_key(t, v);
            track.keys[index].easing = Easing::Linear;
        }
    }
    track
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, seconds: f32, amplitude: f32, rate: u32) -> Vec<f32> {
        (0..(seconds * rate as f32) as usize)
            .map(|i| (i as f32 / rate as f32 * freq * std::f32::consts::TAU).sin() * amplitude)
            .collect()
    }

    #[test]
    fn wav_round_trips() {
        let clip = AudioClip {
            samples: tone(440.0, 0.1, 0.5, 8000),
            sample_rate: 8000,
        };
        let back = AudioClip::from_wav(&clip.to_wav()).expect("decode");
        assert_eq!(back.sample_rate, 8000);
        assert_eq!(back.samples.len(), clip.samples.len());
        for (a, b) in back.samples.iter().zip(&clip.samples) {
            assert!((a - b).abs() < 1e-3);
        }
    }

    #[test]
    fn garbage_is_rejected_with_a_reason() {
        let err = AudioClip::from_wav(b"not audio at all").expect_err("garbage");
        assert!(err.to_string().contains("WAV"), "{err}");
    }

    #[test]
    fn loud_frames_open_the_mouth_and_silence_closes_it() {
        let rate = 16000;
        let mut samples = tone(200.0, 0.5, 0.6, rate);
        samples.extend(vec![0.0; rate as usize / 2]);
        let clip = AudioClip {
            samples,
            sample_rate: rate,
        };
        let frames = clip.analyze(30.0);
        assert!(frames[5].0 > 0.7, "speech frame level {}", frames[5].0);
        assert!(frames[25].0 < 0.05, "silent frame level {}", frames[25].0);
    }

    #[test]
    fn brightness_separates_low_and_high_sounds() {
        let rate = 16000;
        let low = AudioClip {
            samples: tone(250.0, 0.2, 0.5, rate),
            sample_rate: rate,
        };
        let high = AudioClip {
            samples: tone(3000.0, 0.2, 0.5, rate),
            sample_rate: rate,
        };
        let l = low.analyze(30.0)[2].1;
        let h = high.analyze(30.0)[2].1;
        assert!(l < 0.0 && h > 0.5, "low {l}, high {h}");
    }

    #[test]
    fn baking_produces_a_sparse_track() {
        let rate = 16000;
        let mut samples = Vec::new();
        for i in 0..6 {
            let amp = if i % 2 == 0 { 0.6 } else { 0.0 };
            samples.extend(tone(220.0, 0.25, amp, rate));
        }
        let clip = AudioClip {
            samples,
            sample_rate: rate,
        };
        let tracks = clip.bake_lip_sync(30.0, ParameterId(1), Some(ParameterId(2)), 1.0, 0.05);
        assert_eq!(tracks.len(), 2);
        let open = &tracks[0];
        assert!(open.keys.len() > 4, "the mouth opens and closes");
        assert!(open.keys.len() < 45, "keys were simplified: {}", open.keys.len());
        assert!(open.sample(0.12).unwrap_or(0.0) > 0.5);
        assert!(open.sample(0.45).unwrap_or(1.0) < 0.3);
    }
}
