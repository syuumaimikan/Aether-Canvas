//! The rig panel: the deformer/bone/mesh hierarchy and an inspector for the
//! selected object, including the one-click generators.

use crate::icons;
use crate::state::EditorState;
use aether_core::math::Vec2;
use aether_core::{LayerId, ParameterId};
use aether_document::rig::generate::Anchor;
use aether_document::rig::{DeformerKind, IkConstraint, KeyInterpolation, NodeRef, Rig, RigNode};
use egui::{Color32, RichText, Ui};

/// The rig hierarchy and inspector.
pub fn rig_panel(ui: &mut Ui, state: &mut EditorState) {
    toolbar(ui, state);
    ui.separator();
    let height = (ui.available_height() * 0.42).max(120.0);
    egui::ScrollArea::vertical()
        .id_salt("rig-tree")
        .max_height(height)
        .show(ui, |ui| {
            if state.doc.rig.meshes.is_empty()
                && state.doc.rig.deformers.is_empty()
                && state.doc.rig.bones.is_empty()
            {
                ui.label(RichText::new(state.tr("rig.empty")).weak());
            }
            for node in top_level(state) {
                node_row(ui, state, node, 0);
            }
        });
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("rig-inspector")
        .show(ui, |ui| inspector(ui, state));
}

fn report(state: &mut EditorState, context: &str, result: aether_core::Result<()>) {
    if let Err(e) = result {
        state.report_error(context, &e);
    }
}

fn toolbar(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(lang.tr("rig.mesh_layer"))
            .on_hover_text(lang.tr("rig.mesh_layer_hint"))
            .clicked()
        {
            let layer = state.doc.active_layer;
            let r = state.mesh_layer(layer);
            report(state, "Mesh", r);
        }
        if ui
            .button(lang.tr("rig.mesh_all"))
            .on_hover_text(lang.tr("rig.mesh_all_hint"))
            .clicked()
        {
            match state.mesh_all_layers(None) {
                Ok(n) => state.report(format!("{} {n}", lang.tr("rig.meshed"))),
                Err(e) => state.report_error("Mesh", &e),
            }
        }
        if ui
            .button(lang.tr("rig.add_warp"))
            .on_hover_text(lang.tr("rig.wrap_hint"))
            .clicked()
        {
            let r = state.add_warp_deformer("Warp");
            report(state, "Warp deformer", r);
        }
        if ui
            .button(lang.tr("rig.add_rotation"))
            .on_hover_text(lang.tr("rig.wrap_hint"))
            .clicked()
        {
            let r = state.add_rotation_deformer("Rotation");
            report(state, "Rotation deformer", r);
        }
        if let Some(node) = state.rig.tool.selection {
            if ui.button(lang.tr("rig.delete")).clicked() {
                let r = state.delete_rig_node(node);
                report(state, "Delete", r);
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut state.rig.show_mesh, lang.tr("rig.show_mesh"));
        ui.checkbox(&mut state.rig.show_deformers, lang.tr("rig.show_deformers"));
        ui.checkbox(&mut state.rig.show_bones, lang.tr("rig.show_bones"));
        let mut rest = state.rig.rest_view;
        if ui.checkbox(&mut rest, lang.tr("rig.rest_view")).changed() {
            state.rig.rest_view = rest;
        }
    });
}

/// Top-level nodes, hiding meshes whose layer has been deleted.
fn top_level(state: &EditorState) -> Vec<RigNode> {
    visible_children(state, None)
}

fn visible_children(state: &EditorState, parent: Option<NodeRef>) -> Vec<RigNode> {
    state
        .doc
        .rig
        .children_of(parent)
        .into_iter()
        .filter(|n| match n {
            RigNode::Mesh(layer) => state.doc.layers.contains(*layer),
            _ => true,
        })
        .collect()
}

