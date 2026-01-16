//! IR optimization passes.
//!
//! This module provides safe optimization passes for the IR layer.
//! All optimizations preserve program semantics including lazy evaluation.

use crate::ir::*;

/// Trait for IR optimization passes.
///
/// Each pass transforms an IR module while preserving semantics.
pub trait OptimizationPass {
    /// Apply the optimization pass to a module.
    fn optimize_module(&self, module: &IrModule) -> IrModule;

    /// Return the name of this optimization pass.
    fn name(&self) -> &'static str;
}

/// Run a sequence of optimization passes on a module.
pub fn run_passes(module: &IrModule, passes: &[Box<dyn OptimizationPass>]) -> IrModule {
    let mut result = module.clone();
    for pass in passes {
        result = pass.optimize_module(&result);
    }
    result
}

/// Constant folding optimization pass.
///
/// Evaluates constant expressions at compile time.
/// Safe because it only folds pure, ground terms.
pub struct ConstantFolding;

impl ConstantFolding {
    fn fold_expr(&self, expr: &IrExpr) -> IrExpr {
        match expr {
            // Recursively fold subexpressions
            IrExpr::Lambda { params, body } => IrExpr::Lambda {
                params: params.clone(),
                body: Box::new(self.fold_expr(body)),
            },
            IrExpr::Apply { func, args } => {
                let folded_func = self.fold_expr(func);
                let folded_args: Vec<_> = args.iter().map(|a| self.fold_expr(a)).collect();
                
                // Beta reduction for simple cases
                if let IrExpr::Lambda { params, body } = &folded_func {
                    if params.len() == folded_args.len() && folded_args.iter().all(Self::is_value) {
                        // Safe to inline: all arguments are values
                        return self.substitute_params(body, params, &folded_args);
                    }
                }
                
                IrExpr::Apply {
                    func: Box::new(folded_func),
                    args: folded_args,
                }
            }
            IrExpr::If { cond, then_branch, else_branch } => {
                let folded_cond = self.fold_expr(cond);
                
                // Simplify if the condition is a constant
                if let IrExpr::Bool(b) = folded_cond {
                    if b {
                        return self.fold_expr(then_branch);
                    } else {
                        return self.fold_expr(else_branch);
                    }
                }
                
                IrExpr::If {
                    cond: Box::new(folded_cond),
                    then_branch: Box::new(self.fold_expr(then_branch)),
                    else_branch: Box::new(self.fold_expr(else_branch)),
                }
            }
            IrExpr::Let { bindings, body } => {
                let folded_bindings: Vec<_> = bindings
                    .iter()
                    .map(|(name, expr)| (name.clone(), self.fold_expr(expr)))
                    .collect();
                
                IrExpr::Let {
                    bindings: folded_bindings,
                    body: Box::new(self.fold_expr(body)),
                }
            }
            IrExpr::Case { expr, arms } => {
                let folded_expr = self.fold_expr(expr);
                let folded_arms: Vec<_> = arms
                    .iter()
                    .map(|arm| IrCaseArm {
                        pat: arm.pat.clone(),
                        guard: arm.guard.as_ref().map(|g| self.fold_expr(g)),
                        body: self.fold_expr(&arm.body),
                    })
                    .collect();
                
                IrExpr::Case {
                    expr: Box::new(folded_expr),
                    arms: folded_arms,
                }
            }
            IrExpr::IoBind { action, param, body } => IrExpr::IoBind {
                action: Box::new(self.fold_expr(action)),
                param: param.clone(),
                body: Box::new(self.fold_expr(body)),
            },
            IrExpr::IoThen { first, then_expr } => IrExpr::IoThen {
                first: Box::new(self.fold_expr(first)),
                then_expr: Box::new(self.fold_expr(then_expr)),
            },
            IrExpr::Cons { head, tail } => IrExpr::Cons {
                head: Box::new(self.fold_expr(head)),
                tail: Box::new(self.fold_expr(tail)),
            },
            IrExpr::List(es) => {
                IrExpr::List(es.iter().map(|e| self.fold_expr(e)).collect())
            }
            IrExpr::Tuple(es) => {
                IrExpr::Tuple(es.iter().map(|e| self.fold_expr(e)).collect())
            }
            IrExpr::Record(fields) => IrExpr::Record(
                fields
                    .iter()
                    .map(|(name, expr)| (name.clone(), self.fold_expr(expr)))
                    .collect(),
            ),
            IrExpr::CheckedCast { expr, target } => IrExpr::CheckedCast {
                expr: Box::new(self.fold_expr(expr)),
                target: *target,
            },
            
            // Literals and variables are already fully evaluated
            IrExpr::Unit
            | IrExpr::Integer(_)
            | IrExpr::Float64(_)
            | IrExpr::Bool(_)
            | IrExpr::String(_)
            | IrExpr::Char(_)
            | IrExpr::Var(_) => expr.clone(),
        }
    }

