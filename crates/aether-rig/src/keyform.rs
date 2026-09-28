//! Keyform grids.
//!
//! A *keyform* is a complete shape of one rig object — every vertex of a mesh,
//! every lattice point of a warp, the angle of a rotation — recorded at one
//! combination of parameter values. A [`KeyformGrid`] stores one keyform per
//! corner of an N-dimensional grid: the axes are parameters, each with its own
//! list of key values, and the grid holds the product of their key counts.
//!
//! Evaluating the grid interpolates between the keyforms around the current
//! parameter values. Two interpolators are offered:
//!
//! * [`KeyInterpolation::Linear`] — multilinear, the behaviour artists coming
//!   from other rigging tools expect;
//! * [`KeyInterpolation::Smooth`] — Catmull-Rom across keys, so a head turned
//!   through three keys follows an arc instead of two straight segments and
//!   motion never visibly "kinks" as it passes a key.
//!
//! Any number of axes is allowed. For combinations that would otherwise need
//! an explosion of keyforms, objects also carry additive *blend shapes* (see
//! [`BlendShape`]): independent one-axis grids of deltas that are summed on
//! top of the base grid.
//!
//! Structural edits keep the look stable. Inserting a key between two existing
//! ones fills the new keyforms with what the grid already evaluated to there,
//! and adding a parameter axis copies the current forms along it, so neither
//! changes anything on screen until the artist starts editing.

use aether_core::{AetherError, ParameterId, Result};
use serde::{Deserialize, Serialize};

/// Tolerance for "the parameter is sitting on this key".
pub const KEY_EPSILON: f32 = 1e-4;

/// Data that can be interpolated between keyforms.
///
/// Implementations are plain weighted sums: `zeroed` produces the additive
/// identity with the same shape as `self`, and `add_scaled` accumulates.
pub trait Blend: Clone {
    /// A value of the same shape with every component zero.
    fn zeroed(&self) -> Self;
    /// `self += other * weight`.
    fn add_scaled(&mut self, other: &Self, weight: f32);
}

/// How values between keys are computed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyInterpolation {
    /// Straight lines between neighbouring keys.
    #[default]
    Linear,
    /// A Catmull-Rom curve through the keys (C1 continuous).
    Smooth,
}

impl KeyInterpolation {
    /// Both modes, in menu order.
    pub const ALL: [KeyInterpolation; 2] = [KeyInterpolation::Linear, KeyInterpolation::Smooth];
}

/// What an axis needs to know about its parameter's current state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisInput {
    /// Current value.
    pub value: f32,
    /// `(min, span)` when the parameter wraps.
    pub cycle: Option<(f32, f32)>,
}

/// Something that can report parameter values to a grid.
pub trait ParamSource {
    /// The state of `id`, or `None` when no such parameter exists.
    fn sample(&self, id: ParameterId) -> Option<AxisInput>;
}

impl<F: Fn(ParameterId) -> Option<AxisInput>> ParamSource for F {
    fn sample(&self, id: ParameterId) -> Option<AxisInput> {
        self(id)
    }
}

/// One dimension of a grid: a parameter and the values it is keyed at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyAxis {
    /// The driving parameter.
    pub param: ParameterId,
    /// Key values, strictly increasing, never empty.
    pub keys: Vec<f32>,
}

impl KeyAxis {
    /// An axis with the given keys; they are sorted and de-duplicated.
    pub fn new(param: ParameterId, keys: impl IntoIterator<Item = f32>) -> Result<Self> {
        let mut keys: Vec<f32> = keys.into_iter().filter(|k| k.is_finite()).collect();
        keys.sort_by(f32::total_cmp);
        keys.dedup_by(|a, b| (*a - *b).abs() < KEY_EPSILON);
        if keys.is_empty() {
            return Err(AetherError::rig("a parameter axis needs at least one key"));
        }
        Ok(Self { param, keys })
    }

    /// Index of the key `value` sits on, if any.
    pub fn key_at(&self, value: f32) -> Option<usize> {
        self.keys.iter().position(|k| (k - value).abs() <= KEY_EPSILON)
    }

