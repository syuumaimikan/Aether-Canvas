//! Live2D Cubism interchange: motions (`.motion3.json`) and expressions
//! (`.exp3.json`), in and out.
//!
//! Aether's standard parameters are Live2D's without the `Param` prefix
//! (`AngleX` ↔ `ParamAngleX`), and expression blend modes share their names,
//! so motions and expressions move between the two directly. Bring motions
//! along when moving a character to Aether — or animate existing Live2D
//! models with Aether's timeline (elastic, bounce and spring keys, lip sync
//! baked from audio) and use the result anywhere Live2D motions play.
//!
//! Curves translate exactly where both sides have the shape: linear,
//! stepped and Bézier segments, and Aether's ease-in/out and back keys (which
//! are cubic). Elastic, bounce and spring keys are exported as runs of Bézier
//! segments that follow the curve to within 0.2% of the move.

use aether_core::{AetherError, ParameterId, Result};
use aether_document::rig::motion::{
    Easing, Expression, ExpressionBlend, ExpressionEntry, Keyframe, Motion, MotionEvent, Track,
};
use aether_document::rig::{Rig, StandardParam};
use serde::{Deserialize, Serialize};

const LINEAR: f32 = 0.0;
const BEZIER: f32 = 1.0;
const STEPPED: f32 = 2.0;
const INVERSE_STEPPED: f32 = 3.0;