    fn is_value(expr: &IrExpr) -> bool {
        matches!(
            expr,
            IrExpr::Unit
                | IrExpr::Integer(_)
                | IrExpr::Float64(_)
                | IrExpr::Bool(_)
                | IrExpr::String(_)
                | IrExpr::Char(_)
        )
    }

    fn substitute_params(&self, body: &IrExpr, params: &[String], args: &[IrExpr]) -> IrExpr {
        let mut subst_map = std::collections::HashMap::new();
        for (param, arg) in params.iter().zip(args.iter()) {
            subst_map.insert(param.clone(), arg.clone());
        }
        self.substitute(body, &subst_map)
    }

    fn substitute(
        &self,
        expr: &IrExpr,
        subst: &std::collections::HashMap<String, IrExpr>,
    ) -> IrExpr {
        match expr {
            IrExpr::Var(name) => {
                if let Some(replacement) = subst.get(name) {
                    replacement.clone()
                } else {
                    expr.clone()
                }
            }
            IrExpr::Lambda { params, body } => {
                // Remove shadowed variables from substitution
                let mut new_subst = subst.clone();
                for param in params {
                    new_subst.remove(param);
                }
                IrExpr::Lambda {
                    params: params.clone(),
                    body: Box::new(self.substitute(body, &new_subst)),
                }
            }
            IrExpr::Apply { func, args } => IrExpr::Apply {
                func: Box::new(self.substitute(func, subst)),
                args: args.iter().map(|a| self.substitute(a, subst)).collect(),
            },
            IrExpr::If { cond, then_branch, else_branch } => IrExpr::If {
                cond: Box::new(self.substitute(cond, subst)),
                then_branch: Box::new(self.substitute(then_branch, subst)),
                else_branch: Box::new(self.substitute(else_branch, subst)),
            },
            IrExpr::Let { bindings, body } => {
                // Remove bound variables from substitution
                let mut new_subst = subst.clone();
                for (name, _) in bindings {
                    new_subst.remove(name);
                }
                IrExpr::Let {
                    bindings: bindings
                        .iter()
                        .map(|(name, expr)| (name.clone(), self.substitute(expr, &new_subst)))
                        .collect(),
                    body: Box::new(self.substitute(body, &new_subst)),
                }
            }
            IrExpr::Case { expr: case_expr, arms } => IrExpr::Case {
                expr: Box::new(self.substitute(case_expr, subst)),
                arms: arms
                    .iter()
                    .map(|arm| {
                        // Remove pattern-bound variables from substitution
                        let mut new_subst = subst.clone();
                        self.remove_pattern_vars(&arm.pat, &mut new_subst);
                        IrCaseArm {
                            pat: arm.pat.clone(),
                            guard: arm.guard.as_ref().map(|g| self.substitute(g, &new_subst)),
                            body: self.substitute(&arm.body, &new_subst),
                        }
                    })
                    .collect(),
            },
            IrExpr::IoBind { action, param, body } => {
                let mut new_subst = subst.clone();
                new_subst.remove(param);
                IrExpr::IoBind {
                    action: Box::new(self.substitute(action, subst)),
                    param: param.clone(),
                    body: Box::new(self.substitute(body, &new_subst)),
                }
            }
            IrExpr::IoThen { first, then_expr } => IrExpr::IoThen {
                first: Box::new(self.substitute(first, subst)),
                then_expr: Box::new(self.substitute(then_expr, subst)),
            },
            IrExpr::Cons { head, tail } => IrExpr::Cons {
                head: Box::new(self.substitute(head, subst)),
                tail: Box::new(self.substitute(tail, subst)),
            },
            IrExpr::List(es) => {
                IrExpr::List(es.iter().map(|e| self.substitute(e, subst)).collect())
            }
            IrExpr::Tuple(es) => {
                IrExpr::Tuple(es.iter().map(|e| self.substitute(e, subst)).collect())
            }
            IrExpr::Record(fields) => IrExpr::Record(
                fields
                    .iter()
                    .map(|(name, expr)| (name.clone(), self.substitute(expr, subst)))
                    .collect(),
            ),
            IrExpr::CheckedCast { expr: cast_expr, target } => IrExpr::CheckedCast {
                expr: Box::new(self.substitute(cast_expr, subst)),
                target: *target,
            },
            
            // Literals are unchanged
            IrExpr::Unit
            | IrExpr::Integer(_)
            | IrExpr::Float64(_)
            | IrExpr::Bool(_)
            | IrExpr::String(_)
            | IrExpr::Char(_) => expr.clone(),
        }
    }

