//! Rigging Aether-chan.
//!
//! The auto-rigger does most of the work from the layer names (head and
//! body turns, blinking, eyes that look around, brows, mouth, blush, hair
//! and ribbon sway with physics, breathing, an idle loop). On top of that,
//! by hand through the rig API:
//!
//! * **smiling eyes**: `EyeLSmile`/`EyeRSmile` lift the lower lids of open
//!   eyes and bend closed ones into happy arcs;
//! * **expressions**: Smile, Surprised, Angry, Troubled and Shy;
//! * **motions**: Greeting, Happy, Surprised and Shy, next to the idle loop.

use crate::character::{self, Parts};
use aether_core::math::Vec2;
use aether_core::{AetherError, LayerId, ParameterId, Result};
use aether_document::rig::motion::{Easing, Expression, ExpressionBlend, ExpressionEntry};
use aether_document::rig::{MeshForm, Motion, Rig, RigNode};
use aether_document::rigging::auto_rig_document;
use aether_document::Document;
use std::collections::BTreeMap;

/// Mesh density for the auto-mesher.
const DENSITY: f32 = 1.2;

pub(crate) fn param(rig: &Rig, name: &str) -> Result<ParameterId> {
    rig.parameter_named(name)
        .map(|p| p.id)
        .ok_or_else(|| AetherError::rig(format!("no parameter {name}")))
}

/// Edit every keyform of a mesh, given the key each parameter sits on.
fn edit_forms(
    rig: &mut Rig,
    layer: LayerId,
    mut edit: impl FnMut(&BTreeMap<ParameterId, f32>, &[Vec2], &mut MeshForm),
) -> Result<()> {
    let mesh = rig
        .mesh_mut(layer)
        .ok_or_else(|| AetherError::rig("the part has no mesh"))?;
    let rest = mesh.vertices.clone();
    let grid = &mut mesh.keyforms;
    for i in 0..grid.forms.len() {
        let coords = grid.coords_of(i);
        let keys: BTreeMap<ParameterId, f32> = grid
            .axes
            .iter()
            .zip(coords)
            .map(|(axis, k)| (axis.param, axis.keys[k]))
            .collect();
        edit(&keys, &rest, &mut grid.forms[i]);
    }
    Ok(())
}