fn node_row(ui: &mut Ui, state: &mut EditorState, node: RigNode, depth: usize) {
    if depth > 32 {
        return;
    }
    let rig = &state.doc.rig;
    let (icon, color) = match node {
        RigNode::Mesh(_) => (icons::MESH, Color32::from_rgb(170, 200, 255)),
        RigNode::Bone(_) => (icons::BONE, Color32::from_rgb(240, 220, 170)),
        RigNode::Deformer(id) => match rig.deformer(id).map(|d| &d.kind) {
            Some(DeformerKind::Warp(_)) => (icons::WARP, Color32::from_rgb(140, 220, 170)),
            _ => (icons::ROTATION, Color32::from_rgb(230, 170, 230)),
        },
    };
    let mut name = rig.node_name(node);
    if let RigNode::Mesh(layer) = node {
        if let Some(l) = state.doc.layers.get(layer) {
            if rig.mesh(layer).map(|m| m.name.is_empty()).unwrap_or(false) {
                name = l.name.clone();
            }
        }
    }
    let keyed = rig
        .grid(node, None)
        .map(|g| !g.axes().is_empty())
        .unwrap_or(false);
    let selected = state.rig.tool.selection == Some(node);
    let clicked = ui
        .horizontal(|ui| {
            ui.add_space(depth as f32 * 14.0);
            ui.label(RichText::new(icon).color(color));
            let mut text = RichText::new(name);
            if keyed {
                text = text.strong();
            }
            ui.selectable_label(selected, text).clicked()
        })
        .inner;
    if clicked {
        state.rig.tool.selection = Some(node);
        state.rig.tool.blend_shape = None;
        if let RigNode::Mesh(layer) = node {
            state.doc.set_active_layer(layer);
        }
    }
    let children = match node {
        RigNode::Deformer(id) => visible_children(state, Some(NodeRef::Deformer(id))),
        RigNode::Bone(id) => visible_children(state, Some(NodeRef::Bone(id))),
        RigNode::Mesh(_) => Vec::new(),
    };
    for child in children {
        node_row(ui, state, child, depth + 1);
    }
}

/// A combo box choosing one parameter (or none).
fn param_combo(ui: &mut Ui, salt: &str, rig: &Rig, value: &mut Option<ParameterId>, none: &str) -> bool {
    let mut changed = false;
    let text = value
        .and_then(|id| rig.parameter(id))
        .map(|p| p.name.clone())
        .unwrap_or_else(|| none.to_string());
    egui::ComboBox::from_id_salt(salt)
        .selected_text(text)
        .width(130.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(value.is_none(), none).clicked() {
                *value = None;
                changed = true;
            }
            for p in &rig.parameters {
                if ui.selectable_label(*value == Some(p.id), &p.name).clicked() {
                    *value = Some(p.id);
                    changed = true;
                }
            }
        });
    changed
}

fn inspector(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    let Some(node) = state.rig.tool.selection.filter(|n| state.doc.rig.contains(*n)) else {
        ui.label(RichText::new(lang.tr("rig.select_hint")).weak());
        return;
    };
    let key = format!("{node:?}");

    // Name.
    let mut name = state.doc.rig.node_name(node);
    ui.horizontal(|ui| {
        ui.label(lang.tr("dialog.name"));
        if ui.text_edit_singleline(&mut name).changed() {
            let r = state.edit_rig("Rename", Some(format!("rename:{key}")), |rig, _| {
                match node {
                    RigNode::Mesh(l) => {
                        if let Some(m) = rig.mesh_mut(l) {
                            m.name = name.clone();
                        }
                    }
                    RigNode::Deformer(id) => {
                        if let Some(d) = rig.deformer_mut(id) {
                            d.name = name.clone();
                        }
                    }
                    RigNode::Bone(id) => {
                        if let Some(b) = rig.bone_mut(id) {
                            b.name = name.clone();
                        }
                    }
                }
                Ok(())
            });
            report(state, "Rename", r);
        }
    });

    parent_combo(ui, state, node);
    keyed_parameters(ui, state, node);

    match node {
        RigNode::Mesh(layer) => mesh_inspector(ui, state, layer),
        RigNode::Deformer(id) => deformer_inspector(ui, state, id),
        RigNode::Bone(id) => bone_inspector(ui, state, id),
    }
    if !matches!(node, RigNode::Bone(_)) {
        generators(ui, state, node);
    }
}