    fn remove_pattern_vars(
        &self,
        pat: &IrPattern,
        subst: &mut std::collections::HashMap<String, IrExpr>,
    ) {
        match pat {
            IrPattern::Var(name) => {
                subst.remove(name);
            }
            IrPattern::As(name, inner) => {
                subst.remove(name);
                self.remove_pattern_vars(inner, subst);
            }
            IrPattern::Tuple(pats) | IrPattern::List(pats) => {
                for p in pats {
                    self.remove_pattern_vars(p, subst);
                }
            }
            IrPattern::Record(fields) => {
                for (_, p) in fields {
                    self.remove_pattern_vars(p, subst);
                }
            }
            IrPattern::RecordLoose(fields, rest) => {
                for (_, p) in fields {
                    self.remove_pattern_vars(p, subst);
                }
                if let Some(name) = rest {
                    subst.remove(name);
                }
            }
            IrPattern::Cons(a, b) | IrPattern::Or(a, b) => {
                self.remove_pattern_vars(a, subst);
                self.remove_pattern_vars(b, subst);
            }
            IrPattern::Constructor { args, .. } => {
                for p in args {
                    self.remove_pattern_vars(p, subst);
                }
            }
            IrPattern::View(p, _) => {
                self.remove_pattern_vars(p, subst);
            }
            IrPattern::Wildcard | IrPattern::Literal(_) => {}
        }
    }
}

impl OptimizationPass for ConstantFolding {
    fn optimize_module(&self, module: &IrModule) -> IrModule {
        IrModule {
            items: module
                .items
                .iter()
                .map(|item| match item {
                    IrItem::Binding { name, expr } => IrItem::Binding {
                        name: name.clone(),
                        expr: self.fold_expr(expr),
                    },
                })
                .collect(),
        }
    }

    fn name(&self) -> &'static str {
        "constant_folding"
    }
}

/// Dead code elimination pass.
///
/// Removes unused bindings from the module.
/// Safe because it only removes provably unused definitions.
pub struct DeadCodeElimination;