    /// Index of the key closest to `value`.
    pub fn nearest_key(&self, value: f32) -> usize {
        let mut best = 0;
        let mut best_distance = f32::INFINITY;
        for (i, k) in self.keys.iter().enumerate() {
            let d = (k - value).abs();
            if d < best_distance {
                best = i;
                best_distance = d;
            }
        }
        best
    }

    /// Interpolation weights over this axis's keys for `input`.
    ///
    /// At most four `(key index, weight)` pairs are produced; they sum to one.
    pub fn weights(&self, input: Option<AxisInput>, mode: KeyInterpolation) -> AxisWeights {
        let n = self.keys.len();
        let mut out = AxisWeights::default();
        let Some(input) = input else {
            // A parameter that no longer exists pins the axis to its first key.
            out.push(0, 1.0);
            return out;
        };
        if n == 1 {
            out.push(0, 1.0);
            return out;
        }
        let first = self.keys[0];
        let last = self.keys[n - 1];

        // Locate the segment: `lo` → `hi` with local parameter `t`.
        let (lo, hi, t) = match input.cycle {
            Some((min, span)) if span > 0.0 => {
                let v = min + (input.value - min).rem_euclid(span);
                if v < first || v > last {
                    // The wrap-around segment from the last key to the first.
                    let gap = first + span - last;
                    let from_last = if v >= last { v - last } else { v + span - last };
                    let t = if gap > 1e-6 { from_last / gap } else { 0.0 };
                    (n - 1, 0, t.clamp(0.0, 1.0))
                } else {
                    segment(&self.keys, v)
                }
            }
            _ => segment(&self.keys, input.value.clamp(first, last)),
        };

        match mode {
            KeyInterpolation::Linear => {
                out.push(lo, 1.0 - t);
                out.push(hi, t);
            }
            KeyInterpolation::Smooth => {
                let cyclic = input.cycle.is_some();
                let neighbour = |index: usize, step: isize| -> usize {
                    let i = index as isize + step;
                    if cyclic {
                        i.rem_euclid(n as isize) as usize
                    } else {
                        i.clamp(0, n as isize - 1) as usize
                    }
                };
                // Catmull-Rom basis for P(lo-1), P(lo), P(hi), P(hi+1).
                let t2 = t * t;
                let t3 = t2 * t;
                let w = [
                    0.5 * (-t3 + 2.0 * t2 - t),
                    0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
                    0.5 * (-3.0 * t3 + 4.0 * t2 + t),
                    0.5 * (t3 - t2),
                ];
                out.push(neighbour(lo, -1), w[0]);
                out.push(lo, w[1]);
                out.push(hi, w[2]);
                out.push(neighbour(hi, 1), w[3]);
            }
        }
        out
    }
}

/// Find the key segment containing `v` (already within the key range).
fn segment(keys: &[f32], v: f32) -> (usize, usize, f32) {
    let n = keys.len();
    for i in 0..n - 1 {
        if v <= keys[i + 1] {
            let span = keys[i + 1] - keys[i];
            let t = if span > 1e-9 { (v - keys[i]) / span } else { 0.0 };
            return (i, i + 1, t.clamp(0.0, 1.0));
        }
    }
    (n - 2, n - 1, 1.0)
}

/// Up to four weighted key indices along one axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AxisWeights {
    entries: [(usize, f32); 4],
    len: usize,
}

impl AxisWeights {
    fn push(&mut self, index: usize, weight: f32) {
        if weight == 0.0 && self.len > 0 {
            return;
        }
        // Merge duplicates (clamped Catmull-Rom neighbours at the ends).
        for entry in &mut self.entries[..self.len] {
            if entry.0 == index {
                entry.1 += weight;
                return;
            }
        }
        if self.len < 4 {
            self.entries[self.len] = (index, weight);
            self.len += 1;
        }
    }

