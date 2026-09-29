//! Sampling an object's state on a grid of parameter keys.
//!
//! Cubism interpolates keyforms multilinearly between keys. Aether's rigs
//! can do more — Catmull-Rom interpolation across keys, bones that turn on
//! arcs, drivers that mix parameters, blend shapes — so every exported
//! object is *sampled*: its state is evaluated at each combination of keys,
//! and a key is added halfway between two neighbours wherever interpolating
//! between them would stray from the real state by more than the tolerance.
//! Objects whose state is already multilinear gain no keys.

use aether_core::ParameterId;
use std::collections::HashMap;

/// One grid axis: a parameter and its keys (increasing).
#[derive(Clone, Debug, PartialEq)]
pub struct SampleAxis {
    pub param: ParameterId,
    pub keys: Vec<f32>,
}

/// Sampled states, one per grid point; the first axis varies fastest.
#[derive(Clone, Debug, PartialEq)]
pub struct Sampled {
    pub axes: Vec<SampleAxis>,
    pub forms: Vec<Vec<f32>>,
    /// Refinement stopped at the size limit with error left.
    pub capped: bool,
}

/// How hard to try.
#[derive(Clone, Copy, Debug)]
pub struct SampleLimits {
    /// Largest acceptable error, in the units `weights` convert to.
    pub tolerance: f32,
    /// Most grid points for one object.
    pub max_forms: usize,
    /// Most halvings of one key interval.
    pub max_depth: u32,
    /// Most combinations of the other axes checked per interval.
    pub max_probes: usize,
}

impl Default for SampleLimits {
    fn default() -> Self {
        Self {
            tolerance: 0.5,
            max_forms: 2048,
            max_depth: 6,
            max_probes: 24,
        }
    }
}

fn key(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

struct Cache<'a> {
    eval: &'a mut dyn FnMut(&[f32]) -> Vec<f32>,
    memo: HashMap<Vec<u32>, Vec<f32>>,
}

impl Cache<'_> {
    fn at(&mut self, values: &[f32]) -> Vec<f32> {
        let k = key(values);
        if let Some(v) = self.memo.get(&k) {
            return v.clone();
        }
        let v = (self.eval)(values);
        self.memo.insert(k, v.clone());
        v
    }
}

/// Every combination of per-axis choices, first axis fastest.
fn combinations(counts: &[usize]) -> impl Iterator<Item = Vec<usize>> + '_ {
    let total: usize = counts.iter().product();
    (0..total).map(move |mut n| {
        counts
            .iter()
            .map(|&c| {
                let k = n % c;
                n /= c;
                k
            })
            .collect()
    })
}