impl DeadCodeElimination {
    fn collect_free_vars(&self, expr: &IrExpr, vars: &mut std::collections::HashSet<String>) {
        match expr {
            IrExpr::Var(name) => {
                vars.insert(name.clone());
            }
            IrExpr::Lambda { params, body } => {
                let mut body_vars = std::collections::HashSet::new();
                self.collect_free_vars(body, &mut body_vars);
                for var in body_vars {
                    if !params.contains(&var) {
                        vars.insert(var);
                    }
                }
            }
            IrExpr::Apply { func, args } => {
                self.collect_free_vars(func, vars);
                for arg in args {
                    self.collect_free_vars(arg, vars);
                }
            }
            IrExpr::If { cond, then_branch, else_branch } => {
                self.collect_free_vars(cond, vars);
                self.collect_free_vars(then_branch, vars);
                self.collect_free_vars(else_branch, vars);
            }
            IrExpr::Let { bindings, body } => {
                let bound: std::collections::HashSet<_> = 
                    bindings.iter().map(|(name, _)| name.clone()).collect();
                
                for (_, expr) in bindings {
                    self.collect_free_vars(expr, vars);
                }
                
                let mut body_vars = std::collections::HashSet::new();
                self.collect_free_vars(body, &mut body_vars);
                
                for var in body_vars {
                    if !bound.contains(&var) {
                        vars.insert(var);
                    }
                }
            }
            IrExpr::Case { expr: case_expr, arms } => {
                self.collect_free_vars(case_expr, vars);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.collect_free_vars(guard, vars);
                    }
                    self.collect_free_vars(&arm.body, vars);
                }
            }
            IrExpr::IoBind { action, param, body } => {
                self.collect_free_vars(action, vars);
                let mut body_vars = std::collections::HashSet::new();
                self.collect_free_vars(body, &mut body_vars);
                for var in body_vars {
                    if var != *param {
                        vars.insert(var);
                    }
                }
            }
            IrExpr::IoThen { first, then_expr } => {
                self.collect_free_vars(first, vars);
                self.collect_free_vars(then_expr, vars);
            }
            IrExpr::Cons { head, tail } => {
                self.collect_free_vars(head, vars);
                self.collect_free_vars(tail, vars);
            }
            IrExpr::List(es) | IrExpr::Tuple(es) => {
                for e in es {
                    self.collect_free_vars(e, vars);
                }
            }
            IrExpr::Record(fields) => {
                for (_, expr) in fields {
                    self.collect_free_vars(expr, vars);
                }
            }
            IrExpr::CheckedCast { expr: cast_expr, .. } => {
                self.collect_free_vars(cast_expr, vars);
            }
            IrExpr::Unit
            | IrExpr::Integer(_)
            | IrExpr::Float64(_)
            | IrExpr::Bool(_)
            | IrExpr::String(_)
            | IrExpr::Char(_) => {}
        }
    }
}

impl OptimizationPass for DeadCodeElimination {
    fn optimize_module(&self, module: &IrModule) -> IrModule {
        // Always keep main
        let mut live = std::collections::HashSet::new();
        live.insert("main".to_string());
        
        // Compute transitive closure of live bindings
        let mut changed = true;
        while changed {
            changed = false;
            for item in &module.items {
                let IrItem::Binding { name, expr } = item;
                if live.contains(name) {
                    let mut vars = std::collections::HashSet::new();
                    self.collect_free_vars(expr, &mut vars);
                    for var in vars {
                        if !live.contains(&var) {
                            live.insert(var);
                            changed = true;
                        }
                    }
                }
            }
        }
        
        // Keep only live bindings
        IrModule {
            items: module
                .items
                .iter()
                .filter(|item| match item {
                    IrItem::Binding { name, .. } => live.contains(name),
                })
                .cloned()
                .collect(),
        }
    }

    fn name(&self) -> &'static str {
        "dead_code_elimination"
    }
}

/// Case simplification pass.
///
/// Simplifies case expressions with single arms and trivial patterns.
/// Safe because it preserves matching semantics.
pub struct CaseSimplification;