fn parent_combo(ui: &mut Ui, state: &mut EditorState, node: RigNode) {
    let lang = state.language;
    let current = state.doc.rig.parent_of(node);
    let label = |rig: &Rig, p: Option<NodeRef>| match p {
        None => lang.tr("rig.parent_none").to_string(),
        Some(p) => rig.node_name(p.into()),
    };
    let mut chosen: Option<Option<NodeRef>> = None;
    ui.horizontal(|ui| {
        ui.label(lang.tr("rig.parent"));
        egui::ComboBox::from_id_salt("rig-parent")
            .selected_text(label(&state.doc.rig, current))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(current.is_none(), lang.tr("rig.parent_none"))
                    .clicked()
                {
                    chosen = Some(None);
                }
                let rig = &state.doc.rig;
                let mut options: Vec<NodeRef> = Vec::new();
                if !matches!(node, RigNode::Bone(_)) {
                    options.extend(rig.deformers.iter().map(|d| NodeRef::Deformer(d.id)));
                }
                options.extend(rig.bones.iter().map(|b| NodeRef::Bone(b.id)));
                for option in options {
                    if RigNode::from(option) == node {
                        continue;
                    }
                    if ui
                        .selectable_label(current == Some(option), rig.node_name(option.into()))
                        .clicked()
                    {
                        chosen = Some(Some(option));
                    }
                }
            });
    });
    if let Some(parent) = chosen {
        let r = state.set_rig_parent(node, parent);
        report(state, "Parent", r);
    }
}

fn keyed_parameters(ui: &mut Ui, state: &mut EditorState, node: RigNode) {
    let lang = state.language;
    let Some(grid) = state.doc.rig.grid(node, None) else {
        return;
    };
    let axes: Vec<(String, Vec<f32>)> = grid
        .axes()
        .iter()
        .map(|a| {
            (
                state
                    .doc
                    .rig
                    .parameter(a.param)
                    .map(|p| p.name.clone())
                    .unwrap_or_default(),
                a.keys.clone(),
            )
        })
        .collect();
    let mut interpolation = grid.interpolation();
    ui.separator();
    ui.label(RichText::new(lang.tr("rig.keyed_by")).strong());
    if axes.is_empty() {
        ui.label(RichText::new(lang.tr("rig.not_keyed")).weak());
    }
    for (name, keys) in &axes {
        let keys: Vec<String> = keys.iter().map(|k| format!("{k:.2}")).collect();
        ui.label(format!("• {name}: {}", keys.join(", ")));
    }
    let before = interpolation;
    ui.horizontal(|ui| {
        ui.label(lang.tr("rig.interpolation"));
        for mode in KeyInterpolation::ALL {
            let text = match mode {
                KeyInterpolation::Linear => lang.tr("rig.interp.linear"),
                KeyInterpolation::Smooth => lang.tr("rig.interp.smooth"),
            };
            ui.selectable_value(&mut interpolation, mode, text);
        }
    });
    if interpolation != before {
        let r = state.edit_rig("Interpolation", None, |rig, _| {
            if let Some(g) = rig.grid_mut(node, None) {
                g.set_interpolation(interpolation);
            }
            Ok(())
        });
        report(state, "Interpolation", r);
    }

    // Blend shapes: pick one to edit, weight, delete.
    let shapes: Vec<(String, f32)> = match node {
        RigNode::Mesh(l) => state
            .doc
            .rig
            .mesh(l)
            .map(|m| {
                m.blend_shapes
                    .iter()
                    .map(|s| (s.name.clone(), s.weight))
                    .collect()
            })
            .unwrap_or_default(),
        RigNode::Deformer(id) => match state.doc.rig.deformer(id).map(|d| &d.kind) {
            Some(DeformerKind::Warp(w)) => w
                .blend_shapes
                .iter()
                .map(|s| (s.name.clone(), s.weight))
                .collect(),
            Some(DeformerKind::Rotation(r)) => r
                .blend_shapes
                .iter()
                .map(|s| (s.name.clone(), s.weight))
                .collect(),
            None => Vec::new(),
        },
        RigNode::Bone(_) => Vec::new(),
    };
    if shapes.is_empty() {
        return;
    }
    ui.label(RichText::new(lang.tr("rig.blend_shapes")).strong());
    let mut target = state.rig.tool.blend_shape;
    ui.radio_value(&mut target, None, lang.tr("rig.base_keyforms"));
    let mut remove: Option<usize> = None;
    let mut reweigh: Option<(usize, f32)> = None;
    for (i, (name, weight)) in shapes.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.radio_value(&mut target, Some(i), name);
            let mut w = *weight;
            if ui
                .add(egui::Slider::new(&mut w, 0.0..=1.0).max_decimals(2))
                .changed()
            {
                reweigh = Some((i, w));
            }
            if ui.small_button(icons::DELETE).clicked() {
                remove = Some(i);
            }
        });
    }
    state.rig.tool.blend_shape = target;
    if let Some((i, w)) = reweigh {
        let r = state.edit_rig(
            "Blend shape weight",
            Some(format!("shape-weight:{node:?}:{i}")),
            |rig, _| {
                set_shape(rig, node, i, |weight, _| *weight = w);
                Ok(())
            },
        );
        report(state, "Blend shape", r);
    }
    if let Some(i) = remove {
        let r = state.edit_rig("Delete blend shape", None, |rig, _| {
            set_shape(rig, node, i, |_, remove| *remove = true);
            Ok(())
        });
        state.rig.tool.blend_shape = None;
        report(state, "Blend shape", r);
    }
}

