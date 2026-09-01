//! Brush preset import and export.
//!
//! Presets are plain JSON so they can be shared, diffed and hand-edited. A
//! single file may hold one preset or a whole set; both are accepted on load,
//! because an artist downloading "a brush" should not have to know which one
//! they were given.

use aether_core::{AetherError, Result};
use aether_raster::BrushPreset;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Format version written into preset files.
pub const PRESET_VERSION: u32 = 1;

/// The file wrapper: a version plus the presets themselves.
#[derive(Debug, Serialize, Deserialize)]
struct PresetFile {
    version: u32,
    presets: Vec<BrushPreset>,
}

/// Serialise presets to JSON bytes.
pub fn encode_presets(presets: &[BrushPreset]) -> Result<Vec<u8>> {
    let file = PresetFile {
        version: PRESET_VERSION,
        presets: presets.to_vec(),
    };
    serde_json::to_vec_pretty(&file)
        .map_err(|e| AetherError::serialization(format!("could not write brush presets: {e}")))
}

/// Parse presets from JSON bytes.
///
/// Accepts three shapes: the versioned wrapper, a bare array of presets, and a
/// single preset object.
pub fn decode_presets(bytes: &[u8]) -> Result<Vec<BrushPreset>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| AetherError::serialization(format!("brush preset file is not valid JSON: {e}")))?;

    if let Some(version) = value.get("version").and_then(|v| v.as_u64()) {
        if version > PRESET_VERSION as u64 {
            return Err(AetherError::UnsupportedVersion {
                found: version as u32,
                supported: PRESET_VERSION,
            });
        }
        let file: PresetFile = serde_json::from_value(value)
            .map_err(|e| AetherError::serialization(format!("brush preset file is malformed: {e}")))?;
        return Ok(sanitized(file.presets));
    }

    if value.is_array() {
        let presets: Vec<BrushPreset> = serde_json::from_value(value)
            .map_err(|e| AetherError::serialization(format!("brush preset list is malformed: {e}")))?;
        return Ok(sanitized(presets));
    }

    let preset: BrushPreset = serde_json::from_value(value)
        .map_err(|e| AetherError::serialization(format!("brush preset is malformed: {e}")))?;
    Ok(sanitized(vec![preset]))
}

/// Clamp every preset into its legal range after loading.
fn sanitized(mut presets: Vec<BrushPreset>) -> Vec<BrushPreset> {
    for preset in presets.iter_mut() {
        preset.sanitize();
    }
    presets
}

/// Write presets to a file.
pub fn save_presets(presets: &[BrushPreset], path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, encode_presets(presets)?)?;
    Ok(())
}

/// Read presets from a file.
pub fn load_presets(path: impl AsRef<Path>) -> Result<Vec<BrushPreset>> {
    decode_presets(&std::fs::read(path.as_ref())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_raster::brush::{BrushTexture, TexturePattern};

    #[test]
    fn round_trip_preserves_every_builtin() {
        let presets = BrushPreset::builtin();
        let bytes = encode_presets(&presets).expect("encode");
        let back = decode_presets(&bytes).expect("decode");
        assert_eq!(back, presets);
    }

    #[test]
    fn textures_survive_the_round_trip() {
        let preset = BrushPreset {
            name: "Grainy".into(),
            texture: Some(BrushTexture {
                pattern: TexturePattern::Noise { seed: 99 },
                scale: 7.5,
                strength: 0.4,
            }),
            ..Default::default()
        };
        let bytes = encode_presets(std::slice::from_ref(&preset)).expect("encode");
        assert_eq!(decode_presets(&bytes).expect("decode"), vec![preset]);
    }

    #[test]
    fn a_bare_array_of_presets_loads() {
        let presets = vec![BrushPreset::default()];
        let bytes = serde_json::to_vec(&presets).expect("json");
        assert_eq!(decode_presets(&bytes).expect("decode").len(), 1);
    }

    #[test]
    fn a_single_preset_object_loads() {
        let bytes = serde_json::to_vec(&BrushPreset::default()).expect("json");
        assert_eq!(decode_presets(&bytes).expect("decode").len(), 1);
    }

    #[test]
    fn out_of_range_values_are_clamped_on_load() {
        let wild = BrushPreset {
            size: 99_999.0,
            opacity: 4.0,
            ..Default::default()
        };
        let bytes = serde_json::to_vec(&vec![wild]).expect("json");
        let loaded = decode_presets(&bytes).expect("decode");
        assert!(loaded[0].size <= 5000.0);
        assert!(loaded[0].opacity <= 1.0);
    }

    #[test]
    fn a_newer_version_is_refused_clearly() {
        let json = format!(r#"{{"version": {}, "presets": []}}"#, PRESET_VERSION + 1);
        let err = decode_presets(json.as_bytes()).expect_err("must refuse");
        assert!(
            matches!(err, AetherError::UnsupportedVersion { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn garbage_gives_a_readable_error() {
        let err = decode_presets(b"not json").expect_err("must fail");
        assert!(err.to_string().contains("not valid JSON"), "unhelpful: {err}");
    }

    #[test]
    fn saving_and_loading_from_disk_works() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("brushes/mine.json");
        let presets = BrushPreset::builtin();
        save_presets(&presets, &path).expect("save");
        assert_eq!(load_presets(&path).expect("load"), presets);
    }
}
