//! Physics as `physics3.json`.
//!
//! Aether and Cubism both hang chains of particles from an anchor that the
//! input parameters move, but they integrate them differently, so the
//! settings cannot be copied across. Each chain is translated structurally
//! (the same inputs, particles and outputs) and then *fitted*: Aether's
//! simulation and Cubism's (`aether_live2d::Physics`, which follows the
//! Cubism Framework step for step) are run on the same input motions —
//! each input stepped on its own, then everything swaying — and Cubism's
//! delay, mobility, acceleration, output gain and directions are chosen to
//! make the outputs match.

use aether_core::ParameterId;
use aether_document::rig::physics::{InputKind, OutputKind, PhysicsGroup, PhysicsRuntime};
use aether_document::rig::{ParamValues, Parameter, Rig};
use aether_live2d::physics::{Parameters, Physics};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// The Cubism step rate written to the file (Cubism Editor's).
const FPS: f64 = 30.0;
/// Rate the fit plays at.
const FRAME: f32 = 1.0 / 60.0;
/// Cubism's output is radians × scale; Aether reads 45° as full range.
const RADIANS_TO_FULL: f32 = 180.0 / std::f32::consts::PI / 45.0;

#[derive(Clone, Debug, PartialEq)]
struct Fit {
    delay: f64,
    mobility: f64,
    acceleration: f64,
    gain: f64,
    /// Scales the input normalisation ranges (translation, angle).
    reach: (f32, f32),
    input_reflect: Vec<bool>,
    output_reflect: Vec<bool>,
    /// The Cubism vertex each output reads (Cubism measures the first
    /// against straight down and the rest against the segment above, so
    /// the best match for an Aether particle is not always itself).
    output_vertex: Vec<usize>,
}

fn half_range(p: &Parameter) -> f32 {
    (p.max - p.default).max(p.default - p.min).max(1e-6)
}

fn setting(
    group: &PhysicsGroup,
    index: usize,
    rig: &Rig,
    ids: &BTreeMap<ParameterId, String>,
    fit: &Fit,
) -> Value {
    let inputs: Vec<Value> = group
        .inputs
        .iter()
        .zip(&fit.input_reflect)
        .filter_map(|(input, &reflect)| {
            let id = ids.get(&input.param)?;
            Some(json!({
                "Source": { "Target": "Parameter", "Id": id },
                "Weight": (input.weight.abs() * 100.0).clamp(0.0, 100.0),
                "Type": match input.kind {
                    InputKind::X => "X",
                    InputKind::Y => "Y",
                    InputKind::Angle => "Angle",
                },
                "Reflect": reflect,
            }))
        })
        .collect();
    let outputs: Vec<Value> = group
        .outputs
        .iter()
        .zip(&fit.output_reflect)
        .zip(&fit.output_vertex)
        .filter_map(|((output, &reflect), &vertex)| {
            let id = ids.get(&output.param)?;
            let p = rig.parameter(output.param)?;
            let scale = fit.gain * (RADIANS_TO_FULL * output.scale * half_range(p)) as f64;
            Some(json!({
                "Destination": { "Target": "Parameter", "Id": id },
                "VertexIndex": vertex.clamp(1, group.particles.len()),
                "Scale": scale,
                "Weight": 100,
                "Type": "Angle",
                "Reflect": reflect,
            }))
        })
        .collect();
    let mut vertices = vec![json!({
        "Position": { "X": 0, "Y": 0 },
        "Mobility": 1, "Delay": 1, "Acceleration": 1, "Radius": 0,
    })];
    let mut y = 0.0f32;
    for particle in &group.particles {
        let length = particle.length.abs().max(1e-3);
        y += length;
        vertices.push(json!({
            "Position": { "X": 0, "Y": y },
            "Mobility": fit.mobility,
            "Delay": fit.delay,
            "Acceleration": fit.acceleration,
            "Radius": length,
        }));
    }
    let t = group.translation_range.abs().max(1e-3) * fit.reach.0;
    let a = group.angle_range.abs().max(1e-3) * fit.reach.1;
    json!({
        "Id": format!("PhysicsSetting{}", index + 1),
        "Input": inputs,
        "Output": outputs,
        "Vertices": vertices,
        "Normalization": {
            "Position": { "Minimum": -t, "Default": 0, "Maximum": t },
            "Angle": { "Minimum": -a, "Default": 0, "Maximum": a },
        },
    })
}