/// Edit blend shape `i` of `node`: `f(weight, remove)`.
fn set_shape(rig: &mut Rig, node: RigNode, i: usize, f: impl FnOnce(&mut f32, &mut bool)) {
    let mut remove = false;
    macro_rules! apply {
        ($shapes:expr) => {{
            if let Some(shape) = $shapes.get_mut(i) {
                f(&mut shape.weight, &mut remove);
            }
            if remove && i < $shapes.len() {
                $shapes.remove(i);
            }
        }};
    }
    match node {
        RigNode::Mesh(l) => {
            if let Some(m) = rig.mesh_mut(l) {
                apply!(m.blend_shapes)
            }
        }
        RigNode::Deformer(id) => match rig.deformer_mut(id).map(|d| &mut d.kind) {
            Some(DeformerKind::Warp(w)) => apply!(w.blend_shapes),
            Some(DeformerKind::Rotation(r)) => apply!(r.blend_shapes),
            None => {}
        },
        RigNode::Bone(_) => {}
    }
}

fn mesh_inspector(ui: &mut Ui, state: &mut EditorState, layer: LayerId) {
    let lang = state.language;
    let Some(mesh) = state.doc.rig.mesh(layer).cloned() else {
        return;
    };
    ui.separator();
    ui.label(format!(
        "{} {} · {} {}",
        mesh.vertex_count(),
        lang.tr("rig.vertices"),
        mesh.triangles.len(),
        lang.tr("rig.triangles")
    ));
    ui.horizontal(|ui| {
        ui.add(
            egui::Slider::new(&mut state.rig.tool.mesh_density, 0.3..=4.0)
                .text(lang.tr("rig.density"))
                .max_decimals(1),
        );
        if ui
            .button(lang.tr("rig.remesh"))
            .on_hover_text(lang.tr("rig.remesh_hint"))
            .clicked()
        {
            let r = state.mesh_layer(layer);
            report(state, "Re-mesh", r);
        }
    });

    // Jiggle.
    let mut jiggle_on = mesh.jiggle.is_some();
    if ui
        .checkbox(&mut jiggle_on, lang.tr("rig.jiggle"))
        .on_hover_text(lang.tr("rig.jiggle_hint"))
        .changed()
    {
        let r = state.set_jiggle(layer, jiggle_on);
        report(state, "Jiggle", r);
    }
    if let Some(mut j) = mesh.jiggle.clone() {
        let before = j.clone();
        ui.add(egui::Slider::new(&mut j.stiffness, 10.0..=800.0).text(lang.tr("rig.stiffness")));
        ui.add(egui::Slider::new(&mut j.damping, 0.0..=30.0).text(lang.tr("rig.damping")));
        ui.add(egui::Slider::new(&mut j.gravity.y, -800.0..=800.0).text(lang.tr("rig.gravity")));
        ui.add(egui::Slider::new(&mut j.max_offset, 1.0..=200.0).text(lang.tr("rig.max_offset")));
        if j != before {
            let r = state.edit_rig("Jiggle", Some(format!("jiggle:{layer}")), |rig, _| {
                if let Some(m) = rig.mesh_mut(layer) {
                    m.jiggle = Some(j);
                }
                Ok(())
            });
            report(state, "Jiggle", r);
        }
    }

    // Skinning.
    ui.horizontal(|ui| {
        ui.add(
            egui::Slider::new(&mut state.rig.generator.skin_falloff, 1.0..=6.0)
                .text(lang.tr("rig.falloff"))
                .max_decimals(1),
        );
        if ui
            .button(lang.tr("rig.skin"))
            .on_hover_text(lang.tr("rig.skin_hint"))
            .clicked()
        {
            let falloff = state.rig.generator.skin_falloff;
            let r = state.auto_skin(falloff);
            report(state, "Skin", r);
        }
        if mesh.skin.is_some() && ui.button(lang.tr("rig.unskin")).clicked() {
            let r = state.edit_rig("Remove skin", None, |rig, _| {
                if let Some(m) = rig.mesh_mut(layer) {
                    m.skin = None;
                }
                Ok(())
            });
            report(state, "Skin", r);
        }
    });

    // Glue.
    ui.horizontal(|ui| {
        ui.label(lang.tr("rig.glue_to"));
        let others: Vec<(LayerId, String)> = state
            .doc
            .rig
            .meshes
            .iter()
            .filter(|m| m.layer != layer && state.doc.layers.contains(m.layer))
            .map(|m| {
                let name = state
                    .doc
                    .layers
                    .get(m.layer)
                    .map(|l| l.name.clone())
                    .unwrap_or_default();
                (m.layer, name)
            })
            .collect();
        let current = state.rig.generator.glue_other;
        let text = others
            .iter()
            .find(|(id, _)| Some(*id) == current)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| "—".into());
        egui::ComboBox::from_id_salt("glue-other")
            .selected_text(text)
            .show_ui(ui, |ui| {
                for (id, name) in &others {
                    ui.selectable_value(&mut state.rig.generator.glue_other, Some(*id), name);
                }
            });
        if ui
            .button(lang.tr("rig.glue"))
            .on_hover_text(lang.tr("rig.glue_hint"))
            .clicked()
        {
            if let Some(other) = state.rig.generator.glue_other {
                match state.auto_glue(layer, other, 6.0) {
                    Ok(n) => state.report(format!("{} {n}", lang.tr("rig.glued"))),
                    Err(e) => state.report_error("Glue", &e),
                }
            }
        }
    });
    if !mesh.glue.is_empty() {
        ui.label(RichText::new(format!("{} {}", mesh.glue.len(), lang.tr("rig.glued"))).weak());
    }
}

