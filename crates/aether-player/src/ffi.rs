//! The C ABI.
//!
//! One set of functions serves every non-Rust host: C and C++ link the
//! shared library, C# (Unity), Swift, Kotlin, Python and friends call it
//! through their foreign-function interfaces, and the same exports make up
//! the WebAssembly module used by the JavaScript player. The header is
//! `include/aether_player.h`.
//!
//! Conventions:
//!
//! * A player is an opaque pointer from [`aether_player_new`], released with
//!   [`aether_player_free`]. Every function accepts null and then does
//!   nothing and returns zero, `-1` or null.
//! * Strings are UTF-8, returned as a pointer plus a length written through
//!   `len` (which may be null). They are also NUL-terminated, so C can use
//!   them directly. Pointers stay valid until the player is freed; event
//!   names only until the next tick.
//! * Indices are `u32`. Out-of-range indices are ignored.
//! * Nothing here panics across the boundary: failures are reported through
//!   return values and [`aether_last_error`].

use crate::model::Model;
use crate::player::{DrawItem, Player, Stage};
use crate::tracking::{FaceFrame, BLENDSHAPES};
use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

/// Version of this C API. Bumped on any incompatible change.
pub const API_VERSION: u32 = 1;

thread_local! {
    static LAST_ERROR: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn set_error(message: &str) {
    LAST_ERROR.with(|e| {
        let mut e = e.borrow_mut();
        e.clear();
        e.extend_from_slice(message.as_bytes());
        e.push(0);
    });
}

fn c_string(s: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(s.len() + 1);
    bytes.extend_from_slice(s.as_bytes());
    bytes.push(0);
    bytes
}

/// Write a cached string out through the C calling convention.
///
/// # Safety
/// `len` must be null or valid for a write.
unsafe fn out_str(bytes: Option<&Vec<u8>>, len: *mut usize) -> *const u8 {
    let (p, n) = match bytes {
        Some(b) => (b.as_ptr(), b.len().saturating_sub(1)),
        None => (ptr::null(), 0),
    };
    if !len.is_null() {
        *len = n;
    }
    p
}

/// A player plus the flat buffers and strings the C API lends out.
pub struct AetherPlayer {
    player: Player,
    uvs: Vec<Vec<f32>>,
    indices: Vec<Vec<u32>>,
    parameter_names: Vec<Vec<u8>>,
    motion_names: Vec<Vec<u8>>,
    expression_names: Vec<Vec<u8>>,
    hotkey_names: Vec<Vec<u8>>,
    hotkey_keys: Vec<Vec<u8>>,
    part_names: Vec<Vec<u8>>,
    texture_files: Vec<Vec<u8>>,
    event_names: Vec<Vec<u8>>,
}

impl AetherPlayer {
    fn new(player: Player) -> Self {
        let model = player.model();
        let names = |it: &mut dyn Iterator<Item = &str>| it.map(c_string).collect();
        Self {
            uvs: model
                .parts
                .iter()
                .map(|p| p.uvs.iter().flat_map(|uv| [uv.x, uv.y]).collect())
                .collect(),
            indices: model
                .parts
                .iter()
                .map(|p| p.triangles.iter().flatten().copied().collect())
                .collect(),
            parameter_names: names(&mut model.rig.parameters.iter().map(|p| p.name.as_str())),
            motion_names: names(&mut model.rig.motions.iter().map(|m| m.name.as_str())),
            expression_names: names(&mut model.rig.expressions.iter().map(|e| e.name.as_str())),
            hotkey_names: model.rig.hotkeys.iter().map(|h| c_string(&h.label())).collect(),
            hotkey_keys: model
                .rig
                .hotkeys
                .iter()
                .map(|h| c_string(&h.keys.to_string()))
                .collect(),
            part_names: names(&mut model.parts.iter().map(|p| p.name.as_str())),
            texture_files: names(&mut model.textures.iter().map(|t| t.file.as_str())),
            event_names: Vec::new(),
            player,
        }
    }
}

/// Borrow the player behind a handle.
///
/// # Safety
/// `p` must be null or a live handle from [`aether_player_new`].
unsafe fn get<'a>(p: *const AetherPlayer) -> Option<&'a AetherPlayer> {
    p.as_ref()
}

/// # Safety
/// As [`get`], and the handle must not be aliased.
unsafe fn get_mut<'a>(p: *mut AetherPlayer) -> Option<&'a mut AetherPlayer> {
    p.as_mut()
}

// ---- memory and errors ------------------------------------------------------

/// Version of the C API ([`API_VERSION`]).
#[no_mangle]
pub extern "C" fn aether_api_version() -> u32 {
    API_VERSION
}

/// Alignment of [`aether_alloc`] blocks: enough for any value the API
/// reads or writes through a pointer.
const ALLOC_ALIGN: usize = 8;

fn alloc_layout(len: usize) -> Option<std::alloc::Layout> {
    std::alloc::Layout::from_size_align(len.max(1), ALLOC_ALIGN).ok()
}

/// Allocate `len` bytes, 8-byte aligned, for passing data in (WebAssembly
/// hosts use this to place `model.json` in module memory, and as scratch
/// space for out-parameters). Release with [`aether_dealloc`].
#[no_mangle]
pub extern "C" fn aether_alloc(len: usize) -> *mut u8 {
    match alloc_layout(len) {
        // SAFETY: the layout has a non-zero size.
        Some(layout) => unsafe { std::alloc::alloc(layout) },
        None => ptr::null_mut(),
    }
}

/// Release memory from [`aether_alloc`].
///
/// # Safety
/// `ptr` and `len` must come from one [`aether_alloc`] call.
#[no_mangle]
pub unsafe extern "C" fn aether_dealloc(ptr: *mut u8, len: usize) {
    if let (false, Some(layout)) = (ptr.is_null(), alloc_layout(len)) {
        std::alloc::dealloc(ptr, layout);
    }
}

/// The message for the most recent failure on this thread.
///
/// # Safety
/// `len` must be null or valid for a write.
#[no_mangle]
pub unsafe extern "C" fn aether_last_error(len: *mut usize) -> *const u8 {
    LAST_ERROR.with(|e| {
        let e = e.borrow();
        let n = e.len().saturating_sub(1);
        if !len.is_null() {
            *len = n;
        }
        if e.is_empty() {
            ptr::null()
        } else {
            // The buffer lives in a thread-local that outlives the call; it
            // is only replaced by the next failure on this thread.
            e.as_ptr()
        }
    })
}

// ---- lifetime --------------------------------------------------------------

/// Load a player from the bytes of `model.json`. Returns null on failure
/// (see [`aether_last_error`]).
///
/// # Safety
/// `json` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn aether_player_new(json: *const u8, len: usize) -> *mut AetherPlayer {
    if json.is_null() {
        set_error("no model data");
        return ptr::null_mut();
    }
    let bytes = std::slice::from_raw_parts(json, len);
    let result = catch_unwind(|| {
        let text = std::str::from_utf8(bytes).map_err(|_| "model.json is not UTF-8".to_string())?;
        let model = Model::from_json(text).map_err(|e| e.to_string())?;
        Player::new(model).map_err(|e| e.to_string())
    });
    match result {
        Ok(Ok(player)) => Box::into_raw(Box::new(AetherPlayer::new(player))),
        Ok(Err(message)) => {
            set_error(&message);
            ptr::null_mut()
        }
        Err(_) => {
            set_error("internal error while loading the model");
            ptr::null_mut()
        }
    }
}

