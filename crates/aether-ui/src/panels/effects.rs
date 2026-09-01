//! The layer effect stack.
//!
//! The panel edits a *copy* of the stack and hands the whole thing back through
//! one command. That keeps the undo story simple — adding, removing,
//! reordering and retuning are all "the stack was X, now it is Y" — and means
//! the widgets never have to hold a borrow of the document while a slider is
//! being dragged.

use crate::panels::effect_editor;
use crate::state::EditorState;
use aether_core::LayerId;
use aether_raster::effect::{EffectKind, LayerEffect};
use egui::{RichText, Ui};

/// Draw the effect list for `layer`, applying any edit through the history.
pub fn effects_section(ui: &mut Ui, state: &mut EditorState, layer: LayerId) {
    let Some(current) = state.doc.layers.get(layer).map(|l| l.effects.clone()) else {
        return;
    };
    let mut effects = current.clone();
    let mut changed = false;
    let mut label = "Edit Effect";

    ui.horizontal(|ui| {
        ui.label(RichText::new(state.language.tr("effects.title")).strong());
        ui.menu_button(state.language.tr("effects.add"), |ui| {
            for preset in EffectKind::presets() {
                if ui.button(preset.name()).clicked() {
                    effects.push(LayerEffect::new(preset));
                    changed = true;
                    label = "Add Effect";
                    ui.close();
                }
            }
        });
    });

    if effects.is_empty() {
        ui.label(state.language.tr("effects.none"));
        return;
    }

    let mut remove: Option<usize> = None;
    let mut swap: Option<(usize, usize)> = None;
    let count = effects.len();
    for (index, effect) in effects.iter_mut().enumerate() {
        let id = ui.make_persistent_id(("effect", layer.raw(), index));
        egui::CollapsingHeader::new(effect.name())
            .id_salt(id)
            .default_open(index + 1 == count)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut effect.enabled, "On").changed() {
                        changed = true;
                        label = "Toggle Effect";
                    }
                    // Effects apply bottom-up in list order, so moving one
                    // changes the result: shadow-then-blur is not blur-then-shadow.
                    if ui.add_enabled(index > 0, egui::Button::new("▲")).clicked() {
                        swap = Some((index, index - 1));
                    }
                    if ui
                        .add_enabled(index + 1 < count, egui::Button::new("▼"))
                        .clicked()
                    {
                        swap = Some((index, index + 1));
                    }
                    if ui.button(state.language.tr("effects.remove")).clicked() {
                        remove = Some(index);
                    }
                });
                if effect_editor(ui, &mut effect.kind, &format!("fx-{}-{index}", layer.raw())) {
                    changed = true;
                }
            });
    }

    if let Some((a, b)) = swap {
        effects.swap(a, b);
        changed = true;
        label = "Reorder Effects";
    }
    if let Some(index) = remove {
        effects.remove(index);
        changed = true;
        label = "Remove Effect";
    }

    if changed && effects != current {
        if let Err(error) = state.set_layer_effects(layer, effects, label) {
            state.report_error("Effects", &error);
        }
    }
}