/// Gap used to express an inverse-stepped segment (a jump at its start).
const JUMP: f32 = 1e-3;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Motion3 {
    version: u32,
    meta: Meta3,
    curves: Vec<Curve3>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    user_data: Vec<UserData3>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Meta3 {
    duration: f32,
    fps: f32,
    #[serde(default, rename = "Loop")]
    looping: bool,
    #[serde(default)]
    are_beziers_restricted: bool,
    #[serde(default)]
    curve_count: usize,
    #[serde(default)]
    total_segment_count: usize,
    #[serde(default)]
    total_point_count: usize,
    #[serde(default)]
    user_data_count: usize,
    #[serde(default)]
    total_user_data_size: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fade_in_time: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fade_out_time: Option<f32>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Curve3 {
    target: String,
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fade_in_time: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fade_out_time: Option<f32>,
    segments: Vec<f32>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UserData3 {
    time: f32,
    value: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Expression3 {
    #[serde(rename = "Type")]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fade_in_time: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fade_out_time: Option<f32>,
    #[serde(default)]
    parameters: Vec<ExpressionParameter3>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ExpressionParameter3 {
    id: String,
    value: f32,
    #[serde(default = "default_blend")]
    blend: String,
}

fn default_blend() -> String {
    "Add".into()
}

/// The Live2D id for a parameter: standard parameters gain the `Param`
/// prefix, everything else keeps its name.
pub fn live2d_id(name: &str) -> String {
    if StandardParam::named(name).is_some() {
        format!("Param{name}")
    } else {
        name.to_string()
    }
}

/// The parameter a Live2D id refers to: an exact name, the name without
/// `Param`, or the same ignoring case.
pub fn resolve_id(rig: &Rig, id: &str) -> Option<ParameterId> {
    let stripped = match id.get(..5) {
        Some(prefix) if prefix.eq_ignore_ascii_case("param") && id.len() > 5 => &id[5..],
        _ => id,
    };
    rig.parameter_named(id)
        .or_else(|| rig.parameter_named(stripped))
        .or_else(|| {
            rig.parameters
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(id) || p.name.eq_ignore_ascii_case(stripped))
        })
        .map(|p| p.id)
}

fn bad(message: impl std::fmt::Display) -> AetherError {
    AetherError::serialization(format!("not a Live2D file: {message}"))
}

// ---------------------------------------------------------------- motions

/// Read a `.motion3.json` into a motion for `rig`. Returns the motion and
/// notes on anything left out (curves for parameters the rig lacks, part
/// opacity and model-level curves).
pub fn import_motion(json: &str, rig: &Rig, name: &str) -> Result<(Motion, Vec<String>)> {
    let file: Motion3 = serde_json::from_str(json).map_err(bad)?;
    let mut notes = Vec::new();
    let mut motion = Motion::new(name, file.meta.duration.max(0.01), file.meta.fps.max(1.0));
    motion.looping = file.meta.looping;
    if let Some(fade) = file.meta.fade_in_time.filter(|f| f.is_finite() && *f >= 0.0) {
        motion.fade_in = fade;
    }
    if let Some(fade) = file.meta.fade_out_time.filter(|f| f.is_finite() && *f >= 0.0) {
        motion.fade_out = fade;
    }
    for curve in &file.curves {
        // Part opacity curves drive a part parameter of the same name, when
        // the rig has one (an imported Live2D model's switched parts).
        let part = curve.target == "PartOpacity" && rig.parameter_named(&curve.id).is_some();
        if curve.target != "Parameter" && !part {
            notes.push(format!(
                "{} curve {:?} left out (no equivalent)",
                curve.target, curve.id
            ));
            continue;
        }
        let resolved = if part {
            rig.parameter_named(&curve.id).map(|p| p.id)
        } else {
            resolve_id(rig, &curve.id)
        };
        let Some(param) = resolved else {
            notes.push(format!("curve for {:?} left out (no such parameter)", curve.id));
            continue;
        };
        let keys = parse_segments(&curve.segments, file.meta.are_beziers_restricted)
            .map_err(|e| bad(format!("curve {:?}: {e}", curve.id)))?;
        let mut track = Track::new(param);
        track.keys = keys;
        motion.tracks.retain(|t| t.param != param);
        motion.tracks.push(track);
    }
    motion.events = file
        .user_data
        .into_iter()
        .filter(|u| u.time.is_finite())
        .map(|u| MotionEvent {
            time: u.time,
            name: u.value,
        })
        .collect();
    Ok((motion, notes))
}

/// Keys for one curve's segment list.
fn parse_segments(segments: &[f32], restricted: bool) -> std::result::Result<Vec<Keyframe>, String> {
    if segments.len() < 2 || segments.iter().any(|v| !v.is_finite()) {
        return Err("no points".into());
    }
    let key = |time: f32, value: f32| Keyframe {
        time,
        value,
        easing: Easing::Linear,
    };
    let mut keys = vec![key(segments[0], segments[1])];
    let mut i = 2;
    while i < segments.len() {
        let kind = segments[i];
        let (t0, v0) = {
            let last = keys.last().expect("a first key");
            (last.time, last.value)
        };
        let point = |k: usize| -> std::result::Result<(f32, f32), String> {
            match (segments.get(i + 1 + 2 * k), segments.get(i + 2 + 2 * k)) {
                (Some(&t), Some(&v)) => Ok((t, v)),
                _ => Err("a segment is cut short".into()),
            }
        };
        if kind == LINEAR || kind == STEPPED {
            let (t1, v1) = point(0)?;
            keys.last_mut().expect("a key").easing = if kind == STEPPED {
                Easing::Step
            } else {
                Easing::Linear
            };
            keys.push(key(t1, v1));
            i += 3;
        } else if kind == INVERSE_STEPPED {
            // The value jumps at the start of the segment and holds.
            let (t1, v1) = point(0)?;
            keys.last_mut().expect("a key").easing = Easing::Step;
            if t1 - t0 > 2.0 * JUMP {
                keys.push(Keyframe {
                    time: t0 + JUMP,
                    value: v1,
                    easing: Easing::Step,
                });
            }
            keys.push(key(t1, v1));
            i += 3;
        } else if kind == BEZIER {
            let (c1t, c1v) = point(0)?;
            let (c2t, c2v) = point(1)?;
            let (t1, v1) = point(2)?;
            let (dt, dv) = (t1 - t0, v1 - v0);
            // Restricted Béziers take the time fraction as the curve
            // parameter; the others solve the time curve (as ours does).
            let (x1, x2) = if restricted || dt <= 0.0 {
                (1.0 / 3.0, 2.0 / 3.0)
            } else {
                (
                    ((c1t - t0) / dt).clamp(0.0, 1.0),
                    ((c2t - t0) / dt).clamp(0.0, 1.0),
                )
            };
            if dv.abs() > 1e-6 {
                keys.last_mut().expect("a key").easing = Easing::Bezier {
                    x1,
                    y1: (c1v - v0) / dv,
                    x2,
                    y2: (c2v - v0) / dv,
                };
            } else if (c1v - v0).abs() > 1e-6 || (c2v - v0).abs() > 1e-6 {
                // A bump that returns to where it started has no value
                // fraction to scale: follow it with short straight pieces.
                let steps = 8;
                for s in 1..steps {
                    let u = s as f32 / steps as f32;
                    let x = bezier1(0.0, x1, x2, 1.0, u);
                    keys.push(key(t0 + x * dt, bezier1(v0, c1v, c2v, v1, u)));
                }
            }
            keys.push(key(t1, v1));
            i += 7;
        } else {
            return Err(format!("unknown segment type {kind}"));
        }
    }
    // Times must rise; drop anything that does not.
    let mut out: Vec<Keyframe> = Vec::with_capacity(keys.len());
    for k in keys {
        if out.last().is_none_or(|last| k.time > last.time + 1e-6) {
            out.push(k);
        }
    }
    Ok(out)
}

fn bezier1(p0: f32, p1: f32, p2: f32, p3: f32, s: f32) -> f32 {
    let u = 1.0 - s;
    u * u * u * p0 + 3.0 * u * u * s * p1 + 3.0 * u * s * s * p2 + s * s * s * p3
}

/// Write a motion as `.motion3.json`, for a rig's parameters.
pub fn export_motion(motion: &Motion, rig: &Rig) -> String {
    let mut curves = Vec::new();
    let (mut segment_count, mut point_count) = (0usize, 0usize);
    for track in motion.tracks.iter().filter(|t| t.enabled && !t.keys.is_empty()) {
        let Some(param) = rig.parameter(track.param) else {
            continue;
        };
        let first = &track.keys[0];
        let mut segments = vec![first.time, first.value];
        point_count += 1;
        for pair in track.keys.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            for piece in curve_pieces(a, b) {
                segment_count += 1;
                point_count += if piece[0] == BEZIER { 3 } else { 1 };
                segments.extend_from_slice(&piece);
            }
        }
        curves.push(Curve3 {
            target: "Parameter".into(),
            id: live2d_id(&param.name),
            fade_in_time: None,
            fade_out_time: None,
            segments,
        });
    }
    let user_data: Vec<UserData3> = motion
        .events
        .iter()
        .map(|e| UserData3 {
            time: e.time,
            value: e.name.clone(),
        })
        .collect();
    let file = Motion3 {
        version: 3,
        meta: Meta3 {
            duration: motion.duration,
            fps: motion.fps,
            looping: motion.looping,
            are_beziers_restricted: false,
            curve_count: curves.len(),
            total_segment_count: segment_count,
            total_point_count: point_count,
            user_data_count: user_data.len(),
            total_user_data_size: user_data.iter().map(|u| u.value.len() + 1).sum(),
            fade_in_time: Some(motion.fade_in),
            fade_out_time: Some(motion.fade_out),
        },
        curves,
        user_data,
    };
    serde_json::to_string_pretty(&file).expect("motions serialise")
}

/// Live2D segments (type followed by points) for the curve from `a` to `b`.
fn curve_pieces(a: &Keyframe, b: &Keyframe) -> Vec<Vec<f32>> {
    let (t0, v0, t1, v1) = (a.time, a.value, b.time, b.value);
    let (dt, dv) = (t1 - t0, v1 - v0);
    match a.easing {
        Easing::Step => return vec![vec![STEPPED, t1, v1]],
        Easing::Linear => return vec![vec![LINEAR, t1, v1]],
        Easing::Bezier { x1, y1, x2, y2 } => {
            return vec![vec![
                BEZIER,
                t0 + x1.clamp(0.0, 1.0) * dt,
                v0 + y1 * dv,
                t0 + x2.clamp(0.0, 1.0) * dt,
                v0 + y2 * dv,
                t1,
                v1,
            ]]
        }
        _ => {}
    }
    if dv.abs() < 1e-9 {
        return vec![vec![LINEAR, t1, v1]];
    }
    // Everything else: cubic pieces fitted to the easing, split until each
    // follows it closely. Ease in/out and back keys are cubic and come out
    // exact in one or two pieces. With its handles at a third and two thirds
    // of the piece's time, a Bézier's time is linear in its parameter, so its
    // value is exactly the fitted cubic.
    let mut pieces = Vec::new();
    fit(&a.easing, 0.0, 1.0, 0, &mut |s0, s1, b1, b2| {
        let (ta, tb) = (t0 + s0 * dt, t0 + s1 * dt);
        let (tb, vb) = if s1 >= 1.0 {
            (t1, v1)
        } else {
            (tb, v0 + a.easing.apply(s1) * dv)
        };
        pieces.push(vec![
            BEZIER,
            ta + (tb - ta) / 3.0,
            v0 + b1 * dv,
            ta + 2.0 * (tb - ta) / 3.0,
            v0 + b2 * dv,
            tb,
            vb,
        ]);
    });
    pieces
}

/// Fit easing `f` on `[s0, s1]` with a cubic, splitting in half until the
/// fit is within 0.2% of the move. Calls `emit(s0, s1, b1, b2)` with the
/// cubic's inner Bernstein coefficients, as fractions of the whole move.
fn fit(f: &Easing, s0: f32, s1: f32, depth: u32, emit: &mut dyn FnMut(f32, f32, f32, f32)) {
    let p = |u: f32| f.apply(s0 + (s1 - s0) * u);
    let (p0, p1, p2, p3) = (p(0.0), p(1.0 / 3.0), p(2.0 / 3.0), p(1.0));
    // The cubic through the four samples: at u = 1/3 and 2/3 its Bernstein
    // form gives 12·b1 + 6·b2 = a and 6·b1 + 12·b2 = b.
    let a = 27.0 * p1 - 8.0 * p0 - p3;
    let b = 27.0 * p2 - p0 - 8.0 * p3;
    let b1 = (2.0 * a - b) / 18.0;
    let b2 = (2.0 * b - a) / 18.0;
    let worst = (1..32)
        .map(|k| {
            let u = k as f32 / 32.0;
            (bezier1(p0, b1, b2, p3, u) - p(u)).abs()
        })
        .fold(0.0f32, f32::max);
    // Kinks (a bounce landing) only fit once the pieces are small.
    if worst <= 0.002 || depth >= 9 {
        emit(s0, s1, b1, b2);
    } else {
        let mid = (s0 + s1) * 0.5;
        fit(f, s0, mid, depth + 1, emit);
        fit(f, mid, s1, depth + 1, emit);
    }
}

// ------------------------------------------------------------ expressions

/// Read an `.exp3.json` into an expression for `rig`, with notes on entries
/// for parameters the rig lacks.
pub fn import_expression(json: &str, rig: &Rig, name: &str) -> Result<(Expression, Vec<String>)> {
    let file: Expression3 = serde_json::from_str(json).map_err(bad)?;
    if file.kind != "Live2D Expression" {
        return Err(bad(format!("type {:?}", file.kind)));
    }
    let mut notes = Vec::new();
    let mut entries = Vec::new();
    for p in &file.parameters {
        let Some(param) = resolve_id(rig, &p.id) else {
            notes.push(format!("{:?} left out (no such parameter)", p.id));
            continue;
        };
        let blend = match p.blend.as_str() {
            "Add" => ExpressionBlend::Add,
            "Multiply" => ExpressionBlend::Multiply,
            "Overwrite" => ExpressionBlend::Overwrite,
            other => {
                notes.push(format!("{:?} has unknown blend {other:?}; added instead", p.id));
                ExpressionBlend::Add
            }
        };
        if p.value.is_finite() {
            entries.push(ExpressionEntry {
                param,
                value: p.value,
                blend,
            });
        }
    }
    let fade = file
        .fade_in_time
        .filter(|f| f.is_finite() && *f >= 0.0)
        .unwrap_or(1.0);
    Ok((
        Expression {
            name: name.to_string(),
            entries,
            fade,
        },
        notes,
    ))
}

/// Write an expression as `.exp3.json`.
pub fn export_expression(expression: &Expression, rig: &Rig) -> String {
    let file = Expression3 {
        kind: "Live2D Expression".into(),
        fade_in_time: Some(expression.fade),
        fade_out_time: Some(expression.fade),
        parameters: expression
            .entries
            .iter()
            .filter_map(|e| {
                let param = rig.parameter(e.param)?;
                Some(ExpressionParameter3 {
                    id: live2d_id(&param.name),
                    value: e.value,
                    blend: match e.blend {
                        ExpressionBlend::Add => "Add",
                        ExpressionBlend::Multiply => "Multiply",
                        ExpressionBlend::Overwrite => "Overwrite",
                    }
                    .into(),
                })
            })
            .collect(),
    };
    serde_json::to_string_pretty(&file).expect("expressions serialise")
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::id::IdGenerator;

    fn rig() -> Rig {
        let mut rig = Rig::default();
        rig.add_standard_parameters(&IdGenerator::new());
        rig
    }

    fn sample(motion: &Motion, rig: &Rig, name: &str, time: f32) -> f32 {
        let id = rig.parameter_named(name).unwrap().id;
        motion.track(id).unwrap().sample(time).unwrap()
    }

    /// Every segment type, evaluated the way the Cubism SDK does.
    const MOTION: &str = r#"{
      "Version": 3,
      "Meta": { "Duration": 4.0, "Fps": 30.0, "Loop": true, "AreBeziersRestricted": true,
                "CurveCount": 4, "TotalSegmentCount": 5, "TotalPointCount": 9,
                "UserDataCount": 1, "TotalUserDataSize": 5, "FadeInTime": 0.5, "FadeOutTime": 0.7 },
      "Curves": [
        { "Target": "Parameter", "Id": "ParamAngleX",
          "Segments": [0, 0,  0, 1, 30,  1, 1.333, 30, 1.667, -30, 2, -30,  2, 3, 10,  3, 4, 0] },
        { "Target": "Parameter", "Id": "ParamEyeLOpen", "Segments": [0, 1, 0, 4, 1] },
        { "Target": "Parameter", "Id": "ParamNoSuchThing", "Segments": [0, 0, 0, 1, 1] },
        { "Target": "PartOpacity", "Id": "PartArmA", "Segments": [0, 1, 0, 4, 1] }
      ],
      "UserData": [ { "Time": 1.5, "Value": "wave" } ]
    }"#;

    #[test]
    fn a_live2d_motion_imports_with_its_curves_intact() {
        let rig = rig();
        let (motion, notes) = import_motion(MOTION, &rig, "Greeting").expect("import");
        assert_eq!(motion.name, "Greeting");
        assert_eq!((motion.duration, motion.fps, motion.looping), (4.0, 30.0, true));
        assert_eq!((motion.fade_in, motion.fade_out), (0.5, 0.7));
        assert_eq!(motion.tracks.len(), 2);
        assert_eq!(motion.events.len(), 1);
        assert_eq!(motion.events[0].name, "wave");
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("ParamNoSuchThing")));
        assert!(notes.iter().any(|n| n.contains("PartOpacity")));

        let at = |t: f32| sample(&motion, &rig, "AngleX", t);
        // Linear.
        assert!((at(0.5) - 15.0).abs() < 1e-3);
        // A restricted Bézier takes the time fraction as its parameter.
        assert!(at(1.5).abs() < 1e-3, "{}", at(1.5));
        assert!((at(1.25) - 20.625).abs() < 0.05, "{}", at(1.25));
        // Stepped holds, then jumps at the end.
        assert!((at(2.5) + 30.0).abs() < 1e-3);
        assert!((at(3.0) - 10.0).abs() < 1e-3);
        // Inverse stepped jumps at the start, then holds.
        assert!(at(3.5).abs() < 1e-3);
        assert!((sample(&motion, &rig, "EyeLOpen", 2.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn every_easing_survives_a_round_trip() {
        let rig = rig();
        let id = rig.parameter_named("AngleX").unwrap().id;
        let easings = [
            Easing::Step,
            Easing::Linear,
            Easing::Bezier {
                x1: 0.2,
                y1: -0.3,
                x2: 0.7,
                y2: 1.4,
            },
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::BackIn,
            Easing::BackOut,
            Easing::ElasticOut,
            Easing::BounceOut,
            Easing::Spring {
                frequency: 2.5,
                damping: 0.35,
            },
        ];
        for easing in easings {
            let mut motion = Motion::new("m", 2.0, 30.0);
            let track = motion.track_mut(id);
            track.keys = vec![
                Keyframe {
                    time: 0.0,
                    value: -20.0,
                    easing,
                },
                Keyframe {
                    time: 2.0,
                    value: 25.0,
                    easing: Easing::Linear,
                },
            ];
            motion.events.push(MotionEvent {
                time: 1.0,
                name: "cue".into(),
            });
            let json = export_motion(&motion, &rig);
            let (back, notes) = import_motion(&json, &rig, "m").expect("re-import");
            assert!(notes.is_empty(), "{notes:?}");
            assert_eq!(back.events, motion.events);
            let exact = !matches!(
                easing,
                Easing::ElasticOut | Easing::BounceOut | Easing::Spring { .. }
            );
            let tolerance = if exact { 0.01 } else { 0.002 * 45.0 + 0.01 };
            let worst = (0..=400)
                .map(|k| {
                    let t = k as f32 / 200.0;
                    // Skip the instant of a step, where either side is right.
                    if matches!(easing, Easing::Step) && (t - 2.0).abs() < 0.02 {
                        return 0.0;
                    }
                    (sample(&back, &rig, "AngleX", t) - sample(&motion, &rig, "AngleX", t)).abs()
                })
                .fold(0.0f32, f32::max);
            assert!(worst <= tolerance, "{easing:?}: off by {worst}");
        }
    }

    #[test]
    fn exported_motions_are_well_formed() {
        let rig = rig();
        let mut motion = Motion::new("m", 3.0, 24.0);
        for name in ["AngleX", "MouthOpenY"] {
            let id = rig.parameter_named(name).unwrap().id;
            let track = motion.track_mut(id);
            track.set_key(0.0, 0.0);
            track.set_key(1.0, 1.0);
            track.keys[0].easing = Easing::BounceOut;
            track.set_key(3.0, 0.0);
        }
        let json = export_motion(&motion, &rig);
        let file: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(file["Version"], 3);
        assert_eq!(file["Curves"][0]["Id"], "ParamAngleX");
        assert_eq!(file["Curves"][1]["Id"], "ParamMouthOpenY");
        // The counts in Meta match the curves, as the Cubism SDK expects.
        let (mut segments, mut points) = (0, 0);
        for curve in file["Curves"].as_array().unwrap() {
            let s: Vec<f64> = curve["Segments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            points += 1;
            let mut i = 2;
            while i < s.len() {
                segments += 1;
                let (n, step) = if s[i] == 1.0 { (3, 7) } else { (1, 3) };
                points += n;
                i += step;
            }
            assert_eq!(i, s.len());
        }
        assert_eq!(file["Meta"]["CurveCount"], 2);
        assert_eq!(file["Meta"]["TotalSegmentCount"], segments);
        assert_eq!(file["Meta"]["TotalPointCount"], points);
        assert_eq!(file["Meta"]["AreBeziersRestricted"], false);
    }

    #[test]
    fn expressions_translate_both_ways() {
        let rig = rig();
        let json = r#"{ "Type": "Live2D Expression", "FadeInTime": 0.4,
            "Parameters": [
              { "Id": "ParamEyeLSmile", "Value": 1.0, "Blend": "Add" },
              { "Id": "ParamMouthForm", "Value": 0.5, "Blend": "Overwrite" },
              { "Id": "ParamEyeLOpen", "Value": 0.6, "Blend": "Multiply" },
              { "Id": "ParamMissing", "Value": 1.0 }
            ] }"#;
        let (expression, notes) = import_expression(json, &rig, "Smile").expect("import");
        assert_eq!(expression.fade, 0.4);
        assert_eq!(expression.entries.len(), 3);
        assert_eq!(expression.entries[2].blend, ExpressionBlend::Multiply);
        assert_eq!(notes.len(), 1);
        let back = import_expression(&export_expression(&expression, &rig), &rig, "Smile")
            .unwrap()
            .0;
        assert_eq!(back, expression);
        assert!(import_expression(r#"{"Type":"Something else"}"#, &rig, "x").is_err());
        assert!(import_motion("[]", &rig, "x").is_err());
    }

    #[test]
    fn ids_map_between_the_two_namings() {
        let mut rig = rig();
        rig.add_parameter(aether_document::rig::Parameter::new(
            ParameterId(900),
            "SkirtSwing",
            -1.0,
            1.0,
            0.0,
        ))
        .unwrap();
        assert_eq!(live2d_id("AngleX"), "ParamAngleX");
        assert_eq!(live2d_id("SkirtSwing"), "SkirtSwing");
        let skirt = rig.parameter_named("SkirtSwing").unwrap().id;
        assert_eq!(resolve_id(&rig, "SkirtSwing"), Some(skirt));
        assert_eq!(resolve_id(&rig, "ParamSkirtSwing"), Some(skirt));
        assert_eq!(
            resolve_id(&rig, "paramanglex"),
            rig.parameter_named("AngleX").map(|p| p.id)
        );
        assert_eq!(resolve_id(&rig, "Nope"), None);
    }
}