/// Release a player.
///
/// # Safety
/// `p` must be null or a live handle, and must not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn aether_player_free(p: *mut AetherPlayer) {
    if !p.is_null() {
        drop(Box::from_raw(p));
    }
}

// ---- canvas and textures ---------------------------------------------------

/// Canvas width in pixels.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_width(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.player.model().width).unwrap_or(0)
}

/// Canvas height in pixels.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_height(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.player.model().height).unwrap_or(0)
}

/// Number of texture pages.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_texture_count(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.texture_files.len() as u32).unwrap_or(0)
}

/// File name of a texture page, relative to `model.json`.
///
/// # Safety
/// `p` must be null or a live handle; `len` null or writable.
#[no_mangle]
pub unsafe extern "C" fn aether_player_texture_file(
    p: *const AetherPlayer,
    index: u32,
    len: *mut usize,
) -> *const u8 {
    out_str(get(p).and_then(|h| h.texture_files.get(index as usize)), len)
}

// ---- parameters ------------------------------------------------------------

/// Number of parameters.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_parameter_count(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.parameter_names.len() as u32).unwrap_or(0)
}

/// A parameter's name.
///
/// # Safety
/// `p` must be null or a live handle; `len` null or writable.
#[no_mangle]
pub unsafe extern "C" fn aether_player_parameter_name(
    p: *const AetherPlayer,
    index: u32,
    len: *mut usize,
) -> *const u8 {
    out_str(get(p).and_then(|h| h.parameter_names.get(index as usize)), len)
}

