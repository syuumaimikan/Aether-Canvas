//! In-app tutorials: short guided tours of the editor, in English and
//! Japanese.
//!
//! Each tutorial is a few steps; a step says what to do and, where the
//! editor can tell, notices when it has been done (the tool picked, the
//! layer added, the head turned). *Show me* does the step for the user when
//! it is a single action — opening the sample library, running the auto
//! rig, setting a parameter — so no step is a dead end.
//!
//! Every word shown comes from the translation tables: a tutorial's title is
//! `tutorial.<id>`, a step's title `tutorial.<id>.<step>` and its text
//! `tutorial.<id>.<step>.body`.

use crate::shortcuts::Action;
use crate::state::{EditorState, Workspace};
use crate::tools::ToolId;

/// What *Show me* does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Help {
    /// Run an editor action.
    Run(Action),
    /// Switch workspace.
    Workspace(Workspace),
    /// Open the sample library.
    OpenLibrary,
    /// Rig the document from its layer names.
    AutoRig,
    /// Set a parameter, by name.
    Set(&'static str, f32),
    /// Add a motion to the timeline.
    AddMotion,
    /// Export a Live2D model (asks where).
    ExportLive2D,
    /// Export a runtime model (asks where).
    ExportRuntime,
}

/// Counts taken as a step starts, so it can tell what changed since.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Baseline {
    layers: usize,
    history: usize,
    motions: usize,
}

impl Baseline {
    fn of(state: &EditorState) -> Self {
        Self {
            layers: state.doc.layer_count(),
            history: state.history.depth(),
            motions: state.doc.rig.motions.len(),
        }
    }
}

/// Whether a step is done.
type Check = fn(&EditorState, &Baseline) -> bool;

/// One step of a tutorial.
pub struct Step {
    /// Translation key suffix.
    pub id: &'static str,
    check: Option<Check>,
    help: Option<Help>,
}

/// A tutorial.
pub struct Tutorial {
    /// Translation key suffix.
    pub id: &'static str,
    /// Its steps, in order.
    pub steps: &'static [Step],
}

const fn step(id: &'static str, check: Option<Check>, help: Option<Help>) -> Step {
    Step { id, check, help }
}

/// A parameter's value as its slider shows it: the motion's in animate
/// mode, the posed value otherwise.
fn value(state: &EditorState, name: &str) -> Option<f32> {
    let rig = &state.doc.rig;
    let id = rig.parameter_named(name)?.id;
    let animating = state.rig.animate && state.rig.motion.is_some();
    Some(if animating {
        rig.effective_value(id)
    } else {
        rig.value(id)
    })
}

fn in_workspace(state: &EditorState, workspace: Workspace) -> bool {
    state.workspace == workspace
}

