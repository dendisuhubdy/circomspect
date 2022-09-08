use log::debug;
use std::fmt::Write;
use std::collections::{HashMap, HashSet};

use program_structure::cfg::Cfg;
use program_structure::error_code::ReportCode;
use program_structure::error_definition::{Report, ReportCollection};
use program_structure::file_definition::{FileID, FileLocation};
use program_structure::ir::declarations::Declaration;
use program_structure::ir::variable_meta::{VariableMeta, VariableUse};
use program_structure::ir::{Expression, SignalType, Statement, VariableType};

use crate::constraint_analysis::run_constraint_analysis;
use crate::taint_analysis::run_taint_analysis;

pub struct UnusedVariableWarning {
    name: String,
    file_id: Option<FileID>,
    file_location: FileLocation,
}

impl UnusedVariableWarning {
    pub fn into_report(self) -> Report {
        let mut report = Report::warning(
            format!(
                "The variable `{}` is assigned a value, but this value is never read.",
                self.name
            ),
            ReportCode::UnusedVariableValue,
        );
        if let Some(file_id) = self.file_id {
            report.add_primary(
                self.file_location,
                file_id,
                "The value assigned here is never read.".to_string(),
            );
        }
        report
    }
}
pub struct UnconstrainedSignalWarning {
    name: String,
    dimensions: Vec<Expression>,
    file_id: Option<FileID>,
    file_location: FileLocation,
}

impl UnconstrainedSignalWarning {
    pub fn into_report(self) -> Report {
        if self.dimensions.is_empty() {
            let mut report = Report::warning(
                format!("The signal `{}` is not constrained by the template.", self.name),
                ReportCode::UnconstrainedSignal,
            );
            if let Some(file_id) = self.file_id {
                report.add_primary(
                    self.file_location,
                    file_id,
                    "This signal does not occur in a constraint.".to_string(),
                );
            }
            report
        } else {
            let mut report = Report::warning(
                format!(
                    "The signals `{}{}` are not constrained by the template.",
                    self.name,
                    dimensions_to_string(&self.dimensions)
                ),
                ReportCode::UnconstrainedSignal,
            );
            if let Some(file_id) = self.file_id {
                report.add_primary(
                    self.file_location,
                    file_id,
                    "These signals do not occur in a constraint.".to_string(),
                );
            }
            report
        }
    }
}

pub struct UnusedSignalWarning {
    name: String,
    dimensions: Vec<Expression>,
    file_id: Option<FileID>,
    file_location: FileLocation,
}

impl UnusedSignalWarning {
    pub fn into_report(self) -> Report {
        if self.dimensions.is_empty() {
            let mut report = Report::warning(
                format!("The signal `{}` is not used by the template.", self.name),
                ReportCode::UnusedVariableValue,
            );
            if let Some(file_id) = self.file_id {
                report.add_primary(
                    self.file_location,
                    file_id,
                    "This signal is unused and could be removed.".to_string(),
                );
            }
            report
        } else {
            let mut report = Report::warning(
                format!(
                    "The signals `{}{}` are not used by the template.",
                    self.name,
                    dimensions_to_string(&self.dimensions)
                ),
                ReportCode::UnusedVariableValue,
            );
            if let Some(file_id) = self.file_id {
                report.add_primary(
                    self.file_location,
                    file_id,
                    "These signals are unused and could be removed.".to_string(),
                );
            }
            report
        }
    }
}

pub struct UnusedParameterWarning {
    function_name: String,
    variable_name: String,
    file_id: Option<FileID>,
    file_location: FileLocation,
}

impl UnusedParameterWarning {
    pub fn into_report(self) -> Report {
        let mut report = Report::warning(
            format!("The parameter `{}` is never read.", self.variable_name),
            ReportCode::UnusedParameterValue,
        );
        if let Some(file_id) = self.file_id {
            report.add_primary(
                self.file_location,
                file_id,
                format!(
                    "The parameter `{}` is never used in `{}`.",
                    self.variable_name, self.function_name
                ),
            );
        }
        report
    }
}

pub struct VariableWithoutSideEffectsWarning {
    name: String,
    file_id: Option<FileID>,
    file_location: FileLocation,
}

impl VariableWithoutSideEffectsWarning {
    pub fn into_report(self) -> Report {
        let mut report = Report::warning(
            format!(
                "The value assigned to `{}` is not used in witness or constraint generation.",
                self.name
            ),
            ReportCode::VariableWithoutSideEffect,
        );
        if let Some(file_id) = self.file_id {
            report.add_primary(
                self.file_location,
                file_id,
                format!("The value assigned to `{}` here does not influence witness or constraint generation.", self.name),
            );
        }
        report
    }
}

pub struct ParamWithoutSideEffectsWarning {
    name: String,
    file_id: Option<FileID>,
    file_location: FileLocation,
}