/// Index of the parameter named by `name` (`len` bytes of UTF-8), or `-1`.
///
/// # Safety
/// `p` must be null or a live handle; `name` must point to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn aether_player_parameter_find(
    p: *const AetherPlayer,
    name: *const u8,
    len: usize,
) -> i32 {
    let (Some(h), false) = (get(p), name.is_null()) else {
        return -1;
    };
    let name = std::slice::from_raw_parts(name, len);
    std::str::from_utf8(name)
        .ok()
        .and_then(|n| h.player.parameter_index(n))
        .map(|i| i as i32)
        .unwrap_or(-1)
}

/// Write a parameter's range and default into `out[0..3]` as
/// `min, max, default`. Returns 0 for a bad index.
///
/// # Safety
/// `p` must be null or a live handle; `out` must be writable for 3 floats.
#[no_mangle]
pub unsafe extern "C" fn aether_player_parameter_range(
    p: *const AetherPlayer,
    index: u32,
    out: *mut f32,
) -> u32 {
    let Some(param) = get(p).and_then(|h| h.player.parameters().get(index as usize)) else {
        return 0;
    };
    if out.is_null() {
        return 0;
    }
    *out = param.min;
    *out.add(1) = param.max;
    *out.add(2) = param.default;
    1
}

/// A parameter's value as last drawn.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_parameter_value(p: *const AetherPlayer, index: u32) -> f32 {
    get(p)
        .map(|h| h.player.parameter_value(index as usize))
        .unwrap_or(0.0)
}

/// Set a parameter's base value; takes effect at the next tick or update.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_set_parameter(p: *mut AetherPlayer, index: u32, value: f32) {
    if let Some(h) = get_mut(p) {
        h.player.set_parameter(index as usize, value);
    }
}

// ---- motions, expressions and inputs ---------------------------------------

/// Number of motions.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_motion_count(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.motion_names.len() as u32).unwrap_or(0)
}

/// A motion's name.
///
/// # Safety
/// `p` must be null or a live handle; `len` null or writable.
#[no_mangle]
pub unsafe extern "C" fn aether_player_motion_name(
    p: *const AetherPlayer,
    index: u32,
    len: *mut usize,
) -> *const u8 {
    out_str(get(p).and_then(|h| h.motion_names.get(index as usize)), len)
}

/// A motion's length in seconds.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_motion_duration(p: *const AetherPlayer, index: u32) -> f32 {
    get(p)
        .and_then(|h| h.player.model().rig.motions.get(index as usize))
        .map(|m| m.duration)
        .unwrap_or(0.0)
}

/// Start a motion (`additive` non-zero layers it on top). Returns 0 for a
/// bad index.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_play_motion(p: *mut AetherPlayer, index: u32, additive: u32) -> u32 {
    get_mut(p)
        .map(|h| h.player.play_motion(index as usize, additive != 0) as u32)
        .unwrap_or(0)
}