fn bounds(points: &[Vec2]) -> (Vec2, Vec2) {
    points
        .iter()
        .fold((points[0], points[0]), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
}

/// Smiling eyes, for one side.
fn smiling_eyes(rig: &mut Rig, white: LayerId, lash: LayerId, side: &str) -> Result<()> {
    let open = param(rig, &format!("Eye{side}Open"))?;
    let smile = param(rig, &format!("Eye{side}Smile"))?;
    let on = |keys: &BTreeMap<ParameterId, f32>, p: ParameterId, v: f32| {
        keys.get(&p).is_some_and(|k| (k - v).abs() < 1e-4)
    };

    // Open and smiling: the lower lid rises, trimming the eye from below.
    rig.bind_parameter(RigNode::Mesh(white), smile, &[0.0, 1.0])?;
    edit_forms(rig, white, |keys, rest, form| {
        if on(keys, smile, 1.0) && on(keys, open, 1.0) {
            let (lo, hi) = bounds(rest);
            let centre = (lo.y + hi.y) * 0.5;
            for (o, v) in form.offsets.iter_mut().zip(rest) {
                if v.y > centre {
                    o.y -= (v.y - centre) * 0.55;
                }
            }
        }
    })?;
    // Closed and smiling: the lash bends into an upward arc.
    rig.bind_parameter(RigNode::Mesh(lash), smile, &[0.0, 1.0])?;
    edit_forms(rig, lash, |keys, rest, form| {
        if on(keys, smile, 1.0) && on(keys, open, 0.0) {
            let (lo, hi) = bounds(rest);
            let (cx, hw) = ((lo.x + hi.x) * 0.5, ((hi.x - lo.x) * 0.5).max(1.0));
            for (o, v) in form.offsets.iter_mut().zip(rest) {
                let u = ((v.x - cx) / hw).clamp(-1.0, 1.0);
                o.y -= 16.0 * (1.0 - u * u);
            }
        }
    })?;
    Ok(())
}

pub(crate) fn expression(rig: &Rig, name: &str, values: &[(&str, f32)]) -> Result<Expression> {
    let entries = values
        .iter()
        .map(|&(p, value)| {
            Ok(ExpressionEntry {
                param: param(rig, p)?,
                value,
                blend: ExpressionBlend::Overwrite,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Expression {
        name: name.into(),
        entries,
        fade: 0.3,
    })
}

/// A motion from `(parameter, [(time, value)])` curves, eased in and out.
pub(crate) fn motion(
    rig: &Rig,
    name: &str,
    duration: f32,
    looping: bool,
    curves: &[(&str, &[(f32, f32)])],
) -> Result<Motion> {
    let mut m = Motion::new(name, duration, 30.0);
    m.looping = looping;
    m.fade_in = 0.3;
    m.fade_out = 0.4;
    for &(p, keys) in curves {
        let id = param(rig, p)?;
        let track = m.track_mut(id);
        for &(t, v) in keys {
            let i = track.set_key(t, v);
            track.keys[i].easing = Easing::EaseInOut;
        }
    }
    Ok(m)
}

/// Expressions and motions beyond the idle loop.
fn acting(rig: &mut Rig) -> Result<()> {
    let expressions = vec![
        expression(
            rig,
            "Smile",
            &[
                ("EyeLSmile", 1.0),
                ("EyeRSmile", 1.0),
                ("EyeLOpen", 0.0),
                ("EyeROpen", 0.0),
                ("MouthForm", 1.0),
                ("MouthOpenY", 0.35),
                ("Cheek", 0.5),
            ],
        )?,
        expression(
            rig,
            "Surprised",
            &[
                ("BrowLY", 1.0),
                ("BrowRY", 1.0),
                ("MouthOpenY", 0.9),
                ("MouthForm", -0.4),
            ],
        )?,
        expression(
            rig,
            "Angry",
            &[
                ("BrowLY", -1.0),
                ("BrowRY", -1.0),
                ("EyeLOpen", 0.75),
                ("EyeROpen", 0.75),
                ("MouthForm", -1.0),
            ],
        )?,
        expression(
            rig,
            "Troubled",
            &[
                ("BrowLY", 0.8),
                ("BrowRY", 0.8),
                ("EyeLOpen", 0.8),
                ("EyeROpen", 0.8),
                ("MouthForm", -0.6),
            ],
        )?,
        expression(
            rig,
            "Shy",
            &[
                ("Cheek", 1.0),
                ("EyeLSmile", 0.6),
                ("EyeRSmile", 0.6),
                ("MouthForm", 0.4),
                ("EyeBallY", -0.4),
            ],
        )?,
    ];
    rig.expressions.extend(expressions);

    let motions = vec![
        motion(
            rig,
            "Greeting",
            3.2,
            false,
            &[
                ("AngleY", &[(0.0, 0.0), (0.45, -12.0), (0.9, 4.0), (1.4, 0.0)]),
                ("AngleZ", &[(0.0, 0.0), (0.8, 9.0), (2.4, 9.0), (3.0, 0.0)]),
                ("BodyAngleZ", &[(0.0, 0.0), (0.9, 3.0), (2.4, 3.0), (3.1, 0.0)]),
                ("EyeLOpen", &[(0.0, 1.0), (0.5, 0.0), (2.3, 0.0), (2.7, 1.0)]),
                ("EyeROpen", &[(0.0, 1.0), (0.5, 0.0), (2.3, 0.0), (2.7, 1.0)]),
                ("EyeLSmile", &[(0.0, 0.0), (0.5, 1.0), (2.4, 1.0), (2.8, 0.0)]),
                ("EyeRSmile", &[(0.0, 0.0), (0.5, 1.0), (2.4, 1.0), (2.8, 0.0)]),
                ("MouthForm", &[(0.0, 0.0), (0.5, 1.0), (2.6, 1.0), (3.0, 0.2)]),
                (
                    "MouthOpenY",
                    &[
                        (0.0, 0.0),
                        (0.7, 0.7),
                        (1.0, 0.2),
                        (1.3, 0.6),
                        (1.7, 0.1),
                        (2.0, 0.0),
                    ],
                ),
                ("Cheek", &[(0.0, 0.0), (0.6, 0.7), (2.6, 0.7), (3.2, 0.0)]),
            ],
        )?,
        motion(
            rig,
            "Happy",
            2.0,
            true,
            &[
                (
                    "AngleZ",
                    &[(0.0, -6.0), (0.5, 6.0), (1.0, -6.0), (1.5, 6.0), (2.0, -6.0)],
                ),
                (
                    "AngleY",
                    &[
                        (0.0, 0.0),
                        (0.25, 5.0),
                        (0.5, 0.0),
                        (0.75, 5.0),
                        (1.0, 0.0),
                        (1.25, 5.0),
                        (1.5, 0.0),
                        (1.75, 5.0),
                        (2.0, 0.0),
                    ],
                ),
                (
                    "BodyAngleZ",
                    &[(0.0, -3.0), (0.5, 3.0), (1.0, -3.0), (1.5, 3.0), (2.0, -3.0)],
                ),
                ("EyeLSmile", &[(0.0, 1.0), (2.0, 1.0)]),
                ("EyeRSmile", &[(0.0, 1.0), (2.0, 1.0)]),
                ("MouthForm", &[(0.0, 1.0), (2.0, 1.0)]),
                (
                    "MouthOpenY",
                    &[(0.0, 0.5), (0.5, 0.7), (1.0, 0.5), (1.5, 0.7), (2.0, 0.5)],
                ),
                ("Cheek", &[(0.0, 0.6), (2.0, 0.6)]),
            ],
        )?,
        motion(
            rig,
            "Surprised",
            2.6,
            false,
            &[
                ("AngleY", &[(0.0, 0.0), (0.2, 10.0), (1.6, 8.0), (2.4, 0.0)]),
                ("AngleX", &[(0.0, 0.0), (0.2, -4.0), (1.6, -3.0), (2.4, 0.0)]),
                ("BodyAngleY", &[(0.0, 0.0), (0.25, 4.0), (2.4, 0.0)]),
                ("BrowLY", &[(0.0, 0.0), (0.15, 1.0), (1.8, 1.0), (2.4, 0.0)]),
                ("BrowRY", &[(0.0, 0.0), (0.15, 1.0), (1.8, 1.0), (2.4, 0.0)]),
                ("MouthOpenY", &[(0.0, 0.0), (0.2, 0.9), (1.6, 0.7), (2.4, 0.0)]),
                ("MouthForm", &[(0.0, 0.0), (0.2, -0.5), (1.6, -0.3), (2.4, 0.0)]),
                ("EyeBallY", &[(0.0, 0.0), (0.2, 0.3), (2.0, 0.0)]),
            ],
        )?,
        motion(
            rig,
            "Shy",
            3.0,
            false,
            &[
                ("AngleX", &[(0.0, 0.0), (0.8, -14.0), (2.3, -12.0), (3.0, 0.0)]),
                ("AngleY", &[(0.0, 0.0), (0.8, -10.0), (2.3, -9.0), (3.0, 0.0)]),
                ("AngleZ", &[(0.0, 0.0), (0.8, -6.0), (2.3, -5.0), (3.0, 0.0)]),
                ("EyeBallX", &[(0.0, 0.0), (0.6, 0.7), (2.4, 0.6), (3.0, 0.0)]),
                ("EyeBallY", &[(0.0, 0.0), (0.6, -0.4), (2.4, -0.4), (3.0, 0.0)]),
                ("Cheek", &[(0.0, 0.0), (0.7, 1.0), (2.4, 1.0), (3.0, 0.0)]),
                ("MouthForm", &[(0.0, 0.0), (0.7, 0.4), (2.4, 0.4), (3.0, 0.0)]),
            ],
        )?,
    ];
    rig.motions.extend(motions);
    // Motions on 1–9, expressions on Shift+1–9, 0 back to the plain pose.
    rig.hotkeys = aether_document::rig::hotkey::default_hotkeys(rig);
    Ok(())
}

/// Paint and rig Aether-chan: a complete, animated character.
pub fn aether_chan() -> Result<Document> {
    let (mut doc, parts) = character::paint();
    rig(&mut doc, &parts)?;
    Ok(doc)
}

/// Rig painted parts (see the module documentation).
pub fn rig(doc: &mut Document, parts: &Parts) -> Result<()> {
    auto_rig_document(doc, DENSITY)?;
    let rig = &mut doc.rig;
    smiling_eyes(rig, parts.eye_white[0], parts.lash[0], "L")?;
    smiling_eyes(rig, parts.eye_white[1], parts.lash[1], "R")?;
    acting(rig)?;
    rig.validate()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_document::rig::Evaluator;

    #[test]
    fn aether_chan_is_rigged_and_acts() {
        let (mut doc, parts) = character::paint();
        let report = auto_rig_document(&mut doc, DENSITY).expect("auto rig");
        assert!(report.unrecognised.is_empty(), "{:?}", report.unrecognised);
        assert!(report.physics >= 2);
        let mut doc = aether_chan().expect("rigs");
        let rig = &doc.rig;
        assert_eq!(rig.meshes.len(), 22, "every part is meshed");
        for name in ["Idle", "Greeting", "Happy", "Surprised", "Shy"] {
            assert!(rig.motions.iter().any(|m| m.name == name), "{name}");
        }
        assert_eq!(rig.expressions.len(), 5);

        // Smiling closed eyes arch upward in the middle.
        let lash = parts.lash[0];
        let closed = |doc: &mut Document, smile: f32| {
            let open = param(&doc.rig, "EyeLOpen").unwrap();
            let s = param(&doc.rig, "EyeLSmile").unwrap();
            doc.rig.set_value(open, 0.0);
            doc.rig.set_value(s, smile);
            let pose = Evaluator::new(&doc.rig).pose();
            let p = &pose.meshes[&lash].positions;
            let (lo, hi) = bounds(p);
            let mid: Vec<f32> = p
                .iter()
                .filter(|v| (v.x - (lo.x + hi.x) * 0.5).abs() < 6.0)
                .map(|v| v.y)
                .collect();
            mid.iter().sum::<f32>() / mid.len().max(1) as f32
        };
        let plain = closed(&mut doc, 0.0);
        let happy = closed(&mut doc, 1.0);
        assert!(happy < plain - 8.0, "the middle rises: {plain} → {happy}");
    }
}