impl ParamWithoutSideEffectsWarning {
    pub fn into_report(self) -> Report {
        let mut report = Report::warning(
            format!(
                "The parameter `{}` is not used in witness or constraint generation.",
                self.name
            ),
            ReportCode::VariableWithoutSideEffect,
        );
        if let Some(file_id) = self.file_id {
            report.add_primary(
                self.file_location,
                file_id,
                format!("The value of the parameter `{}` does not influence witness or constraint generation.", self.name),
            );
        }
        report
    }
}

/// Local variables and intermediate signals that do not flow into either
///
///   1. an input or output signal,
///   3. a function return value, or
///   2. a constraint restricting and input or output signal
///
/// are side-effect free and do not affect either witness or constraint
/// generation.
pub fn run_side_effect_analysis(cfg: &Cfg) -> ReportCollection {
    debug!("running side-effect analysis pass");

    // 1. Run taint and constraint analysis to be able to track data flow.
    let taint_analysis = run_taint_analysis(cfg);
    let constraint_analysis = run_constraint_analysis(cfg);

    // 2. Compute the set of variables read.
    let mut variables_read = HashSet::new();
    for basic_block in cfg.iter() {
        variables_read.extend(basic_block.variables_read().map(|var| var.name().clone()));
    }

    // 3. Compute the set of sinks as follows:
    //
    //   1. Generate the set of input and output signals `A`.
    //   2. Compute the set `B` of variables tainted by `A`.
    //   3. Compute the set `C` of variables occurring in a
    //      constraint together with an element from `B`.
    //   4. Generate the set `D` of variables occurring in
    //      a dimension expression in a declaration, in a
    //      return value, or in an asserted value.
    //
    // The set of sinks is the union of A, C and D.

    // Compute the set of input and output signals.
    let signal_decls = cfg
        .declarations()
        .iter()
        .filter_map(|(name, declaration)| {
            if matches!(declaration.variable_type(), VariableType::Signal(_)) {
                Some((name, declaration))
            } else {
                None
            }
        })
        .collect::<HashMap<_, _>>();
    let exported_signals = signal_decls
        .iter()
        .filter_map(|(name, declaration)| {
            if matches!(
                declaration.variable_type(),
                VariableType::Signal(SignalType::Input | SignalType::Output)
            ) {
                Some(*name)
            } else {
                None
            }
        })
        .cloned()
        .collect::<HashSet<_>>();
    // println!("exported signals: {:?}", exported_signals.keys().collect::<HashSet<_>>());

    // Compute the set of variables tainted by input and output signals.
    let exported_sinks = exported_signals
        .iter()
        .flat_map(|source| taint_analysis.multi_step_taint(source))
        .collect::<HashSet<_>>();
    // println!("exported sinks: {:?}", exported_sinks);

    // Collect variables constraining input and output sinks.
    let mut sinks = exported_sinks
        .iter()
        .flat_map(|source| {
            let mut result = constraint_analysis.multi_step_constraint(source);
            // If the source is part of a constraint we include it in the result.
            if !result.is_empty() {
                result.insert(source.clone());
            }
            result
        })
        .collect::<HashSet<_>>();

    // Add input and output signals to this set.
    sinks.extend(exported_signals.into_iter());

    // println!("constraint sinks: {:?}", sinks);

    // Add variables occurring in declarations, return values, asserts, and
    // control-flow conditions.
    use Statement::*;
    for basic_block in cfg.iter() {
        for stmt in basic_block.iter() {
            match stmt {
                Declaration { .. } | Return { .. } | Assert { .. } | IfThenElse { .. } => {
                    // If a variable used in a dimension expression is side-effect free,
                    // the declared variable must also be side-effect free.
                    sinks.extend(stmt.variables_read().map(|var| var.name().clone()));
                }
                _ => {}
            }
        }
    }
    // println!("all sinks: {:?}", sinks);
    // println!("variables read: {:?}", variables_read);

    let mut reports = ReportCollection::new();
    let mut reported_vars = HashSet::new();

    // Generate a report for any variable that does not taint a sink.
    for source in taint_analysis.definitions() {
        if !variables_read.contains(source.name()) {
            // If the variable is unread, the corresponding value is unused.
            if cfg.parameters().contains(source.name()) {
                reports.push(build_unused_param(cfg.name(), source))
            } else {
                reports.push(build_unused_variable(source));
            }
            reported_vars.insert(source.name());
        } else if !taint_analysis.taints_any(source.name(), &sinks) {
            // If the variable does not flow into any of the sinks, it is side-effect free.
            if cfg.parameters().contains(source.name()) {
                reports.push(build_param_without_side_effect(source));
            } else {
                reports.push(build_variable_without_side_effect(source));
            }
            reported_vars.insert(source.name());
        }
    }
    // Generate reports for unused or unconstrained signals.
    for (source, declaration) in signal_decls {
        // Don't report on variables twice.
        if reported_vars.contains(source) {
            continue;
        }
        if !variables_read.contains(source) {
            // If the variable is unread, it must be unconstrained.
            reports.push(build_unused_signal(declaration));
        } else if !taint_analysis.taints_any(source, &constraint_analysis.constrained_variables()) {
            // If the signal does not flow to a constraint, it is unconstrained.
            reports.push(build_unconstrained_signal(declaration));
        }
    }
    reports
}