/// Fade every motion out.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_stop_motions(p: *mut AetherPlayer) {
    if let Some(h) = get_mut(p) {
        h.player.stop_motions();
    }
}

/// 1 while any motion is playing or fading.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_is_playing(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.player.is_playing() as u32).unwrap_or(0)
}

/// Number of expressions.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_expression_count(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.expression_names.len() as u32).unwrap_or(0)
}

/// An expression's name.
///
/// # Safety
/// `p` must be null or a live handle; `len` null or writable.
#[no_mangle]
pub unsafe extern "C" fn aether_player_expression_name(
    p: *const AetherPlayer,
    index: u32,
    len: *mut usize,
) -> *const u8 {
    out_str(get(p).and_then(|h| h.expression_names.get(index as usize)), len)
}

/// Fade to an expression, or out of all of them for a negative index.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_set_expression(p: *mut AetherPlayer, index: i32) {
    if let Some(h) = get_mut(p) {
        h.player.set_expression(usize::try_from(index).ok());
    }
}

/// Switch an expression on or off, leaving the others.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_toggle_expression(p: *mut AetherPlayer, index: u32) {
    if let Some(h) = get_mut(p) {
        h.player.toggle_expression(index as usize);
    }
}

/// 1 while an expression is switched on.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_expression_active(p: *const AetherPlayer, index: u32) -> u32 {
    get(p)
        .map(|h| h.player.active_expressions().contains(&(index as usize)) as u32)
        .unwrap_or(0)
}

/// Number of hotkeys.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_hotkey_count(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.hotkey_names.len() as u32).unwrap_or(0)
}

/// A hotkey's name (its own, or what it plays).
///
/// # Safety
/// `p` must be null or a live handle; `len` null or writable.
#[no_mangle]
pub unsafe extern "C" fn aether_player_hotkey_name(
    p: *const AetherPlayer,
    index: u32,
    len: *mut usize,
) -> *const u8 {
    out_str(get(p).and_then(|h| h.hotkey_names.get(index as usize)), len)
}

/// A hotkey's keys, written like `Shift+1`.
///
/// # Safety
/// `p` must be null or a live handle; `len` null or writable.
#[no_mangle]
pub unsafe extern "C" fn aether_player_hotkey_keys(
    p: *const AetherPlayer,
    index: u32,
    len: *mut usize,
) -> *const u8 {
    out_str(get(p).and_then(|h| h.hotkey_keys.get(index as usize)), len)
}

/// Carry out a hotkey as if its key were pressed. Returns 0 for a bad index
/// or a missing motion or expression.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_trigger_hotkey(p: *mut AetherPlayer, index: u32) -> u32 {
    get_mut(p)
        .map(|h| h.player.trigger_hotkey(index as usize) as u32)
        .unwrap_or(0)
}

/// Modifier bits for [`aether_player_press_key`].
pub const MODIFIER_CTRL: u32 = 1;
/// Shift.
pub const MODIFIER_SHIFT: u32 = 2;
/// Alt.
pub const MODIFIER_ALT: u32 = 4;

/// A key was pressed: `key` names it in UTF-8 (`1`, `Digit1`, `a`, `F5`,
/// `Space`…), `modifiers` combines 1 (Ctrl), 2 (Shift) and 4 (Alt).
/// Returns the index of the hotkey that fired, or -1.
///
/// # Safety
/// `p` must be null or a live handle; `key` null or valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn aether_player_press_key(
    p: *mut AetherPlayer,
    key: *const u8,
    len: usize,
    modifiers: u32,
) -> i32 {
    let (Some(h), false) = (get_mut(p), key.is_null()) else {
        return -1;
    };
    let Ok(key) = std::str::from_utf8(std::slice::from_raw_parts(key, len)) else {
        return -1;
    };
    h.player
        .press_key(
            key,
            modifiers & MODIFIER_CTRL != 0,
            modifiers & MODIFIER_SHIFT != 0,
            modifiers & MODIFIER_ALT != 0,
        )
        .map(|i| i as i32)
        .unwrap_or(-1)
}