fn rigging(s: &EditorState, _: &Baseline) -> bool {
    in_workspace(s, Workspace::Rigging)
}
fn illustration(s: &EditorState, _: &Baseline) -> bool {
    in_workspace(s, Workspace::Illustration)
}
fn animation(s: &EditorState, _: &Baseline) -> bool {
    in_workspace(s, Workspace::Animation)
}
fn brush_picked(s: &EditorState, _: &Baseline) -> bool {
    s.tools.active_id() == ToolId::Brush
}
fn painted(s: &EditorState, b: &Baseline) -> bool {
    s.history.depth() > b.history
}
fn layer_added(s: &EditorState, b: &Baseline) -> bool {
    s.doc.layer_count() > b.layers
}
fn saved(s: &EditorState, _: &Baseline) -> bool {
    s.path.is_some() && !s.has_unsaved_changes()
}
fn parts_open(s: &EditorState, _: &Baseline) -> bool {
    s.doc.layer_count() >= 5
}
fn rigged(s: &EditorState, _: &Baseline) -> bool {
    !s.doc.rig.deformers.is_empty()
}
fn animatable(s: &EditorState, _: &Baseline) -> bool {
    !s.doc.rig.parameters.is_empty() && (!s.doc.rig.deformers.is_empty() || !s.doc.rig.cubism.is_empty())
}
fn head_turned(s: &EditorState, _: &Baseline) -> bool {
    value(s, "AngleX").is_some_and(|v| v.abs() > 5.0)
}
fn eye_closed(s: &EditorState, _: &Baseline) -> bool {
    value(s, "EyeLOpen").is_some_and(|v| v < 0.3)
}
fn playing(s: &EditorState, _: &Baseline) -> bool {
    s.rig.playing
}
fn motion_added(s: &EditorState, b: &Baseline) -> bool {
    s.doc.rig.motions.len() > b.motions
}
fn keyed(s: &EditorState, _: &Baseline) -> bool {
    s.rig
        .motion
        .and_then(|m| s.doc.rig.motions.get(m))
        .is_some_and(|m| m.tracks.iter().any(|t| t.keys.len() >= 2))
}
fn live2d_open(s: &EditorState, _: &Baseline) -> bool {
    !s.doc.rig.cubism.is_empty()
}
fn posed(s: &EditorState, _: &Baseline) -> bool {
    s.doc
        .rig
        .parameters
        .iter()
        .any(|p| value(s, &p.name).is_some_and(|v| (v - p.default).abs() > 1e-3))
}
fn exported(s: &EditorState, _: &Baseline) -> bool {
    s.export_report.is_some()
}

/// Every tutorial, in the order they are offered.
pub static TUTORIALS: &[Tutorial] = &[
    Tutorial {
        id: "tour",
        steps: &[
            step("panels", None, None),
            step(
                "rigging",
                Some(rigging),
                Some(Help::Workspace(Workspace::Rigging)),
            ),
            step("view", None, None),
            step(
                "back",
                Some(illustration),
                Some(Help::Workspace(Workspace::Illustration)),
            ),
        ],
    },
    Tutorial {
        id: "paint",
        steps: &[
            step("brush", Some(brush_picked), Some(Help::Run(Action::ToolBrush))),
            step("draw", Some(painted), None),
            step("layer", Some(layer_added), Some(Help::Run(Action::AddLayer))),
            step("save", Some(saved), Some(Help::Run(Action::Save))),
        ],
    },
    Tutorial {
        id: "rig",
        steps: &[
            step("open", Some(parts_open), Some(Help::OpenLibrary)),
            step("auto", Some(rigged), Some(Help::AutoRig)),
            step("turn", Some(head_turned), Some(Help::Set("AngleX", 20.0))),
            step("blink", Some(eye_closed), Some(Help::Set("EyeLOpen", 0.0))),
            step("play", Some(playing), Some(Help::Run(Action::PlayPause))),
        ],
    },
    Tutorial {
        id: "animate",
        steps: &[
            step("rigged", Some(animatable), None),
            step(
                "workspace",
                Some(animation),
                Some(Help::Workspace(Workspace::Animation)),
            ),
            step("motion", Some(motion_added), Some(Help::AddMotion)),
            step("keys", Some(keyed), None),
            step("play", Some(playing), Some(Help::Run(Action::PlayPause))),
        ],
    },
    Tutorial {
        id: "live2d",
        steps: &[
            step("open", Some(live2d_open), Some(Help::OpenLibrary)),
            step("pose", Some(posed), None),
            step("play", Some(playing), Some(Help::Run(Action::PlayPause))),
        ],
    },
    Tutorial {
        id: "export",
        steps: &[
            step("live2d", Some(exported), Some(Help::ExportLive2D)),
            step("runtime", None, Some(Help::ExportRuntime)),
            step("cli", None, None),
        ],
    },
];

/// The tutorials window's state.
#[derive(Clone, Debug, Default)]
pub struct TutorialState {
    /// Whether the window is shown.
    pub open: bool,
    /// The tutorial shown.
    pub tutorial: usize,
    /// Its current step.
    pub step: usize,
    /// Tutorials finished this session.
    pub finished: Vec<bool>,
    baseline: Baseline,
}