    /// The `(key index, weight)` pairs.
    pub fn as_slice(&self) -> &[(usize, f32)] {
        &self.entries[..self.len]
    }
}

/// Keyforms laid out on a grid of parameter keys.
///
/// Forms are stored with axis 0 varying fastest. A grid with no axes holds a
/// single form that applies whatever the parameters are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyformGrid<T> {
    /// The parameter axes.
    pub axes: Vec<KeyAxis>,
    /// One form per grid corner; `forms.len()` is the product of key counts.
    pub forms: Vec<T>,
    /// How to interpolate between keys.
    #[serde(default)]
    pub interpolation: KeyInterpolation,
}

impl<T: Blend> KeyformGrid<T> {
    /// A grid with no axes: `rest` always applies.
    pub fn constant(rest: T) -> Self {
        Self {
            axes: Vec::new(),
            forms: vec![rest],
            interpolation: KeyInterpolation::Linear,
        }
    }

    /// Number of forms the axes call for.
    pub fn expected_len(&self) -> usize {
        self.axes.iter().map(|a| a.keys.len()).product()
    }

    /// Check the invariants a hand-edited or plugin-written file might break.
    pub fn validate(&self) -> Result<()> {
        if self.forms.is_empty() {
            return Err(AetherError::rig("a keyform grid has no forms"));
        }
        for axis in &self.axes {
            if axis.keys.is_empty() {
                return Err(AetherError::rig("a parameter axis has no keys"));
            }
            if axis.keys.windows(2).any(|w| w[1] <= w[0]) {
                return Err(AetherError::rig("parameter keys must be strictly increasing"));
            }
        }
        for (i, a) in self.axes.iter().enumerate() {
            if self.axes[i + 1..].iter().any(|b| b.param == a.param) {
                return Err(AetherError::rig("a parameter appears twice in one keyform grid"));
            }
        }
        if self.forms.len() != self.expected_len() {
            return Err(AetherError::rig(format!(
                "keyform grid holds {} forms but its keys call for {}",
                self.forms.len(),
                self.expected_len()
            )));
        }
        Ok(())
    }

    /// Distance in `forms` between neighbouring keys of each axis.
    pub fn strides(&self) -> Vec<usize> {
        let mut strides = Vec::with_capacity(self.axes.len());
        let mut stride = 1;
        for axis in &self.axes {
            strides.push(stride);
            stride *= axis.keys.len();
        }
        strides
    }

    /// Split a form index into per-axis key indices.
    pub fn coords_of(&self, mut index: usize) -> Vec<usize> {
        self.axes
            .iter()
            .map(|axis| {
                let k = index % axis.keys.len();
                index /= axis.keys.len();
                k
            })
            .collect()
    }

    /// Form index for per-axis key indices.
    pub fn index_of(&self, coords: &[usize]) -> usize {
        self.strides()
            .iter()
            .zip(coords)
            .map(|(stride, k)| stride * k)
            .sum()
    }

    /// Which axis is driven by `param`.
    pub fn axis_for(&self, param: ParameterId) -> Option<usize> {
        self.axes.iter().position(|a| a.param == param)
    }

    /// True when `param` drives this grid.
    pub fn uses(&self, param: ParameterId) -> bool {
        self.axis_for(param).is_some()
    }

    /// `(form index, weight)` pairs for the current parameter values.
    pub fn weights(&self, source: &dyn ParamSource) -> Vec<(usize, f32)> {
        let mut out = vec![(0usize, 1.0f32)];
        for (axis, stride) in self.axes.iter().zip(self.strides()) {
            let weights = axis.weights(source.sample(axis.param), self.interpolation);
            let mut next = Vec::with_capacity(out.len() * weights.as_slice().len());
            for &(base, bw) in &out {
                for &(k, kw) in weights.as_slice() {
                    let w = bw * kw;
                    if w != 0.0 {
                        next.push((base + k * stride, w));
                    }
                }
            }
            out = next;
        }
        out.retain(|(i, _)| *i < self.forms.len());
        out
    }