/// Look towards `(x, y)` in `-1..=1`, y up.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_look_at(p: *mut AetherPlayer, x: f32, y: f32) {
    if let Some(h) = get_mut(p) {
        h.player.look_at(Some((x, y)));
    }
}

/// Look straight ahead.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_look_ahead(p: *mut AetherPlayer) {
    if let Some(h) = get_mut(p) {
        h.player.look_at(None);
    }
}

/// Voice loudness (`0..=1`) and brightness (`-1..=1`) for lip sync.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_set_audio(p: *mut AetherPlayer, level: f32, brightness: f32) {
    if let Some(h) = get_mut(p) {
        h.player.set_audio(level, brightness);
    }
}

/// Switch a stage on or off: 0 motions, 1 behaviours, 2 drivers,
/// 3 physics, 4 jiggle.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_set_stage(p: *mut AetherPlayer, stage: u32, enabled: u32) {
    if let (Some(h), Some(stage)) = (get_mut(p), Stage::from_code(stage)) {
        h.player.set_stage(stage, enabled != 0);
    }
}

// ---- time ------------------------------------------------------------------

fn guarded(h: &mut AetherPlayer, f: impl FnOnce(&mut Player)) {
    if catch_unwind(AssertUnwindSafe(|| f(&mut h.player))).is_err() {
        set_error("internal error while updating the model");
    }
}

/// Advance `dt` seconds and recompute the pose. Event names from the
/// previous tick are released.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_tick(p: *mut AetherPlayer, dt: f32) {
    if let Some(h) = get_mut(p) {
        guarded(h, |player| player.tick(dt));
        h.event_names = h.player.events().iter().map(|e| c_string(&e.name)).collect();
    }
}

/// Recompute the pose without advancing time.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_update(p: *mut AetherPlayer) {
    if let Some(h) = get_mut(p) {
        guarded(h, Player::update);
    }
}

/// Stop everything and return to the default pose.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_reset(p: *mut AetherPlayer) {
    if let Some(h) = get_mut(p) {
        guarded(h, Player::reset);
    }
}

/// Number of motion events fired by the last tick.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_event_count(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.event_names.len() as u32).unwrap_or(0)
}

/// Name of a fired event (valid until the next tick).
///
/// # Safety
/// `p` must be null or a live handle; `len` null or writable.
#[no_mangle]
pub unsafe extern "C" fn aether_player_event_name(
    p: *const AetherPlayer,
    index: u32,
    len: *mut usize,
) -> *const u8 {
    out_str(get(p).and_then(|h| h.event_names.get(index as usize)), len)
}

/// Motion that fired an event.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_event_motion(p: *const AetherPlayer, index: u32) -> i32 {
    get(p)
        .and_then(|h| h.player.events().get(index as usize))
        .map(|e| e.motion as i32)
        .unwrap_or(-1)
}

// ---- face tracking ---------------------------------------------------------

/// Number of blend shapes a face sample carries (52).
#[no_mangle]
pub extern "C" fn aether_blendshape_count() -> u32 {
    BLENDSHAPES.len() as u32
}

/// Name of blend shape `index`, in the order
/// [`aether_player_track_face`] expects (ARKit naming, as MediaPipe uses).
///
/// # Safety
/// `len` must be null or valid for a write.
#[no_mangle]
pub unsafe extern "C" fn aether_blendshape_name(index: u32, len: *mut usize) -> *const u8 {
    static NAMES: std::sync::OnceLock<Vec<Vec<u8>>> = std::sync::OnceLock::new();
    let names = NAMES.get_or_init(|| BLENDSHAPES.iter().map(|n| c_string(n)).collect());
    out_str(names.get(index as usize), len)
}

