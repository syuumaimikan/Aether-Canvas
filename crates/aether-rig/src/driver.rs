//! Parameter drivers.
//!
//! A driver computes one parameter from an expression over the others (see
//! [`crate::expr`]). Drivers run after motions and procedural behaviours and
//! before physics, so physics sees — and can react to — driven values.
//!
//! Drivers may read each other's outputs: they are evaluated in dependency
//! order, and any driver caught in a cycle is reported and skipped rather than
//! producing an order-dependent result.

use crate::expr::Program;
use crate::param::{ParamValues, Parameter};
use aether_core::{AetherError, ParameterId, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

/// One driven parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Driver {
    /// The parameter written.
    pub target: ParameterId,
    /// The expression; parameter names, `time` and `self` (the target's value
    /// before driving) are available.
    pub expression: String,
    /// Whether the driver runs.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Blend between the undriven value (0) and the expression (1).
    #[serde(default = "one")]
    pub mix: f32,
}

impl Driver {
    /// An enabled driver at full strength.
    pub fn new(target: ParameterId, expression: impl Into<String>) -> Self {
        Self {
            target,
            expression: expression.into(),
            enabled: true,
            mix: 1.0,
        }
    }

    /// Compile against `parameters`, reporting syntax and name errors.
    pub fn compile(&self, parameters: &[Parameter]) -> Result<Program> {
        let self_slot = parameters.len();
        let resolve = |name: &str| -> Option<usize> {
            if name == "self" {
                return Some(self_slot);
            }
            parameters.iter().position(|p| p.name == name)
        };
        Program::compile(&self.expression, &resolve)
    }
}

/// A problem found while running drivers.
#[derive(Clone, Debug, PartialEq)]
pub struct DriverIssue {
    /// Index into the rig's driver list.
    pub driver: usize,
    /// What went wrong.
    pub message: String,
}

/// Compile every driver and check for dependency cycles, without running.
pub fn check(parameters: &[Parameter], drivers: &[Driver]) -> Vec<DriverIssue> {
    let mut values = ParamValues::new();
    apply(parameters, drivers, &mut values, 0.0)
}

/// Run the enabled drivers in dependency order, writing into `values`.
pub fn apply(
    parameters: &[Parameter],
    drivers: &[Driver],
    values: &mut ParamValues,
    time: f32,
) -> Vec<DriverIssue> {
    let mut issues = Vec::new();
    let slot_of: BTreeMap<ParameterId, usize> =
        parameters.iter().enumerate().map(|(i, p)| (p.id, i)).collect();

    // Compile.
    let mut compiled: Vec<(usize, Program, usize)> = Vec::new();
    let mut writers: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, driver) in drivers.iter().enumerate() {
        if !driver.enabled {
            continue;
        }
        let Some(&target_slot) = slot_of.get(&driver.target) else {
            issues.push(DriverIssue {
                driver: i,
                message: "the driven parameter no longer exists".into(),
            });
            continue;
        };
        match driver.compile(parameters) {
            Ok(program) => {
                if writers.get(&target_slot).is_some_and(|w| !w.is_empty()) {
                    issues.push(DriverIssue {
                        driver: i,
                        message: format!(
                            "'{}' already has a driver; only the first one runs",
                            parameters[target_slot].name
                        ),
                    });
                    continue;
                }
                writers.entry(target_slot).or_default().push(compiled.len());
                compiled.push((i, program, target_slot));
            }
            Err(error) => issues.push(DriverIssue {
                driver: i,
                message: error.to_string(),
            }),
        }
    }

    // Order: a driver that reads parameter P runs after the driver writing P.
    let n = compiled.len();
    let mut indegree = vec![0usize; n];
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (reader, (_, program, target)) in compiled.iter().enumerate() {
        for &slot in &program.variables {
            if slot == *target {
                continue; // reading your own target means "self", not a cycle
            }
            for &writer in writers.get(&slot).map(|w| w.as_slice()).unwrap_or(&[]) {
                edges[writer].push(reader);
                indegree[reader] += 1;
            }
        }
    }
    let mut ready: Vec<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(next) = ready.pop() {
        order.push(next);
        for &to in &edges[next] {
            indegree[to] -= 1;
            if indegree[to] == 0 {
                ready.push(to);
            }
        }
    }
    if order.len() < n {
        for (i, (driver, _, _)) in compiled.iter().enumerate() {
            if !order.contains(&i) {
                issues.push(DriverIssue {
                    driver: *driver,
                    message: "drivers depend on each other in a loop".into(),
                });
            }
        }
    }

    // Evaluate.
    for index in order {
        let (driver_index, program, target_slot) = &compiled[index];
        let target = &parameters[*target_slot];
        let before = target.clamp(values.get(&target.id).copied().unwrap_or(target.default));
        let read = |slot: usize| -> f32 {
            if slot == parameters.len() {
                return before;
            }
            parameters
                .get(slot)
                .map(|p| p.clamp(values.get(&p.id).copied().unwrap_or(p.default)))
                .unwrap_or(0.0)
        };
        let driven = target.clamp(program.eval(&read, time));
        let mix = drivers[*driver_index].mix.clamp(0.0, 1.0);
        let value = target.clamp(before + (driven - before) * mix);
        values.insert(target.id, value);
    }
    issues
}