impl CaseSimplification {
    fn simplify_expr(&self, expr: &IrExpr) -> IrExpr {
        match expr {
            IrExpr::Case { expr: case_expr, arms } if arms.len() == 1 => {
                let arm = &arms[0];
                
                // If the pattern is a wildcard or simple var without guard, simplify
                if arm.guard.is_none() {
                    match &arm.pat {
                        IrPattern::Wildcard => {
                            return self.simplify_expr(&arm.body);
                        }
                        IrPattern::Var(name) => {
                            // case x of v -> body  ~~>  let v = x in body
                            return self.simplify_expr(&IrExpr::Let {
                                bindings: vec![(name.clone(), case_expr.as_ref().clone())],
                                body: Box::new(arm.body.clone()),
                            });
                        }
                        _ => {}
                    }
                }
                
                // Otherwise, keep the case but simplify recursively
                IrExpr::Case {
                    expr: Box::new(self.simplify_expr(case_expr)),
                    arms: vec![IrCaseArm {
                        pat: arm.pat.clone(),
                        guard: arm.guard.as_ref().map(|g| self.simplify_expr(g)),
                        body: self.simplify_expr(&arm.body),
                    }],
                }
            }
            IrExpr::Case { expr: case_expr, arms } => IrExpr::Case {
                expr: Box::new(self.simplify_expr(case_expr)),
                arms: arms
                    .iter()
                    .map(|arm| IrCaseArm {
                        pat: arm.pat.clone(),
                        guard: arm.guard.as_ref().map(|g| self.simplify_expr(g)),
                        body: self.simplify_expr(&arm.body),
                    })
                    .collect(),
            },
            IrExpr::Lambda { params, body } => IrExpr::Lambda {
                params: params.clone(),
                body: Box::new(self.simplify_expr(body)),
            },
            IrExpr::Apply { func, args } => IrExpr::Apply {
                func: Box::new(self.simplify_expr(func)),
                args: args.iter().map(|a| self.simplify_expr(a)).collect(),
            },
            IrExpr::If { cond, then_branch, else_branch } => IrExpr::If {
                cond: Box::new(self.simplify_expr(cond)),
                then_branch: Box::new(self.simplify_expr(then_branch)),
                else_branch: Box::new(self.simplify_expr(else_branch)),
            },
            IrExpr::Let { bindings, body } => IrExpr::Let {
                bindings: bindings
                    .iter()
                    .map(|(name, expr)| (name.clone(), self.simplify_expr(expr)))
                    .collect(),
                body: Box::new(self.simplify_expr(body)),
            },
            IrExpr::IoBind { action, param, body } => IrExpr::IoBind {
                action: Box::new(self.simplify_expr(action)),
                param: param.clone(),
                body: Box::new(self.simplify_expr(body)),
            },
            IrExpr::IoThen { first, then_expr } => IrExpr::IoThen {
                first: Box::new(self.simplify_expr(first)),
                then_expr: Box::new(self.simplify_expr(then_expr)),
            },
            IrExpr::Cons { head, tail } => IrExpr::Cons {
                head: Box::new(self.simplify_expr(head)),
                tail: Box::new(self.simplify_expr(tail)),
            },
            IrExpr::List(es) => {
                IrExpr::List(es.iter().map(|e| self.simplify_expr(e)).collect())
            }
            IrExpr::Tuple(es) => {
                IrExpr::Tuple(es.iter().map(|e| self.simplify_expr(e)).collect())
            }
            IrExpr::Record(fields) => IrExpr::Record(
                fields
                    .iter()
                    .map(|(name, expr)| (name.clone(), self.simplify_expr(expr)))
                    .collect(),
            ),
            IrExpr::CheckedCast { expr: cast_expr, target } => IrExpr::CheckedCast {
                expr: Box::new(self.simplify_expr(cast_expr)),
                target: *target,
            },
            
            // Literals and variables are unchanged
            IrExpr::Unit
            | IrExpr::Integer(_)
            | IrExpr::Float64(_)
            | IrExpr::Bool(_)
            | IrExpr::String(_)
            | IrExpr::Char(_)
            | IrExpr::Var(_) => expr.clone(),
        }
    }
}