/// Feed one face-tracker sample: head angles in degrees in the tracked
/// person's frame (yaw toward their left, pitch up, roll toward their left
/// shoulder) and `count` blend-shape weights in
/// [`aether_blendshape_name`] order. Missing weights count as zero.
///
/// # Safety
/// `p` must be null or a live handle; `shapes` must be null or point to
/// `count` floats.
#[no_mangle]
pub unsafe extern "C" fn aether_player_track_face(
    p: *mut AetherPlayer,
    yaw: f32,
    pitch: f32,
    roll: f32,
    shapes: *const f32,
    count: u32,
) {
    let Some(h) = get_mut(p) else {
        return;
    };
    let mut frame = FaceFrame {
        yaw,
        pitch,
        roll,
        ..Default::default()
    };
    if !shapes.is_null() {
        let given = std::slice::from_raw_parts(shapes, count as usize);
        for (slot, value) in frame.shapes.iter_mut().zip(given) {
            *slot = *value;
        }
    }
    h.player.track_face(&frame);
}

/// Make the latest tracked face the neutral one.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_track_calibrate(p: *mut AetherPlayer) {
    if let Some(h) = get_mut(p) {
        h.player.calibrate_tracking();
    }
}

/// Stop following the tracker.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_track_stop(p: *mut AetherPlayer) {
    if let Some(h) = get_mut(p) {
        h.player.stop_tracking();
    }
}

/// 1 while a tracker drives the model.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_is_tracking(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.player.is_tracking() as u32).unwrap_or(0)
}

/// Tracking options: mirror (non-zero: move like a mirror image), smoothing
/// time constant in seconds, head gain, how much the body follows the head,
/// and mouth gain. Non-finite values leave that option unchanged.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_track_settings(
    p: *mut AetherPlayer,
    mirror: u32,
    smoothing: f32,
    head_gain: f32,
    body_follow: f32,
    mouth_gain: f32,
) {
    let Some(h) = get_mut(p) else {
        return;
    };
    let settings = h.player.tracking_settings_mut();
    settings.mirror = mirror != 0;
    let set = |slot: &mut f32, v: f32| {
        if v.is_finite() {
            *slot = v.max(0.0);
        }
    };
    set(&mut settings.smoothing, smoothing);
    set(&mut settings.head_gain, head_gain);
    set(&mut settings.body_follow, body_follow);
    set(&mut settings.mouth_gain, mouth_gain);
}

// ---- geometry --------------------------------------------------------------

/// Number of parts.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_part_count(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.part_names.len() as u32).unwrap_or(0)
}

/// A part's name.
///
/// # Safety
/// `p` must be null or a live handle; `len` null or writable.
#[no_mangle]
pub unsafe extern "C" fn aether_player_part_name(
    p: *const AetherPlayer,
    index: u32,
    len: *mut usize,
) -> *const u8 {
    out_str(get(p).and_then(|h| h.part_names.get(index as usize)), len)
}

/// Texture page a part samples.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_part_texture(p: *const AetherPlayer, index: u32) -> u32 {
    get(p)
        .and_then(|h| h.player.model().parts.get(index as usize))
        .map(|part| part.texture)
        .unwrap_or(0)
}

/// Number of vertices in a part.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_part_vertex_count(p: *const AetherPlayer, index: u32) -> u32 {
    get(p)
        .and_then(|h| h.uvs.get(index as usize))
        .map(|uv| (uv.len() / 2) as u32)
        .unwrap_or(0)
}

/// A part's texture coordinates, two floats per vertex. Constant for the
/// player's lifetime.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_part_uvs(p: *const AetherPlayer, index: u32) -> *const f32 {
    get(p)
        .and_then(|h| h.uvs.get(index as usize))
        .map(|v| v.as_ptr())
        .unwrap_or(ptr::null())
}

/// Number of triangle indices (three per triangle) in a part.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_part_index_count(p: *const AetherPlayer, index: u32) -> u32 {
    get(p)
        .and_then(|h| h.indices.get(index as usize))
        .map(|i| i.len() as u32)
        .unwrap_or(0)
}