fn file(settings: Vec<(String, Value)>) -> Value {
    let count = |key: &str| -> usize {
        settings
            .iter()
            .map(|(_, s)| s[key].as_array().map(Vec::len).unwrap_or(0))
            .sum()
    };
    let dictionary: Vec<Value> = settings
        .iter()
        .map(|(name, s)| json!({ "Id": s["Id"], "Name": name }))
        .collect();
    json!({
        "Version": 3,
        "Meta": {
            "PhysicsSettingCount": settings.len(),
            "TotalInputCount": count("Input"),
            "TotalOutputCount": count("Output"),
            "VertexCount": count("Vertices"),
            "Fps": FPS,
            "EffectiveForces": {
                "Gravity": { "X": 0, "Y": -1 },
                "Wind": { "X": 0, "Y": 0 },
            },
            "PhysicsDictionary": dictionary,
        },
        "PhysicsSettings": settings.into_iter().map(|(_, s)| s).collect::<Vec<_>>(),
    })
}

/// The input motion both simulations are played: a moment at rest, each
/// input stepped on its own and released, then all inputs swaying.
fn schedule(group: &PhysicsGroup, rig: &Rig) -> Vec<ParamValues> {
    let n = group.inputs.len();
    let step = 72; // 1.2 s at 60 fps
    let rest = 48;
    let sway = 150;
    let mut frames = Vec::with_capacity(rest + n * (step + rest) + sway);
    for _ in 0..rest / 2 {
        frames.push(ParamValues::new());
    }
    let value = |i: usize, amount: f32| -> Option<(ParameterId, f32)> {
        let input = group.inputs.get(i)?;
        let p = rig.parameter(input.param)?;
        let side = if amount >= 0.0 {
            p.max - p.default
        } else {
            p.default - p.min
        };
        Some((p.id, p.default + amount * side))
    };
    for i in 0..n {
        for _ in 0..step {
            frames.push(value(i, 0.7).into_iter().collect());
        }
        for _ in 0..rest {
            frames.push(ParamValues::new());
        }
    }
    for f in 0..sway {
        let t = f as f32 * FRAME;
        frames.push(
            (0..n)
                .filter_map(|i| value(i, 0.5 * (t * 7.5 + i as f32 * 1.3).sin()))
                .collect(),
        );
    }
    frames
}

fn ours(group: &PhysicsGroup, rig: &Rig, frames: &[ParamValues]) -> Vec<Vec<f32>> {
    let mut runtime = PhysicsRuntime::new();
    let groups = [group.clone()];
    frames
        .iter()
        .map(|inputs| {
            let mut values = inputs.clone();
            runtime.update(&groups, &rig.parameters, &mut values, FRAME);
            group
                .outputs
                .iter()
                .map(|o| {
                    let p = rig.parameter(o.param);
                    values
                        .get(&o.param)
                        .copied()
                        .or(p.map(|p| p.default))
                        .unwrap_or(0.0)
                })
                .collect()
        })
        .collect()
}