    /// Interpolate the form for the current parameter values.
    pub fn evaluate(&self, source: &dyn ParamSource) -> T {
        let weights = self.weights(source);
        let mut result = self.forms[0].zeroed();
        for (index, weight) in weights {
            result.add_scaled(&self.forms[index], weight);
        }
        result
    }

    /// The form the parameters currently sit exactly on, if every axis is at
    /// one of its keys. This is the form an edit at the current pose changes.
    pub fn form_at_keys(&self, source: &dyn ParamSource) -> Option<usize> {
        let mut coords = Vec::with_capacity(self.axes.len());
        for axis in &self.axes {
            let value = source.sample(axis.param).map(|s| s.value)?;
            coords.push(axis.key_at(value)?);
        }
        Some(self.index_of(&coords))
    }

    /// Axes whose parameter is *not* sitting on a key, for explaining why an
    /// edit was refused.
    pub fn axes_off_key(&self, source: &dyn ParamSource) -> Vec<ParameterId> {
        self.axes
            .iter()
            .filter(|axis| {
                source
                    .sample(axis.param)
                    .map(|s| axis.key_at(s.value).is_none())
                    .unwrap_or(true)
            })
            .map(|axis| axis.param)
            .collect()
    }

    /// Add a parameter axis. The current forms are copied to every new key,
    /// so the object looks the same until one of the new keyforms is edited.
    pub fn add_axis(&mut self, axis: KeyAxis) -> Result<()> {
        if self.uses(axis.param) {
            return Err(AetherError::rig("that parameter already drives this object"));
        }
        let count = axis.keys.len();
        let old = std::mem::take(&mut self.forms);
        let mut forms = Vec::with_capacity(old.len() * count);
        for _ in 0..count {
            forms.extend(old.iter().cloned());
        }
        self.forms = forms;
        self.axes.push(axis);
        Ok(())
    }

    /// Insert a key on an axis. The new keyforms are what the grid evaluated
    /// to at that value, so inserting a key never changes the pose.
    pub fn insert_key(&mut self, axis_index: usize, value: f32) -> Result<usize> {
        let axis = self
            .axes
            .get(axis_index)
            .ok_or_else(|| AetherError::rig("no such parameter axis"))?;
        if !value.is_finite() {
            return Err(AetherError::rig("a key must be a finite number"));
        }
        if let Some(existing) = axis.key_at(value) {
            return Ok(existing);
        }
        let position = axis
            .keys
            .iter()
            .position(|k| *k > value)
            .unwrap_or(axis.keys.len());

        // Evaluate along this axis only, at `value`, for every combination of
        // the other axes' keys.
        let one_axis = KeyAxis {
            param: axis.param,
            keys: axis.keys.clone(),
        };
        let weights = one_axis.weights(Some(AxisInput { value, cycle: None }), self.interpolation);
        let old_count = axis.keys.len();
        let strides = self.strides();
        let stride = strides[axis_index];
        let outer = self.forms.len() / (stride * old_count);

        let mut forms = Vec::with_capacity(self.forms.len() / old_count * (old_count + 1));
        for o in 0..outer {
            for k in 0..=old_count {
                for inner in 0..stride {
                    if k == position {
                        let mut form = self.forms[0].zeroed();
                        for &(key, w) in weights.as_slice() {
                            let index = inner + key * stride + o * stride * old_count;
                            form.add_scaled(&self.forms[index], w);
                        }
                        forms.push(form);
                    } else {
                        let source_k = if k < position { k } else { k - 1 };
                        let index = inner + source_k * stride + o * stride * old_count;
                        forms.push(self.forms[index].clone());
                    }
                }
            }
        }
        self.forms = forms;
        self.axes[axis_index].keys.insert(position, value);
        Ok(position)
    }