impl TutorialState {
    /// The step shown.
    pub fn current(&self) -> &'static Step {
        let tutorial = &TUTORIALS[self.tutorial.min(TUTORIALS.len() - 1)];
        &tutorial.steps[self.step.min(tutorial.steps.len() - 1)]
    }
}

impl EditorState {
    /// Show the tutorials.
    pub fn open_tutorials(&mut self) {
        self.tutorial.open = true;
        if self.tutorial.finished.len() != TUTORIALS.len() {
            self.tutorial.finished = vec![false; TUTORIALS.len()];
        }
        self.tutorial.baseline = Baseline::of(self);
    }

    /// Start tutorial `index` from its first step.
    pub fn start_tutorial(&mut self, index: usize) {
        self.tutorial.tutorial = index.min(TUTORIALS.len() - 1);
        self.go_to_step(0);
    }

    /// Show step `index` of the current tutorial.
    pub fn go_to_step(&mut self, index: usize) {
        let steps = TUTORIALS[self.tutorial.tutorial].steps.len();
        self.tutorial.step = index.min(steps - 1);
        self.tutorial.baseline = Baseline::of(self);
    }

    /// Whether the current step is done: `None` when it cannot tell.
    pub fn tutorial_step_done(&self) -> Option<bool> {
        let check = self.tutorial.current().check?;
        Some(check(self, &self.tutorial.baseline))
    }

    /// Past the last step: mark the tutorial finished and move to the next.
    fn finish_tutorial(&mut self) {
        let t = self.tutorial.tutorial;
        if let Some(done) = self.tutorial.finished.get_mut(t) {
            *done = true;
        }
        self.status = self.tr("tutorial.finished").to_string();
        self.start_tutorial((t + 1) % TUTORIALS.len());
    }
}