fn theirs(
    json: &Value,
    group: &PhysicsGroup,
    rig: &Rig,
    ids: &BTreeMap<ParameterId, String>,
    frames: &[ParamValues],
) -> Option<Vec<Vec<f32>>> {
    let mut physics = Physics::from_json(json)?;
    let order: Vec<&Parameter> = rig
        .parameters
        .iter()
        .filter(|p| ids.contains_key(&p.id))
        .collect();
    let names: Vec<String> = order.iter().map(|p| ids[&p.id].clone()).collect();
    physics.bind(&names);
    let min: Vec<f32> = order.iter().map(|p| p.min).collect();
    let max: Vec<f32> = order.iter().map(|p| p.max).collect();
    let default: Vec<f32> = order.iter().map(|p| p.default).collect();
    let slot: BTreeMap<ParameterId, usize> = order.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
    let mut values = default.clone();
    {
        let mut p = Parameters {
            values: &mut values,
            min: &min,
            max: &max,
            default: &default,
        };
        physics.stabilize(&mut p);
    }
    let mut out = Vec::with_capacity(frames.len());
    for inputs in frames {
        values.copy_from_slice(&default);
        for (id, &v) in inputs {
            if let Some(&i) = slot.get(id) {
                values[i] = v;
            }
        }
        let mut p = Parameters {
            values: &mut values,
            min: &min,
            max: &max,
            default: &default,
        };
        physics.evaluate(&mut p, FRAME as f64);
        out.push(
            group
                .outputs
                .iter()
                .map(|o| slot.get(&o.param).map(|&i| values[i]).unwrap_or(0.0))
                .collect(),
        );
    }
    Some(out)
}