    /// Remove one key and its keyforms. Removing the last key of an axis
    /// removes the axis.
    pub fn remove_key(&mut self, axis_index: usize, key_index: usize) -> Result<()> {
        let axis = self
            .axes
            .get(axis_index)
            .ok_or_else(|| AetherError::rig("no such parameter axis"))?;
        if key_index >= axis.keys.len() {
            return Err(AetherError::rig("no such key"));
        }
        if axis.keys.len() == 1 {
            return self.remove_axis(axis_index, 0);
        }
        let count = axis.keys.len();
        let stride = self.strides()[axis_index];
        let forms = std::mem::take(&mut self.forms);
        self.forms = forms
            .into_iter()
            .enumerate()
            .filter(|(i, _)| (i / stride) % count != key_index)
            .map(|(_, f)| f)
            .collect();
        self.axes[axis_index].keys.remove(key_index);
        Ok(())
    }

    /// Remove an axis, keeping the slice of forms at `keep_key`.
    pub fn remove_axis(&mut self, axis_index: usize, keep_key: usize) -> Result<()> {
        let axis = self
            .axes
            .get(axis_index)
            .ok_or_else(|| AetherError::rig("no such parameter axis"))?;
        let count = axis.keys.len();
        let keep_key = keep_key.min(count - 1);
        let stride = self.strides()[axis_index];
        let forms = std::mem::take(&mut self.forms);
        self.forms = forms
            .into_iter()
            .enumerate()
            .filter(|(i, _)| (i / stride) % count == keep_key)
            .map(|(_, f)| f)
            .collect();
        self.axes.remove(axis_index);
        Ok(())
    }

    /// Stop depending on `param`, keeping the forms at the key nearest to
    /// `value` (normally the parameter's default).
    pub fn detach_param(&mut self, param: ParameterId, value: f32) {
        if let Some(axis_index) = self.axis_for(param) {
            let keep = self.axes[axis_index].nearest_key(value);
            // Cannot fail: the axis exists.
            let _ = self.remove_axis(axis_index, keep);
        }
    }

    /// Move a key to a new value, keeping keys ordered.
    pub fn move_key(&mut self, axis_index: usize, key_index: usize, value: f32) -> Result<()> {
        let axis = self
            .axes
            .get_mut(axis_index)
            .ok_or_else(|| AetherError::rig("no such parameter axis"))?;
        if key_index >= axis.keys.len() || !value.is_finite() {
            return Err(AetherError::rig("no such key"));
        }
        let lower = if key_index > 0 {
            axis.keys[key_index - 1] + KEY_EPSILON * 10.0
        } else {
            f32::NEG_INFINITY
        };
        let upper = if key_index + 1 < axis.keys.len() {
            axis.keys[key_index + 1] - KEY_EPSILON * 10.0
        } else {
            f32::INFINITY
        };
        axis.keys[key_index] = value.clamp(lower, upper);
        Ok(())
    }

    /// Apply `f` to every form (used when a mesh gains or loses vertices).
    pub fn for_each_form(&mut self, mut f: impl FnMut(&mut T)) {
        for form in &mut self.forms {
            f(form);
        }
    }
}

/// An additive correction driven by one parameter.
///
/// The base grid of an object handles the main poses; blend shapes layer
/// independent adjustments on top — a smile that should combine with any head
/// angle, say — without multiplying the number of keyforms. Each form is a
/// *delta*, and the form at the neutral key is normally all zeros.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendShape<T> {
    /// Display name.
    pub name: String,
    /// Deltas keyed along one or more parameters.
    pub grid: KeyformGrid<T>,
    /// Scales the whole shape; 0 disables it without losing it.
    #[serde(default = "one")]
    pub weight: f32,
}

fn one() -> f32 {
    1.0
}

impl<T: Blend> BlendShape<T> {
    /// A blend shape on `param`, zero at every key.
    pub fn new(name: impl Into<String>, axis: KeyAxis, zero: T) -> Self {
        let forms = vec![zero; axis.keys.len()];
        Self {
            name: name.into(),
            grid: KeyformGrid {
                axes: vec![axis],
                forms,
                interpolation: KeyInterpolation::Linear,
            },
            weight: 1.0,
        }
    }