/// A part's triangle indices. Constant for the player's lifetime.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_part_indices(p: *const AetherPlayer, index: u32) -> *const u32 {
    get(p)
        .and_then(|h| h.indices.get(index as usize))
        .map(|v| v.as_ptr())
        .unwrap_or(ptr::null())
}

/// A part's posed positions in document pixels, two floats per vertex.
/// Valid until the next tick, update or reset.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_part_positions(p: *const AetherPlayer, index: u32) -> *const f32 {
    match get(p) {
        Some(h) if (index as usize) < h.part_names.len() => h.player.positions(index as usize).as_ptr(),
        _ => ptr::null(),
    }
}

/// Number of draw calls at the current pose.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_draw_count(p: *const AetherPlayer) -> u32 {
    get(p).map(|h| h.player.draw_list().len() as u32).unwrap_or(0)
}

/// The draw calls, back to front. Valid until the next tick, update or
/// reset.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_draw_items(p: *const AetherPlayer) -> *const DrawItem {
    get(p)
        .map(|h| h.player.draw_list().as_ptr())
        .unwrap_or(ptr::null())
}

/// The topmost visible part under a document-space point, or `-1`.
///
/// # Safety
/// `p` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn aether_player_hit_test(p: *const AetherPlayer, x: f32, y: f32) -> i32 {
    get(p)
        .and_then(|h| h.player.hit_test(x, y))
        .map(|i| i as i32)
        .unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlendKind, Node, Part, Texture};
    use aether_core::math::Vec2;
    use aether_core::LayerId;

    fn model_json() -> String {
        let mut model = Model::new("ffi", 16, 16);
        model.textures.push(Texture {
            file: "texture_0.png".into(),
            width: 2,
            height: 2,
        });
        model.parts.push(Part {
            layer: LayerId(7),
            name: "Face".into(),
            texture: 0,
            vertices: vec![Vec2::new(0.0, 0.0), Vec2::new(8.0, 0.0), Vec2::new(0.0, 8.0)],
            uvs: vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)],
            triangles: vec![[0, 1, 2]],
            opacity: 1.0,
            blend: BlendKind::Multiply,
            transform: None,
        });
        model.tree.push(Node::Part {
            index: 0,
            part: 0,
            clipped: vec![],
        });
        model.to_json()
    }

    unsafe fn text(p: *const u8, len: usize) -> String {
        String::from_utf8(std::slice::from_raw_parts(p, len).to_vec()).unwrap()
    }

    #[test]
    fn the_c_api_loads_and_draws_a_model() {
        let json = model_json();
        unsafe {
            let p = aether_player_new(json.as_ptr(), json.len());
            assert!(!p.is_null());
            assert_eq!(aether_player_width(p), 16);
            assert_eq!(aether_player_part_count(p), 1);
            let mut len = 0usize;
            let name = aether_player_part_name(p, 0, &mut len);
            assert_eq!(text(name, len), "Face");
            assert_eq!(*name.add(len), 0, "strings are NUL-terminated");
            let file = aether_player_texture_file(p, 0, &mut len);
            assert_eq!(text(file, len), "texture_0.png");

            aether_player_tick(p, 1.0 / 60.0);
            assert_eq!(aether_player_draw_count(p), 1);
            let item = &*aether_player_draw_items(p);
            assert_eq!(item.part, 0);
            assert_eq!(item.mask, -1);
            assert_eq!(item.blend, BlendKind::Multiply as u32);
            let positions = std::slice::from_raw_parts(aether_player_part_positions(p, 0), 6);
            assert_eq!(positions, &[0.0, 0.0, 8.0, 0.0, 0.0, 8.0]);
            assert_eq!(aether_player_part_index_count(p, 0), 3);
            assert_eq!(aether_player_hit_test(p, 1.0, 1.0), 0);
            assert_eq!(aether_player_hit_test(p, 7.0, 7.0), -1);

            // Out-of-range and null inputs are harmless.
            assert!(aether_player_part_positions(p, 5).is_null());
            assert!(aether_player_parameter_name(p, 3, &mut len).is_null());
            assert_eq!(len, 0);
            aether_player_set_parameter(p, 99, 1.0);
            assert_eq!(aether_player_play_motion(p, 0, 0), 0);
            aether_player_tick(ptr::null_mut(), 0.1);
            assert_eq!(aether_player_draw_count(ptr::null()), 0);
            aether_player_free(p);
        }
    }

    #[test]
    fn hotkeys_through_the_c_api() {
        use aether_rig::{Hotkey, HotkeyAction, KeyChord, Motion};
        let mut model = Model::from_json(&model_json()).unwrap();
        model.rig.motions.push(Motion::new("Wave", 1.0, 30.0));
        model.rig.hotkeys.push(Hotkey::new(
            KeyChord::parse("Ctrl+W").unwrap(),
            HotkeyAction::PlayMotion("Wave".into()),
        ));
        let json = model.to_json();
        unsafe {
            let p = aether_player_new(json.as_ptr(), json.len());
            assert!(!p.is_null());
            assert_eq!(aether_player_hotkey_count(p), 1);
            let mut len = 0usize;
            let name = aether_player_hotkey_name(p, 0, &mut len);
            assert_eq!(text(name, len), "Wave");
            let keys = aether_player_hotkey_keys(p, 0, &mut len);
            assert_eq!(text(keys, len), "Ctrl+W");
            let w = b"w";
            assert_eq!(
                aether_player_press_key(p, w.as_ptr(), 1, 0),
                -1,
                "Ctrl is part of it"
            );
            assert_eq!(aether_player_press_key(p, w.as_ptr(), 1, MODIFIER_CTRL), 0);
            assert_eq!(aether_player_is_playing(p), 1);
            assert_eq!(aether_player_trigger_hotkey(p, 3), 0);
            assert_eq!(aether_player_press_key(p, ptr::null(), 0, 0), -1);
            aether_player_free(p);
            assert_eq!(aether_player_hotkey_count(ptr::null()), 0);
        }
    }

    #[test]
    fn load_failures_explain_themselves() {
        let bad = b"{\"format\":\"aether-model\"}";
        unsafe {
            let p = aether_player_new(bad.as_ptr(), bad.len());
            assert!(p.is_null());
            let mut len = 0usize;
            let message = aether_last_error(&mut len);
            assert!(text(message, len).contains("not a valid model"));
        }
    }

    #[test]
    fn face_samples_arrive_through_the_c_api() {
        unsafe {
            assert_eq!(aether_blendshape_count(), 52);
            let mut len = 0usize;
            let name = aether_blendshape_name(24, &mut len);
            assert_eq!(text(name, len), "jawOpen");
            assert!(aether_blendshape_name(52, &mut len).is_null());

            let json = model_json();
            let p = aether_player_new(json.as_ptr(), json.len());
            assert_eq!(aether_player_is_tracking(p), 0);
            let shapes = [0.5f32; 30];
            aether_player_track_face(p, 10.0, 0.0, 0.0, shapes.as_ptr(), shapes.len() as u32);
            aether_player_track_face(p, 10.0, 0.0, 0.0, ptr::null(), 0);
            assert_eq!(aether_player_is_tracking(p), 1);
            aether_player_track_settings(p, 0, f32::NAN, 1.5, 0.0, 1.0);
            aether_player_track_calibrate(p);
            aether_player_tick(p, 0.016);
            aether_player_track_stop(p);
            assert_eq!(aether_player_is_tracking(p), 0);
            aether_player_track_face(ptr::null_mut(), 0.0, 0.0, 0.0, shapes.as_ptr(), 52);
            aether_player_free(p);
        }
    }

    #[test]
    fn allocations_round_trip() {
        let p = aether_alloc(64);
        assert!(!p.is_null());
        assert_eq!(p as usize % ALLOC_ALIGN, 0);
        unsafe {
            p.write_bytes(7, 64);
            aether_dealloc(p, 64);
        }
    }
}