fn deformer_inspector(ui: &mut Ui, state: &mut EditorState, id: aether_core::DeformerId) {
    let lang = state.language;
    let Some(deformer) = state.doc.rig.deformer(id).cloned() else {
        return;
    };
    ui.separator();
    match &deformer.kind {
        DeformerKind::Warp(w) => {
            let (mut cols, mut rows, mut smooth) = (w.cols, w.rows, w.smooth);
            ui.horizontal(|ui| {
                ui.label(lang.tr("rig.lattice"));
                ui.add(egui::DragValue::new(&mut cols).range(1..=32));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut rows).range(1..=32));
                ui.checkbox(&mut smooth, lang.tr("rig.bicubic"))
                    .on_hover_text(lang.tr("rig.bicubic_hint"));
            });
            if (cols, rows, smooth) != (w.cols, w.rows, w.smooth) {
                let r = state.edit_rig("Lattice", Some(format!("lattice:{id}")), |rig, _| {
                    if let Some(DeformerKind::Warp(w)) = rig.deformer_mut(id).map(|d| &mut d.kind) {
                        w.resize_lattice(cols, rows);
                        w.smooth = smooth;
                    }
                    Ok(())
                });
                report(state, "Lattice", r);
            }

            // Head turn.
            ui.separator();
            ui.label(RichText::new(lang.tr("rig.head_turn")).strong());
            ui.label(RichText::new(lang.tr("rig.head_turn_hint")).weak().small());
            if state.rig.generator.turn_x.is_none() {
                state.rig.generator.turn_x = state.doc.rig.parameter_named("AngleX").map(|p| p.id);
            }
            if state.rig.generator.turn_y.is_none() {
                state.rig.generator.turn_y = state.doc.rig.parameter_named("AngleY").map(|p| p.id);
            }
            ui.horizontal(|ui| {
                ui.label(lang.tr("rig.turn_x"));
                let rig = state.doc.rig.clone();
                param_combo(ui, "turn-x", &rig, &mut state.rig.generator.turn_x, "—");
                ui.label(lang.tr("rig.turn_y"));
                param_combo(ui, "turn-y", &rig, &mut state.rig.generator.turn_y, "—");
            });
            let options = &mut state.rig.head_turn;
            ui.add(egui::Slider::new(&mut options.yaw, 0.0..=40.0).text(lang.tr("rig.yaw")));
            ui.add(egui::Slider::new(&mut options.pitch, 0.0..=35.0).text(lang.tr("rig.pitch")));
            ui.add(egui::Slider::new(&mut options.depth, 0.0..=1.2).text(lang.tr("rig.depth")));
            ui.add(egui::Slider::new(&mut options.perspective, 0.0..=0.4).text(lang.tr("rig.perspective")));
            ui.add(egui::Slider::new(&mut options.center.y, 0.2..=0.8).text(lang.tr("rig.face_center")));
            if ui.button(lang.tr("rig.generate_turn")).clicked() {
                match state.rig.generator.turn_x {
                    Some(x) => {
                        let y = state.rig.generator.turn_y;
                        let r = state.generate_head_turn(x, y);
                        report(state, "Head turn", r);
                    }
                    None => state.report(lang.tr("rig.need_param")),
                }
            }
        }
        DeformerKind::Rotation(r) => {
            let mut origin = r.origin;
            ui.horizontal(|ui| {
                ui.label(lang.tr("rig.pivot"));
                ui.add(egui::DragValue::new(&mut origin.x).speed(0.5));
                ui.add(egui::DragValue::new(&mut origin.y).speed(0.5));
            });
            if origin != r.origin {
                let result = state.edit_rig("Pivot", Some(format!("pivot:{id}")), |rig, _| {
                    if let Some(DeformerKind::Rotation(r)) = rig.deformer_mut(id).map(|d| &mut d.kind) {
                        r.origin = origin;
                    }
                    Ok(())
                });
                report(state, "Pivot", result);
            }
            ui.label(RichText::new(lang.tr("rig.rotation_hint")).weak().small());
        }
    }
}