    /// Add this shape's contribution to `target`.
    pub fn accumulate(&self, target: &mut T, source: &dyn ParamSource) {
        if self.weight == 0.0 {
            return;
        }
        for (index, w) in self.grid.weights(source) {
            target.add_scaled(&self.grid.forms[index], w * self.weight);
        }
    }
}

/// Type-erased structural operations on a keyform grid, so editing code can
/// add keys or parameters to a mesh, a warp or a bone alike.
pub trait GridOps {
    /// The parameter axes.
    fn axes(&self) -> &[KeyAxis];
    /// Number of stored forms.
    fn form_count(&self) -> usize;
    /// Interpolation mode.
    fn interpolation(&self) -> KeyInterpolation;
    /// Change the interpolation mode.
    fn set_interpolation(&mut self, mode: KeyInterpolation);
    /// See [`KeyformGrid::add_axis`].
    fn add_axis(&mut self, axis: KeyAxis) -> Result<()>;
    /// See [`KeyformGrid::insert_key`].
    fn insert_key(&mut self, axis: usize, value: f32) -> Result<usize>;
    /// See [`KeyformGrid::remove_key`].
    fn remove_key(&mut self, axis: usize, key: usize) -> Result<()>;
    /// See [`KeyformGrid::detach_param`].
    fn detach_param(&mut self, param: ParameterId, value: f32);
    /// See [`KeyformGrid::move_key`].
    fn move_key(&mut self, axis: usize, key: usize, value: f32) -> Result<()>;
    /// Overwrite form `to` with a copy of form `from`.
    fn copy_form(&mut self, from: usize, to: usize) -> Result<()>;
    /// See [`KeyformGrid::form_at_keys`].
    fn form_at_keys(&self, source: &dyn ParamSource) -> Option<usize>;
    /// See [`KeyformGrid::axes_off_key`].
    fn axes_off_key(&self, source: &dyn ParamSource) -> Vec<ParameterId>;
}

impl<T: Blend> GridOps for KeyformGrid<T> {
    fn axes(&self) -> &[KeyAxis] {
        &self.axes
    }
    fn form_count(&self) -> usize {
        self.forms.len()
    }
    fn interpolation(&self) -> KeyInterpolation {
        self.interpolation
    }
    fn set_interpolation(&mut self, mode: KeyInterpolation) {
        self.interpolation = mode;
    }
    fn add_axis(&mut self, axis: KeyAxis) -> Result<()> {
        KeyformGrid::add_axis(self, axis)
    }
    fn insert_key(&mut self, axis: usize, value: f32) -> Result<usize> {
        KeyformGrid::insert_key(self, axis, value)
    }
    fn remove_key(&mut self, axis: usize, key: usize) -> Result<()> {
        KeyformGrid::remove_key(self, axis, key)
    }
    fn detach_param(&mut self, param: ParameterId, value: f32) {
        KeyformGrid::detach_param(self, param, value)
    }
    fn move_key(&mut self, axis: usize, key: usize, value: f32) -> Result<()> {
        KeyformGrid::move_key(self, axis, key, value)
    }
    fn copy_form(&mut self, from: usize, to: usize) -> Result<()> {
        let form = self
            .forms
            .get(from)
            .cloned()
            .ok_or_else(|| AetherError::rig("no such keyform"))?;
        let slot = self
            .forms
            .get_mut(to)
            .ok_or_else(|| AetherError::rig("no such keyform"))?;
        *slot = form;
        Ok(())
    }
    fn form_at_keys(&self, source: &dyn ParamSource) -> Option<usize> {
        KeyformGrid::form_at_keys(self, source)
    }
    fn axes_off_key(&self, source: &dyn ParamSource) -> Vec<ParameterId> {
        KeyformGrid::axes_off_key(self, source)
    }
}