/// Sample `eval` (axis values in `axes` order → state) on a grid over
/// `axes`, adding keys where needed. `weights[i]` converts a difference in
/// state component `i` into tolerance units (components beyond `weights`
/// use the last weight).
pub fn sample(
    axes: Vec<SampleAxis>,
    weights: &[f32],
    limits: &SampleLimits,
    eval: &mut dyn FnMut(&[f32]) -> Vec<f32>,
) -> Sampled {
    let mut axes = axes;
    let mut cache = Cache {
        eval,
        memo: HashMap::new(),
    };
    let weight = |i: usize| -> f32 { weights.get(i).or(weights.last()).copied().unwrap_or(1.0) };
    let error = |a: &[f32], b: &[f32], c: &[f32]| -> f32 {
        let mut worst = 0.0f32;
        for i in 0..a.len().min(b.len()).min(c.len()) {
            let e = (c[i] - (a[i] + b[i]) * 0.5).abs() * weight(i);
            if e.is_nan() || e > worst {
                worst = if e.is_nan() { f32::INFINITY } else { e };
            }
        }
        worst
    };
    let forms = |axes: &[SampleAxis]| axes.iter().map(|a| a.keys.len()).product::<usize>();

    let mut capped = false;
    for _ in 0..limits.max_depth {
        let mut changed = false;
        for a in 0..axes.len() {
            let counts: Vec<usize> = axes
                .iter()
                .enumerate()
                .map(|(i, x)| if i == a { 1 } else { x.keys.len() })
                .collect();
            let total: usize = counts.iter().product();
            // A spread of the other axes' key combinations.
            let stride = total.div_ceil(limits.max_probes.max(1)).max(1);
            let probes: Vec<Vec<usize>> = combinations(&counts).step_by(stride).collect();
            let keys = axes[a].keys.clone();
            let span = (keys[keys.len() - 1] - keys[0]).abs().max(1e-6);
            let mut added = Vec::new();
            for w in keys.windows(2) {
                let (k0, k1) = (w[0], w[1]);
                if k1 - k0 < span / 512.0 {
                    continue;
                }
                let mid = (k0 + k1) * 0.5;
                let mut worst = 0.0f32;
                for probe in &probes {
                    let mut at = |k: f32| {
                        let values: Vec<f32> = axes
                            .iter()
                            .enumerate()
                            .map(|(i, x)| if i == a { k } else { x.keys[probe[i]] })
                            .collect();
                        cache.at(&values)
                    };
                    let (f0, f1, fm) = (at(k0), at(k1), at(mid));
                    worst = worst.max(error(&f0, &f1, &fm));
                    if worst > limits.tolerance {
                        break;
                    }
                }
                if worst > limits.tolerance {
                    added.push(mid);
                }
            }
            if added.is_empty() {
                continue;
            }
            let grown = forms(&axes) / axes[a].keys.len() * (axes[a].keys.len() + added.len());
            if grown > limits.max_forms {
                capped = true;
                continue;
            }
            axes[a].keys.extend(added);
            axes[a].keys.sort_by(f32::total_cmp);
            changed = true;
        }
        if !changed && axes.len() >= 2 {
            // Interactions: the centre of each cell of every pair of axes
            // against the average of its corners.
            let mut split: Vec<Vec<f32>> = vec![Vec::new(); axes.len()];
            for a in 0..axes.len() {
                for b in a + 1..axes.len() {
                    let counts: Vec<usize> = axes
                        .iter()
                        .enumerate()
                        .map(|(i, x)| if i == a || i == b { 1 } else { x.keys.len() })
                        .collect();
                    let total: usize = counts.iter().product();
                    let stride = total.div_ceil(limits.max_probes.max(1)).max(1);
                    let probes: Vec<Vec<usize>> = combinations(&counts).step_by(stride).collect();
                    let (ka, kb) = (axes[a].keys.clone(), axes[b].keys.clone());
                    for wa in ka.windows(2) {
                        for wb in kb.windows(2) {
                            let mut worst = 0.0f32;
                            for probe in &probes {
                                let mut at = |va: f32, vb: f32| {
                                    let values: Vec<f32> = axes
                                        .iter()
                                        .enumerate()
                                        .map(|(i, x)| {
                                            if i == a {
                                                va
                                            } else if i == b {
                                                vb
                                            } else {
                                                x.keys[probe[i]]
                                            }
                                        })
                                        .collect();
                                    cache.at(&values)
                                };
                                let corners = [
                                    at(wa[0], wb[0]),
                                    at(wa[1], wb[0]),
                                    at(wa[0], wb[1]),
                                    at(wa[1], wb[1]),
                                ];
                                let centre = at((wa[0] + wa[1]) * 0.5, (wb[0] + wb[1]) * 0.5);
                                let low: Vec<f32> =
                                    corners[0].iter().zip(&corners[3]).map(|(x, y)| x + y).collect();
                                let high: Vec<f32> =
                                    corners[1].iter().zip(&corners[2]).map(|(x, y)| x + y).collect();
                                // Average of the four corners = mean of the
                                // two diagonal sums, halved again.
                                let doubled: Vec<f32> = centre.iter().map(|c| c * 2.0).collect();
                                worst = worst.max(error(&low, &high, &doubled) * 0.5);
                                if worst > limits.tolerance {
                                    break;
                                }
                            }
                            if worst > limits.tolerance {
                                split[a].push((wa[0] + wa[1]) * 0.5);
                                split[b].push((wb[0] + wb[1]) * 0.5);
                            }
                        }
                    }
                }
            }
            for (a, mut added) in split.into_iter().enumerate() {
                added.sort_by(f32::total_cmp);
                added.dedup();
                let span = (axes[a].keys[axes[a].keys.len() - 1] - axes[a].keys[0])
                    .abs()
                    .max(1e-6);
                added.retain(|m| {
                    axes[a]
                        .keys
                        .windows(2)
                        .any(|w| (w[0] + w[1]) * 0.5 == *m && w[1] - w[0] >= span / 512.0)
                });
                if added.is_empty() {
                    continue;
                }
                let grown = forms(&axes) / axes[a].keys.len() * (axes[a].keys.len() + added.len());
                if grown > limits.max_forms {
                    capped = true;
                    continue;
                }
                axes[a].keys.extend(added);
                axes[a].keys.sort_by(f32::total_cmp);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let counts: Vec<usize> = axes.iter().map(|a| a.keys.len()).collect();
    let forms = combinations(&counts)
        .map(|c| {
            let values: Vec<f32> = c.iter().zip(&axes).map(|(&k, a)| a.keys[k]).collect();
            cache.at(&values)
        })
        .collect();
    Sampled { axes, forms, capped }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(param: u64, keys: &[f32]) -> SampleAxis {
        SampleAxis {
            param: ParameterId(param),
            keys: keys.to_vec(),
        }
    }

    #[test]
    fn multilinear_states_gain_no_keys() {
        let mut calls = 0;
        let s = sample(
            vec![axis(1, &[-1.0, 1.0]), axis(2, &[0.0, 1.0])],
            &[1.0],
            &SampleLimits::default(),
            &mut |v| {
                calls += 1;
                vec![3.0 * v[0] + 2.0 * v[1] + v[0] * v[1], 1.0]
            },
        );
        assert_eq!(s.axes[0].keys, vec![-1.0, 1.0]);
        assert_eq!(s.axes[1].keys, vec![0.0, 1.0]);
        assert_eq!(s.forms.len(), 4);
        // The first axis varies fastest.
        assert_eq!(s.forms[1][0], 3.0);
        assert!(calls <= 12, "{calls} evaluations");
    }

    #[test]
    fn curves_gain_keys_until_within_tolerance() {
        // A point on a 100 px arc, turned by the parameter (±60°).
        let arc = |v: &[f32]| {
            let a = v[0].to_radians();
            vec![100.0 * a.cos(), 100.0 * a.sin()]
        };
        let s = sample(
            vec![axis(1, &[-60.0, 0.0, 60.0])],
            &[1.0],
            &SampleLimits::default(),
            &mut |v| arc(v),
        );
        let keys = &s.axes[0].keys;
        assert!(keys.len() > 3 && keys.len() <= 17, "{keys:?}");
        // Linear interpolation between neighbours stays within 0.5 px.
        for w in keys.windows(2) {
            let mid = arc(&[(w[0] + w[1]) / 2.0]);
            let (a, b) = (arc(&[w[0]]), arc(&[w[1]]));
            let e = ((a[0] + b[0]) / 2.0 - mid[0]).hypot((a[1] + b[1]) / 2.0 - mid[1]);
            assert!(e < 0.75, "{e} between {w:?}");
        }
        assert!(!s.capped);
    }

    #[test]
    fn refinement_stops_at_the_size_limit() {
        let limits = SampleLimits {
            max_forms: 6,
            ..Default::default()
        };
        let s = sample(vec![axis(1, &[0.0, 1.0])], &[1.0], &limits, &mut |v| {
            vec![1000.0 * (v[0] * 10.0).sin()]
        });
        assert!(s.axes[0].keys.len() <= 6);
        assert!(s.capped);
    }
}