/// Parse an expression just to validate it against a parameter set.
pub fn validate_expression(parameters: &[Parameter], expression: &str) -> Result<()> {
    let probe = Driver::new(ParameterId::NONE, expression);
    probe
        .compile(parameters)
        .map(|_| ())
        .map_err(|e| AetherError::rig(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Vec<Parameter> {
        vec![
            Parameter::new(ParameterId(1), "AngleX", -30.0, 30.0, 0.0),
            Parameter::new(ParameterId(2), "BodyAngleX", -10.0, 10.0, 0.0),
            Parameter::new(ParameterId(3), "Hip", -10.0, 10.0, 0.0),
        ]
    }

    #[test]
    fn a_driver_follows_its_source() {
        let p = params();
        let drivers = vec![Driver::new(ParameterId(2), "AngleX * 0.25")];
        let mut values: ParamValues = [(ParameterId(1), 20.0)].into_iter().collect();
        assert!(apply(&p, &drivers, &mut values, 0.0).is_empty());
        assert_eq!(values[&ParameterId(2)], 5.0);
    }

    #[test]
    fn chains_run_in_dependency_order_regardless_of_list_order() {
        let p = params();
        // Hip reads BodyAngleX, which is itself driven — listed first.
        let drivers = vec![
            Driver::new(ParameterId(3), "BodyAngleX * 2"),
            Driver::new(ParameterId(2), "AngleX / 10"),
        ];
        let mut values: ParamValues = [(ParameterId(1), 30.0)].into_iter().collect();
        assert!(apply(&p, &drivers, &mut values, 0.0).is_empty());
        assert_eq!(values[&ParameterId(2)], 3.0);
        assert_eq!(values[&ParameterId(3)], 6.0);
    }

    #[test]
    fn results_are_clamped_and_mixed() {
        let p = params();
        let mut d = Driver::new(ParameterId(2), "AngleX");
        d.mix = 0.5;
        let mut values: ParamValues = [(ParameterId(1), 30.0), (ParameterId(2), 2.0)]
            .into_iter()
            .collect();
        apply(&p, &[d], &mut values, 0.0);
        // Driven value clamps to 10, then mixes halfway from 2.
        assert_eq!(values[&ParameterId(2)], 6.0);
    }

    #[test]
    fn self_and_time_are_available() {
        let p = params();
        let drivers = vec![Driver::new(ParameterId(2), "self + time")];
        let mut values: ParamValues = [(ParameterId(2), 1.0)].into_iter().collect();
        apply(&p, &drivers, &mut values, 2.0);
        assert_eq!(values[&ParameterId(2)], 3.0);
    }

    #[test]
    fn cycles_and_errors_are_reported_not_run() {
        let p = params();
        let drivers = vec![
            Driver::new(ParameterId(2), "Hip"),
            Driver::new(ParameterId(3), "BodyAngleX"),
            Driver::new(ParameterId(1), "Nope + 1"),
        ];
        let mut values = ParamValues::new();
        let issues = apply(&p, &drivers, &mut values, 0.0);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert!(issues.iter().any(|i| i.message.contains("loop")));
        assert!(issues.iter().any(|i| i.message.contains("unknown name")));
        assert!(values.is_empty(), "nothing in a cycle is written");
    }

    #[test]
    fn a_second_driver_on_one_parameter_is_flagged() {
        let p = params();
        let drivers = vec![Driver::new(ParameterId(2), "1"), Driver::new(ParameterId(2), "2")];
        let mut values = ParamValues::new();
        let issues = apply(&p, &drivers, &mut values, 0.0);
        assert_eq!(issues.len(), 1);
        assert_eq!(values[&ParameterId(2)], 1.0);
    }
}