fn bone_inspector(ui: &mut Ui, state: &mut EditorState, id: aether_core::BoneId) {
    let lang = state.language;
    let Some(bone) = state.doc.rig.bone(id).cloned() else {
        return;
    };
    ui.separator();
    let mut edited = bone.clone();
    ui.horizontal(|ui| {
        ui.label(lang.tr("rig.head"));
        ui.add(egui::DragValue::new(&mut edited.head.x).speed(0.5));
        ui.add(egui::DragValue::new(&mut edited.head.y).speed(0.5));
    });
    ui.horizontal(|ui| {
        ui.label(lang.tr("rig.tail"));
        ui.add(egui::DragValue::new(&mut edited.tail.x).speed(0.5));
        ui.add(egui::DragValue::new(&mut edited.tail.y).speed(0.5));
    });
    ui.checkbox(&mut edited.deform, lang.tr("rig.deforms"))
        .on_hover_text(lang.tr("rig.deforms_hint"));

    // IK.
    let mut ik_on = edited.ik.is_some();
    if ui.checkbox(&mut ik_on, lang.tr("rig.ik")).changed() {
        edited.ik = if ik_on {
            let target = state.doc.rig.bones.iter().find(|b| b.id != id).map(|b| b.id);
            target.map(|target| IkConstraint {
                target,
                chain: 2,
                bend_positive: true,
                weight: 1.0,
            })
        } else {
            None
        };
        if ik_on && edited.ik.is_none() {
            state.report(lang.tr("rig.ik_needs_target"));
        }
    }
    if let Some(ik) = &mut edited.ik {
        let text = state
            .doc
            .rig
            .bone(ik.target)
            .map(|b| b.name.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(lang.tr("rig.ik_target"));
            egui::ComboBox::from_id_salt("ik-target")
                .selected_text(text)
                .show_ui(ui, |ui| {
                    for b in &state.doc.rig.bones {
                        if b.id != id {
                            ui.selectable_value(&mut ik.target, b.id, &b.name);
                        }
                    }
                });
        });
        ui.add(egui::Slider::new(&mut ik.chain, 1..=8).text(lang.tr("rig.ik_chain")));
        ui.checkbox(&mut ik.bend_positive, lang.tr("rig.ik_bend"));
        ui.add(egui::Slider::new(&mut ik.weight, 0.0..=1.0).text(lang.tr("rig.ik_weight")));
    }
    if edited != bone {
        let r = state.edit_rig("Bone", Some(format!("bone:{id}")), |rig, _| {
            if let Some(b) = rig.bone_mut(id) {
                *b = edited;
            }
            Ok(())
        });
        report(state, "Bone", r);
    }
    if ui
        .button(lang.tr("rig.bone_control"))
        .on_hover_text(lang.tr("rig.bone_control_hint"))
        .clicked()
    {
        if let Err(e) = state.add_bone_rotation_control() {
            state.report_error("Bone control", &e);
        }
    }
}