impl Blend for f32 {
    fn zeroed(&self) -> Self {
        0.0
    }
    fn add_scaled(&mut self, other: &Self, weight: f32) {
        *self += other * weight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const A: ParameterId = ParameterId(1);
    const B: ParameterId = ParameterId(2);

    fn values(pairs: &[(ParameterId, f32)]) -> impl ParamSource {
        let map: BTreeMap<ParameterId, f32> = pairs.iter().copied().collect();
        move |id: ParameterId| map.get(&id).map(|&value| AxisInput { value, cycle: None })
    }

    fn grid_1d(keys: &[f32], forms: &[f32]) -> KeyformGrid<f32> {
        KeyformGrid {
            axes: vec![KeyAxis::new(A, keys.iter().copied()).expect("axis")],
            forms: forms.to_vec(),
            interpolation: KeyInterpolation::Linear,
        }
    }

    #[test]
    fn linear_interpolation_hits_keys_and_blends_between() {
        let grid = grid_1d(&[-30.0, 0.0, 30.0], &[-1.0, 0.0, 2.0]);
        assert_eq!(grid.evaluate(&values(&[(A, -30.0)])), -1.0);
        assert_eq!(grid.evaluate(&values(&[(A, 15.0)])), 1.0);
        assert_eq!(
            grid.evaluate(&values(&[(A, 99.0)])),
            2.0,
            "values clamp to the end keys"
        );
    }

    #[test]
    fn two_axes_interpolate_bilinearly() {
        // Corners: (a0,b0)=0, (a1,b0)=1, (a0,b1)=2, (a1,b1)=3.
        let grid = KeyformGrid {
            axes: vec![
                KeyAxis::new(A, [0.0, 1.0]).expect("axis"),
                KeyAxis::new(B, [0.0, 1.0]).expect("axis"),
            ],
            forms: vec![0.0, 1.0, 2.0, 3.0],
            interpolation: KeyInterpolation::Linear,
        };
        grid.validate().expect("valid");
        let v = grid.evaluate(&values(&[(A, 0.5), (B, 0.5)]));
        assert!((v - 1.5).abs() < 1e-6, "got {v}");
        assert_eq!(grid.form_at_keys(&values(&[(A, 1.0), (B, 1.0)])), Some(3));
        assert_eq!(grid.form_at_keys(&values(&[(A, 0.5), (B, 1.0)])), None);
        assert_eq!(grid.axes_off_key(&values(&[(A, 0.5), (B, 1.0)])), vec![A]);
    }

    #[test]
    fn smooth_interpolation_passes_through_keys_without_kinks() {
        let mut grid = grid_1d(&[0.0, 1.0, 2.0], &[0.0, 1.0, 0.0]);
        grid.interpolation = KeyInterpolation::Smooth;
        for (v, expected) in [(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)] {
            let got = grid.evaluate(&values(&[(A, v)]));
            assert!((got - expected).abs() < 1e-6, "at {v}: {got}");
        }
        // The slope just left and right of the middle key must match: a smooth
        // peak, not the corner linear interpolation would give.
        let h = 1e-3;
        let left = (grid.evaluate(&values(&[(A, 1.0)])) - grid.evaluate(&values(&[(A, 1.0 - h)]))) / h;
        let right = (grid.evaluate(&values(&[(A, 1.0 + h)])) - grid.evaluate(&values(&[(A, 1.0)]))) / h;
        assert!((left - right).abs() < 0.01, "kink at the key: {left} vs {right}");
    }

    #[test]
    fn cyclic_axes_interpolate_across_the_seam() {
        let grid = grid_1d(&[0.0, 90.0, 180.0, 270.0], &[0.0, 1.0, 2.0, 3.0]);
        let source = |id: ParameterId| {
            (id == A).then_some(AxisInput {
                value: 315.0,
                cycle: Some((0.0, 360.0)),
            })
        };
        // Halfway from the key at 270 (3.0) back round to the key at 0 (0.0).
        let v = grid.evaluate(&source);
        assert!((v - 1.5).abs() < 1e-5, "got {v}");
    }

    #[test]
    fn inserting_a_key_does_not_change_the_pose() {
        let mut grid = KeyformGrid {
            axes: vec![
                KeyAxis::new(A, [0.0, 10.0]).expect("axis"),
                KeyAxis::new(B, [0.0, 1.0]).expect("axis"),
            ],
            forms: vec![0.0, 10.0, 5.0, 25.0],
            interpolation: KeyInterpolation::Linear,
        };
        let probes = [(2.5, 0.25), (5.0, 0.5), (7.0, 1.0), (10.0, 0.0)];
        let before: Vec<f32> = probes
            .iter()
            .map(|&(a, b)| grid.evaluate(&values(&[(A, a), (B, b)])))
            .collect();
        let at = grid.insert_key(0, 5.0).expect("insert");
        assert_eq!(at, 1);
        grid.validate().expect("still valid");
        assert_eq!(grid.axes[0].keys, vec![0.0, 5.0, 10.0]);
        for (probe, expected) in probes.iter().zip(before) {
            let got = grid.evaluate(&values(&[(A, probe.0), (B, probe.1)]));
            assert!((got - expected).abs() < 1e-4, "{probe:?}: {got} vs {expected}");
        }
    }

    #[test]
    fn adding_an_axis_copies_forms_and_removing_it_restores_them() {
        let mut grid = grid_1d(&[0.0, 1.0], &[3.0, 4.0]);
        grid.add_axis(KeyAxis::new(B, [-1.0, 0.0, 1.0]).expect("axis"))
            .expect("add");
        grid.validate().expect("valid");
        assert_eq!(grid.forms.len(), 6);
        assert_eq!(grid.evaluate(&values(&[(A, 1.0), (B, -1.0)])), 4.0);
        assert!(grid.add_axis(KeyAxis::new(B, [0.0]).expect("axis")).is_err());
        // Edit the (A=1, B=1) corner, then drop B keeping its middle key.
        let corner = grid.index_of(&[1, 2]);
        grid.forms[corner] = 99.0;
        grid.detach_param(B, 0.0);
        grid.validate().expect("valid");
        assert_eq!(grid.forms, vec![3.0, 4.0]);
    }

    #[test]
    fn removing_keys_shrinks_the_grid() {
        let mut grid = grid_1d(&[0.0, 1.0, 2.0], &[0.0, 1.0, 2.0]);
        grid.remove_key(0, 1).expect("remove");
        assert_eq!(grid.forms, vec![0.0, 2.0]);
        grid.remove_key(0, 0).expect("remove");
        grid.remove_key(0, 0).expect("remove last");
        assert!(grid.axes.is_empty(), "removing the last key removes the axis");
        assert_eq!(grid.forms.len(), 1);
    }

    #[test]
    fn missing_parameters_pin_to_the_first_key() {
        let grid = grid_1d(&[0.0, 1.0], &[5.0, 6.0]);
        assert_eq!(grid.evaluate(&values(&[])), 5.0);
    }

    #[test]
    fn corrupt_grids_are_reported() {
        let mut grid = grid_1d(&[0.0, 1.0], &[5.0, 6.0]);
        grid.forms.pop();
        assert!(grid.validate().is_err());
        assert!(KeyAxis::new(A, []).is_err());
    }

    #[test]
    fn blend_shapes_add_on_top_of_the_base() {
        let mut shape = BlendShape::new("Smile", KeyAxis::new(B, [0.0, 1.0]).expect("axis"), 0.0f32);
        shape.grid.forms[1] = 2.0;
        let mut base = 10.0f32;
        shape.accumulate(&mut base, &values(&[(B, 0.5)]));
        assert_eq!(base, 11.0);
        shape.weight = 0.0;
        shape.accumulate(&mut base, &values(&[(B, 1.0)]));
        assert_eq!(base, 11.0, "a zero-weight shape contributes nothing");
    }

    #[test]
    fn moving_a_key_keeps_the_order() {
        let mut grid = grid_1d(&[0.0, 1.0, 2.0], &[0.0, 1.0, 2.0]);
        grid.move_key(0, 1, 5.0).expect("move");
        assert!(grid.axes[0].keys[1] < 2.0);
        grid.validate().expect("keys stay ordered");
    }
}