/// Mean squared difference, in units of each output's half range.
fn score(a: &[Vec<f32>], b: &[Vec<f32>], group: &PhysicsGroup, rig: &Rig) -> f64 {
    let ranges: Vec<f32> = group
        .outputs
        .iter()
        .map(|o| rig.parameter(o.param).map(half_range).unwrap_or(1.0))
        .collect();
    let mut sum = 0.0f64;
    let mut n = 0usize;
    for (fa, fb) in a.iter().zip(b) {
        for ((x, y), r) in fa.iter().zip(fb).zip(&ranges) {
            let d = ((x - y) / r) as f64;
            sum += d * d;
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}

/// Every combination of `n` choices from `0..k`, capped at `limit`
/// combinations (the rest keep their first choice).
fn choices(n: usize, k: usize, limit: usize) -> Vec<Vec<usize>> {
    let mut out = vec![vec![0; n]];
    for i in 0..n {
        let mut next = Vec::new();
        for c in &out {
            for v in 0..k {
                if v > 0 && next.len() >= limit {
                    break;
                }
                let mut c = c.clone();
                c[i] = v;
                next.push(c);
            }
        }
        out = next;
    }
    out
}

fn fit_group(
    group: &PhysicsGroup,
    index: usize,
    rig: &Rig,
    ids: &BTreeMap<ParameterId, String>,
) -> (Fit, f64) {
    let frames = schedule(group, rig);
    let target = ours(group, rig, &frames);
    let outputs = group.outputs.len();
    let ranges: Vec<f32> = group
        .outputs
        .iter()
        .map(|o| rig.parameter(o.param).map(half_range).unwrap_or(1.0))
        .collect();
    // Simulate `fit` with unit gain and no output reflection, then pick
    // the best gain (and so direction) for each output in closed form;
    // one gain is shared, directions are per output.
    let solve = |fit: &Fit| -> (Fit, f64) {
        let mut unit = fit.clone();
        unit.gain = 1.0;
        unit.output_reflect = vec![false; outputs];
        let json = file(vec![(group.name.clone(), setting(group, index, rig, ids, &unit))]);
        let Some(series) = theirs(&json, group, rig, ids, &frames) else {
            return (fit.clone(), f64::INFINITY);
        };
        let mut reflect = vec![false; outputs];
        let (mut num, mut den) = (0.0f64, 0.0f64);
        for k in 0..outputs {
            let r = ranges[k] as f64;
            let (mut n, mut d) = (0.0f64, 0.0f64);
            for (o, c) in target.iter().zip(&series) {
                n += (o[k] as f64 / r) * (c[k] as f64 / r);
                d += (c[k] as f64 / r).powi(2);
            }
            reflect[k] = n < 0.0;
            num += n.abs();
            den += d;
        }
        let gain = if den > 1e-12 {
            (num / den).clamp(0.05, 20.0)
        } else {
            1.0
        };
        let mut best = fit.clone();
        best.gain = gain;
        best.output_reflect = reflect;
        // Score the real thing (outputs clamp to their ranges).
        let json = file(vec![(group.name.clone(), setting(group, index, rig, ids, &best))]);
        let score = theirs(&json, group, rig, ids, &frames)
            .map(|t| score(&target, &t, group, rig))
            .unwrap_or(f64::INFINITY);
        (best, score)
    };

    // Start from Cubism settings that swing at the same rate: per 30 Hz
    // step Cubism accelerates by acceleration × delay², and loses
    // velocity by mobility.
    let particle = &group.particles[0];
    let g = group.gravity.length().max(1.0) as f64;
    let delay = 0.7;
    let acceleration = (g / (FPS * FPS) / (delay * delay)).clamp(0.2, 20.0);
    let mobility = (-(particle.damping.max(0.0) as f64) / FPS).exp().clamp(0.5, 1.0);
    let mut best = Fit {
        delay,
        mobility,
        acceleration,
        gain: 1.0,
        reach: (1.0, 1.0),
        input_reflect: group.inputs.iter().map(|i| i.invert).collect(),
        output_reflect: vec![false; outputs],
        output_vertex: group.outputs.iter().map(|o| o.particle).collect(),
    };
    let mut best_score = f64::INFINITY;
    let mut consider = |candidate: Fit, best: &mut Fit, best_score: &mut f64| {
        let (fit, s) = solve(&candidate);
        if s < *best_score {
            *best = fit;
            *best_score = s;
        }
    };
    let n = group.particles.len();
    let structure = |base: &Fit,
                     best: &mut Fit,
                     best_score: &mut f64,
                     consider: &mut dyn FnMut(Fit, &mut Fit, &mut f64)| {
        for vertices in choices(outputs, n, 16) {
            for reflect in choices(group.inputs.len(), 2, 8) {
                for (t, a) in [
                    (0.5, 0.5),
                    (1.0, 1.0),
                    (2.0, 2.0),
                    (0.5, 2.0),
                    (2.0, 0.5),
                    (1.0, 0.5),
                    (0.5, 1.0),
                    (1.0, 2.0),
                    (2.0, 1.0),
                ] {
                    let mut c = base.clone();
                    c.output_vertex = vertices.iter().map(|v| v + 1).collect();
                    c.input_reflect = reflect.iter().map(|&r| r == 1).collect();
                    c.reach = (t, a);
                    consider(c, best, best_score);
                }
            }
        }
    };
    let start = best.clone();
    structure(&start, &mut best, &mut best_score, &mut consider);
    for _ in 0..2 {
        let base = best.clone();
        for dm in [0.6, 0.8, 1.0, 1.25, 1.6] {
            for mobility in [0.85, 0.9, 0.93, 0.95, 0.97, 0.99] {
                for am in [0.5, 0.75, 1.0, 1.35, 1.8] {
                    let delay = (base.delay * dm).clamp(0.1, 2.0);
                    // Keep the swing rate while trading delay against
                    // acceleration, then vary it.
                    let acceleration = base.acceleration * (base.delay / delay).powi(2) * am;
                    consider(
                        Fit {
                            delay,
                            mobility,
                            acceleration,
                            ..base.clone()
                        },
                        &mut best,
                        &mut best_score,
                    );
                }
            }
        }
        let base = best.clone();
        structure(&base, &mut best, &mut best_score, &mut consider);
    }
    (best, best_score)
}

/// `physics3.json` for the rig's physics, or `None` when it has none that
/// can be exported.
pub(super) fn export_physics(
    rig: &Rig,
    ids: &BTreeMap<ParameterId, String>,
    notes: &mut Vec<String>,
) -> Option<Value> {
    let mut settings = Vec::new();
    for group in rig.physics.iter().filter(|g| g.enabled && g.validate().is_ok()) {
        let mapped_in = group.inputs.iter().any(|i| ids.contains_key(&i.param));
        let mapped_out = group.outputs.iter().any(|o| ids.contains_key(&o.param));
        if !mapped_in || !mapped_out {
            continue;
        }
        if !group.colliders.is_empty() {
            notes.push(format!("physics \"{}\": colliders are left out", group.name));
        }
        if group.wind != Default::default() || group.turbulence > 0.0 {
            notes.push(format!("physics \"{}\": wind is left out", group.name));
        }
        if group.particles.iter().any(|p| p.max_angle < 180.0) {
            notes.push(format!("physics \"{}\": angle limits are left out", group.name));
        }
        for o in &group.outputs {
            if o.kind != OutputKind::Angle {
                notes.push(format!(
                    "physics \"{}\": X/Y outputs are written as swing angles (Cubism does not scale X/Y)",
                    group.name
                ));
                break;
            }
        }
        for o in &group.outputs {
            if let Some(p) = rig.parameter(o.param).filter(|p| p.default.abs() > 1e-4) {
                notes.push(format!(
                    "physics \"{}\": \"{}\" rests at 0 in Cubism, not at its default {}",
                    group.name, p.name, p.default
                ));
            }
        }
        let index = settings.len();
        let (fit, error) = fit_group(group, index, rig, ids);
        if error > 0.05 {
            notes.push(format!(
                "physics \"{}\" sways somewhat differently in Cubism (typical difference {:.0}% of its range)",
                group.name,
                error.sqrt() * 100.0
            ));
        }
        settings.push((group.name.clone(), setting(group, index, rig, ids, &fit)));
    }
    (!settings.is_empty()).then(|| file(settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_document::rig::physics::InputKind;

    fn rig() -> (Rig, BTreeMap<ParameterId, String>) {
        let mut rig = Rig::default();
        for (id, name) in [(1, "AngleX"), (2, "AngleZ")] {
            rig.parameters
                .push(Parameter::new(ParameterId(id), name, -30.0, 30.0, 0.0));
        }
        rig.parameters
            .push(Parameter::new(ParameterId(3), "HairFront", -1.0, 1.0, 0.0));
        rig.physics.push(PhysicsGroup::sway(
            "Front hair",
            &[
                (ParameterId(1), InputKind::X, 1.0),
                (ParameterId(2), InputKind::Angle, 1.0),
            ],
            ParameterId(3),
        ));
        let ids = rig
            .parameters
            .iter()
            .map(|p| (p.id, format!("Param{}", p.name)))
            .collect();
        (rig, ids)
    }

    #[test]
    fn exported_physics_sways_like_the_editor() {
        let (rig, ids) = rig();
        let mut notes = Vec::new();
        let json = export_physics(&rig, &ids, &mut notes).expect("exported");
        assert_eq!(json["Meta"]["PhysicsSettingCount"], 1);
        assert_eq!(
            json["PhysicsSettings"][0]["Vertices"].as_array().unwrap().len(),
            4
        );
        let group = &rig.physics[0];
        let frames = schedule(group, &rig);
        let a = ours(group, &rig, &frames);
        let b = theirs(&json, group, &rig, &ids, &frames).expect("parses");
        let error = score(&a, &b, group, &rig);
        // Clearly closer than doing nothing (the output staying at rest).
        let still = score(&a, &vec![vec![0.0]; a.len()], group, &rig);
        assert!(error < still * 0.7, "fit {error} vs still {still}");
        // Both swing the same way while the head sways.
        let sway = 264..a.len();
        let together: f32 = a[sway.clone()]
            .iter()
            .zip(&b[sway])
            .map(|(x, y)| x[0] * y[0])
            .sum();
        assert!(together > 0.0, "{notes:?}");
    }
}