/// The tutorials window. Returns what *Show me* asked for, for the app to
/// carry out (some of it, like switching workspace, is the app's to do).
pub fn tutorial_window(state: &mut EditorState, ctx: &egui::Context) -> Option<Help> {
    if !state.tutorial.open {
        return None;
    }
    let lang = state.language;
    let mut open = true;
    let mut help = None;
    let mut start: Option<usize> = None;
    let mut go: Option<usize> = None;
    let mut finish = false;
    let (t, s) = (state.tutorial.tutorial, state.tutorial.step);
    let tutorial = &TUTORIALS[t];
    let step = &tutorial.steps[s];
    let done = state.tutorial_step_done();
    egui::Window::new(lang.tr("tutorial.title"))
        // Stays put when the language, and so the title, changes.
        .id(egui::Id::new("tutorials"))
        .default_width(640.0)
        .default_height(320.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(170.0);
                    for (i, tutorial) in TUTORIALS.iter().enumerate() {
                        let mark = if state.tutorial.finished.get(i).copied().unwrap_or(false) {
                            "✓ "
                        } else {
                            ""
                        };
                        let title = format!("{mark}{}", lang.tr(&format!("tutorial.{}", tutorial.id)));
                        if ui.selectable_label(i == t, title).clicked() {
                            start = Some(i);
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_min_width(380.0);
                    ui.heading(lang.tr(&format!("tutorial.{}", tutorial.id)));
                    ui.weak(
                        lang.tr("tutorial.step")
                            .replace("{n}", &(s + 1).to_string())
                            .replace("{total}", &tutorial.steps.len().to_string()),
                    );
                    ui.add_space(6.0);
                    ui.strong(lang.tr(&format!("tutorial.{}.{}", tutorial.id, step.id)));
                    ui.label(lang.tr(&format!("tutorial.{}.{}.body", tutorial.id, step.id)));
                    ui.add_space(6.0);
                    match done {
                        Some(true) => {
                            ui.colored_label(
                                egui::Color32::from_rgb(90, 190, 110),
                                format!("✓ {}", lang.tr("tutorial.done")),
                            );
                        }
                        Some(false) => {
                            ui.weak(lang.tr("tutorial.waiting"));
                        }
                        None => {}
                    }
                    if let Some(h) = step.help {
                        if ui
                            .add_enabled(done != Some(true), egui::Button::new(lang.tr("tutorial.show_me")))
                            .clicked()
                        {
                            help = Some(h);
                        }
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(s > 0, egui::Button::new(lang.tr("tutorial.back")))
                            .clicked()
                        {
                            go = Some(s - 1);
                        }
                        let last = s + 1 == tutorial.steps.len();
                        let label = lang.tr(if last { "tutorial.finish" } else { "tutorial.next" });
                        let next = egui::Button::new(label).selected(done == Some(true));
                        if ui.add(next).clicked() {
                            if last {
                                finish = true;
                            } else {
                                go = Some(s + 1);
                            }
                        }
                    });
                });
            });
        });
    if !open {
        state.tutorial.open = false;
    }
    if let Some(i) = start {
        state.start_tutorial(i);
    } else if finish {
        state.finish_tutorial();
    } else if let Some(i) = go {
        state.go_to_step(i);
    }
    help
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn every_tutorial_is_translated() {
        for language in Language::ALL {
            for key in [
                "tutorial.title",
                "tutorial.step",
                "tutorial.done",
                "tutorial.waiting",
                "tutorial.show_me",
            ] {
                assert_ne!(language.tr(key), key, "{language:?}: {key}");
            }
            for tutorial in TUTORIALS {
                let title = format!("tutorial.{}", tutorial.id);
                assert_ne!(language.tr(&title), title, "{language:?}: {title}");
                for step in tutorial.steps {
                    for key in [
                        format!("tutorial.{}.{}", tutorial.id, step.id),
                        format!("tutorial.{}.{}.body", tutorial.id, step.id),
                    ] {
                        assert_ne!(language.tr(&key), key, "{language:?}: {key}");
                    }
                }
            }
        }
        // Japanese is not English.
        assert_ne!(
            Language::Japanese.tr("tutorial.rig.auto.body"),
            Language::English.tr("tutorial.rig.auto.body")
        );
    }

    #[test]
    fn steps_notice_what_was_done() {
        let mut state = EditorState::default();
        state.open_tutorials();
        let paint = TUTORIALS.iter().position(|t| t.id == "paint").unwrap();
        state.start_tutorial(paint);
        state.select_tool(ToolId::Eraser);
        assert_eq!(state.tutorial_step_done(), Some(false));
        state.select_tool(ToolId::Brush);
        assert_eq!(state.tutorial_step_done(), Some(true), "the brush is picked");

        state.go_to_step(2);
        assert_eq!(state.tutorial_step_done(), Some(false));
        state.handle_action(Action::AddLayer);
        assert_eq!(
            state.tutorial_step_done(),
            Some(true),
            "a layer was added since the step began"
        );

        let tour = TUTORIALS.iter().position(|t| t.id == "tour").unwrap();
        state.start_tutorial(tour);
        assert_eq!(
            state.tutorial_step_done(),
            None,
            "reading steps have nothing to check"
        );
        state.go_to_step(1);
        state.set_workspace(Workspace::Rigging);
        assert_eq!(state.tutorial_step_done(), Some(true));
    }

    #[test]
    fn finishing_moves_on() {
        let mut state = EditorState::default();
        state.open_tutorials();
        state.start_tutorial(0);
        let steps = TUTORIALS[0].steps.len();
        state.go_to_step(steps - 1);
        state.finish_tutorial();
        assert!(state.tutorial.finished[0]);
        assert_eq!((state.tutorial.tutorial, state.tutorial.step), (1, 0));
    }
}