impl OptimizationPass for CaseSimplification {
    fn optimize_module(&self, module: &IrModule) -> IrModule {
        IrModule {
            items: module
                .items
                .iter()
                .map(|item| match item {
                    IrItem::Binding { name, expr } => IrItem::Binding {
                        name: name.clone(),
                        expr: self.simplify_expr(expr),
                    },
                })
                .collect(),
        }
    }

    fn name(&self) -> &'static str {
        "case_simplification"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_folding_if_true() {
        let pass = ConstantFolding;
        let expr = IrExpr::If {
            cond: Box::new(IrExpr::Bool(true)),
            then_branch: Box::new(IrExpr::Integer("42".to_string())),
            else_branch: Box::new(IrExpr::Integer("0".to_string())),
        };
        let result = pass.fold_expr(&expr);
        assert_eq!(result, IrExpr::Integer("42".to_string()));
    }

    #[test]
    fn test_constant_folding_if_false() {
        let pass = ConstantFolding;
        let expr = IrExpr::If {
            cond: Box::new(IrExpr::Bool(false)),
            then_branch: Box::new(IrExpr::Integer("42".to_string())),
            else_branch: Box::new(IrExpr::Integer("0".to_string())),
        };
        let result = pass.fold_expr(&expr);
        assert_eq!(result, IrExpr::Integer("0".to_string()));
    }

    #[test]
    fn test_constant_folding_beta_reduction() {
        let pass = ConstantFolding;
        let expr = IrExpr::Apply {
            func: Box::new(IrExpr::Lambda {
                params: vec!["x".to_string()],
                body: Box::new(IrExpr::Var("x".to_string())),
            }),
            args: vec![IrExpr::Integer("42".to_string())],
        };
        let result = pass.fold_expr(&expr);
        assert_eq!(result, IrExpr::Integer("42".to_string()));
    }

    #[test]
    fn test_dead_code_elimination_keeps_main() {
        let pass = DeadCodeElimination;
        let module = IrModule {
            items: vec![
                IrItem::Binding {
                    name: "main".to_string(),
                    expr: IrExpr::Unit,
                },
                IrItem::Binding {
                    name: "unused".to_string(),
                    expr: IrExpr::Integer("42".to_string()),
                },
            ],
        };
        let result = pass.optimize_module(&module);
        assert_eq!(result.items.len(), 1);
        assert_eq!(
            result.items[0],
            IrItem::Binding {
                name: "main".to_string(),
                expr: IrExpr::Unit,
            }
        );
    }

    #[test]
    fn test_dead_code_elimination_keeps_used() {
        let pass = DeadCodeElimination;
        let module = IrModule {
            items: vec![
                IrItem::Binding {
                    name: "main".to_string(),
                    expr: IrExpr::Var("helper".to_string()),
                },
                IrItem::Binding {
                    name: "helper".to_string(),
                    expr: IrExpr::Integer("42".to_string()),
                },
                IrItem::Binding {
                    name: "unused".to_string(),
                    expr: IrExpr::Integer("0".to_string()),
                },
            ],
        };
        let result = pass.optimize_module(&module);
        assert_eq!(result.items.len(), 2);
    }

    #[test]
    fn test_case_simplification_wildcard() {
        let pass = CaseSimplification;
        let expr = IrExpr::Case {
            expr: Box::new(IrExpr::Integer("42".to_string())),
            arms: vec![IrCaseArm {
                pat: IrPattern::Wildcard,
                guard: None,
                body: IrExpr::Unit,
            }],
        };
        let result = pass.simplify_expr(&expr);
        assert_eq!(result, IrExpr::Unit);
    }

    #[test]
    fn test_case_simplification_var() {
        let pass = CaseSimplification;
        let expr = IrExpr::Case {
            expr: Box::new(IrExpr::Integer("42".to_string())),
            arms: vec![IrCaseArm {
                pat: IrPattern::Var("x".to_string()),
                guard: None,
                body: IrExpr::Var("x".to_string()),
            }],
        };
        let result = pass.simplify_expr(&expr);
        // Should become: let x = 42 in x
        assert!(matches!(result, IrExpr::Let { .. }));
    }
}