fn build_unused_variable(definition: &VariableUse) -> Report {
    UnusedVariableWarning {
        name: definition.name().to_string(),
        file_id: definition.meta().file_id(),
        file_location: definition.meta().file_location(),
    }
    .into_report()
}

fn build_unused_param(function_name: &str, definition: &VariableUse) -> Report {
    UnusedParameterWarning {
        function_name: function_name.to_string(),
        variable_name: definition.name().to_string(),
        file_id: definition.meta().file_id(),
        file_location: definition.meta().file_location(),
    }
    .into_report()
}

fn build_unused_signal(declaration: &Declaration) -> Report {
    UnusedSignalWarning {
        name: declaration.variable_name().to_string(),
        dimensions: declaration.dimensions().clone(),
        file_id: declaration.file_id(),
        file_location: declaration.file_location(),
    }
    .into_report()
}

fn build_unconstrained_signal(declaration: &Declaration) -> Report {
    UnconstrainedSignalWarning {
        name: declaration.variable_name().to_string(),
        dimensions: declaration.dimensions().clone(),
        file_id: declaration.file_id(),
        file_location: declaration.file_location(),
    }
    .into_report()
}

fn build_variable_without_side_effect(definition: &VariableUse) -> Report {
    VariableWithoutSideEffectsWarning {
        name: definition.name().to_string(),
        file_id: definition.meta().file_id(),
        file_location: definition.meta().file_location(),
    }
    .into_report()
}

fn build_param_without_side_effect(definition: &VariableUse) -> Report {
    ParamWithoutSideEffectsWarning {
        name: definition.name().to_string(),
        file_id: definition.meta().file_id(),
        file_location: definition.meta().file_location(),
    }
    .into_report()
}

fn dimensions_to_string(dimensions: &[Expression]) -> String {
    let mut result = String::new();
    for size in dimensions {
        // We ignore errors here.
        let _ = write!(result, "[{}]", size);
    }
    result
}

#[cfg(test)]
mod tests {
    use parser::parse_definition;
    use program_structure::cfg::IntoCfg;

    use super::*;

    #[test]
    fn test_side_effect_analysis() {
        let src = r#"
            template T(n) {
              signal input in;
              signal output out[n];

              var lin = in * in;
              var lout = 0;  // The value assigned here is side-effect free.
              var nout = 0;

              var e = 1;  // The value assigned here is side-effect free.
              for (var k = 0; k < n; k++) {
                out[k] <-- (in >> k) & 1;
                out[k] * (out[k] - 1) === 0;

                lout += out[k] * e;  // The value assigned here is side-effect free.
                e = e + e;  // The value assigned here is side-effect free.
              }

              lin === nout;  // Should use `lout`, but uses `nout` by mistake.
            }
        "#;
        validate_reports(src, 4);

        let src = r#"
            template PointOnLine(k, m, n) {
                signal input in[2];

                var LOGK = log2(k);
                var LOGK2 = log2(k * k);
                assert(3 * n + LOGK2 < 251);

                component left = BigTemplate(n, k, 2 * n + LOGK + 1);
                component right[m];
                for (var i = 0; i < n; i++) {
                    right[i] = SmallTemplate(k);
                }
                left.a <== right[0].a;
                left.b <== right[0].b;
            }
        "#;
        validate_reports(src, 4);

        let src = r#"
            template Sum(n) {
                signal input in[n];
                signal output out[n];

                var e = 1;
                var lin = 0;
                for (var i = 0; i < n; i++) {
                    lin += in[i] * e;
                    e += e;
                }

                var lout = 0;
                for (var i = 0; i < n; i++) {
                    lout += out[i];
                }

                lin === lout;
            }
        "#;
        validate_reports(src, 0);

        let src = r#"
            template T(n) {
                signal tmp[n];

                tmp[0] <-- 0;
            }
        "#;
        validate_reports(src, 1);
    }

    fn validate_reports(src: &str, expected_len: usize) {
        // Build CFG.
        let mut reports = ReportCollection::new();
        let cfg =
            parse_definition(src).unwrap().into_cfg(&mut reports).unwrap().into_ssa().unwrap();
        assert!(reports.is_empty());

        // Generate report collection.
        let reports = run_side_effect_analysis(&cfg);
        assert_eq!(reports.len(), expected_len);
    }
}