fn generators(ui: &mut Ui, state: &mut EditorState, node: RigNode) {
    let lang = state.language;
    ui.separator();
    ui.label(RichText::new(lang.tr("rig.generators")).strong());
    let rig = state.doc.rig.clone();

    // Sway.
    if state.rig.generator.sway_param.is_none() {
        state.rig.generator.sway_param = rig.parameter_named("HairFront").map(|p| p.id);
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(lang.tr("rig.sway"));
        param_combo(ui, "sway-param", &rig, &mut state.rig.generator.sway_param, "—");
        ui.add(
            egui::DragValue::new(&mut state.rig.generator.sway_amount)
                .speed(0.5)
                .suffix(" px"),
        );
        egui::ComboBox::from_id_salt("sway-anchor")
            .selected_text(anchor_name(state.rig.generator.sway_anchor, lang))
            .show_ui(ui, |ui| {
                for a in [Anchor::Top, Anchor::Bottom, Anchor::Left, Anchor::Right] {
                    ui.selectable_value(&mut state.rig.generator.sway_anchor, a, anchor_name(a, lang));
                }
            });
        if ui
            .button(lang.tr("rig.generate"))
            .on_hover_text(lang.tr("rig.sway_hint"))
            .clicked()
        {
            let g = state.rig.generator.clone();
            match g.sway_param {
                Some(p) => {
                    let r = state.generate_sway(p, g.sway_amount, g.sway_anchor);
                    report(state, "Sway", r);
                }
                None => state.report(lang.tr("rig.need_param")),
            }
        }
    });

    // Close.
    if state.rig.generator.squash_param.is_none() {
        state.rig.generator.squash_param = rig.parameter_named("EyeLOpen").map(|p| p.id);
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(lang.tr("rig.close"));
        param_combo(
            ui,
            "squash-param",
            &rig,
            &mut state.rig.generator.squash_param,
            "—",
        );
        ui.add(
            egui::Slider::new(&mut state.rig.generator.squash_line, 0.0..=1.0)
                .text(lang.tr("rig.close_line"))
                .max_decimals(2),
        );
        if ui
            .button(lang.tr("rig.generate"))
            .on_hover_text(lang.tr("rig.close_hint"))
            .clicked()
        {
            let g = state.rig.generator.clone();
            match g.squash_param {
                Some(p) => {
                    let r = state.generate_squash(p, g.squash_line);
                    report(state, "Close", r);
                }
                None => state.report(lang.tr("rig.need_param")),
            }
        }
    });
    let _ = (node, Vec2::ZERO);
}

fn anchor_name(anchor: Anchor, lang: crate::i18n::Language) -> &'static str {
    match anchor {
        Anchor::Top => lang.tr("rig.anchor.top"),
        Anchor::Bottom => lang.tr("rig.anchor.bottom"),
        Anchor::Left => lang.tr("rig.anchor.left"),
        Anchor::Right => lang.tr("rig.anchor.right"),
    }
}
