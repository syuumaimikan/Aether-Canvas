//! # aether-rig
//!
//! 2D rigging and animation: the part of Aether Canvas that makes painted
//! artwork move.
//!
//! The model follows the proven parameter-and-keyform approach of 2D rigging
//! and extends it where artists hit its limits:
//!
//! * [`param`] — named, ranged parameters: the single currency of pose.
//! * [`keyform`] — N-dimensional keyform grids with linear or smooth
//!   (Catmull-Rom) interpolation, plus additive blend shapes.
//! * [`mesh`] — triangle meshes bound directly to raster layers, with keyed
//!   offsets, opacity, multiply/screen tint and draw order, skin weights,
//!   jiggle and glue.
//! * [`deformer`] — warp lattices (bilinear or bicubic) and rotation pivots,
//!   nestable to any depth.
//! * [`skeleton`] — bones with parameter-driven FK, analytic and CCD inverse
//!   kinematics, and linear blend skinning.
//! * [`physics`] — fixed-step pendulum chains with gravity, wind, stiffness,
//!   angle limits and colliders.
//! * [`expr`] / [`driver`] — a sandboxed expression language computing
//!   parameters from other parameters and time.
//! * [`motion`] — keyframed clips with rich easing, a layered animator with
//!   crossfades, and expression presets.
//! * [`behaviour`] / [`audio`] — auto-blink, breathing, look-at and lip sync
//!   (live, or baked from a WAV file).
//! * [`automesh`] / [`generate`] — automatic meshing, 3D head-turn
//!   generation, keyform mirroring and standard physics setup.
//! * [`rig`] / [`pose`] / [`runtime`] — the rig container, pure evaluation
//!   to a [`RigPose`], and the time-dependent runtime.
//!
//! Nothing here touches the GPU, the filesystem or the layer tree: a rig is
//! plain data plus pure functions, which is what makes it testable in
//! isolation and embeddable in a player.

pub mod audio;
pub mod automesh;
pub mod behaviour;
pub mod deformer;
pub mod driver;
pub mod expr;
pub(crate) mod flat;
pub mod generate;
pub mod geom;
pub mod keyform;
pub mod mesh;
pub mod motion;
pub mod param;
pub mod physics;
pub mod pose;
pub mod rig;
pub mod runtime;
pub mod skeleton;

pub use behaviour::Behaviours;
pub use deformer::{Deformer, DeformerKind, RotationDeformer, RotationForm, WarpDeformer, WarpForm};
pub use driver::Driver;
pub use keyform::{BlendShape, KeyAxis, KeyInterpolation, KeyformGrid};
pub use mesh::{ArtMesh, Jiggle, MeshForm, Skin};
pub use motion::{Easing, Expression, Keyframe, Motion, Track};
pub use param::{ParamValues, Parameter, StandardParam};
pub use physics::PhysicsGroup;
pub use pose::{MeshPose, PoseChange, RigPose};
pub use rig::{Evaluator, NodeRef, Rig, RigNode};
pub use runtime::RigRuntime;
pub use skeleton::{Bone, BoneForm, IkConstraint};
