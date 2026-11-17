//! The Rust counterpart to the rule in `scripts/eslint/named-conditions.mjs`.
//!
//! A binding may name an entire predicate to preserve its short-circuit order.
//! Check compound conditions at their use sites and in function and closure
//! results. Macro expansion is outside this source check.

use proc_macro2::LineColumn;
use syn::spanned::Spanned as _;
use syn::visit::{self, Visit};
use syn::{Arm, BinOp, Block, Expr, ImplItemFn, ItemFn, Stmt, TraitItemFn, UnOp};

#[cfg(test)]
mod tests;

pub(crate) fn check(source: &str) -> syn::Result<Vec<LineColumn>> {
    let file = syn::parse_file(source)?;
    let mut checker = Checker {
        findings: Vec::new(),
    };
    checker.visit_file(&file);
    checker.findings.sort_unstable();
    Ok(checker.findings)
}

struct Checker {
    findings: Vec<LineColumn>,
}

impl Checker {
    fn check_condition(&mut self, condition: &Expr) {
        // Short conditions are readable inline. Six or more clauses need names
        // for the parts that contain comparisons or other expressions.
        if clause_count(condition) < 6 {
            return;
        }

        self.check_parts(condition);
    }

    fn check_parts(&mut self, condition: &Expr) {
        if let Some((left, right)) = compound_parts(condition) {
            self.check_part(left);
            self.check_part(right);
        }
    }

    fn check_part(&mut self, part: &Expr) {
        if compound_parts(part).is_some() {
            self.check_parts(part);
        } else if !is_named(part) {
            self.findings.push(part.span().start());
        }
    }

    fn check_block_result(&mut self, block: &Block) {
        if let Some(Stmt::Expr(result, None)) = block.stmts.last() {
            self.check_result(result);
        }
    }

    fn check_result(&mut self, result: &Expr) {
        match unparenthesized(result) {
            Expr::Async(expression) => self.check_block_result(&expression.block),
            Expr::Block(expression) => self.check_block_result(&expression.block),
            Expr::Const(expression) => self.check_block_result(&expression.block),
            Expr::If(expression) => {
                self.check_block_result(&expression.then_branch);
                if let Some((_, alternate)) = &expression.else_branch {
                    self.check_result(alternate);
                }
            }
            Expr::Match(expression) => {
                for arm in &expression.arms {
                    self.check_result(&arm.body);
                }
            }
            Expr::Unsafe(expression) => self.check_block_result(&expression.block),
            _ => self.check_condition(result),
        }
    }
}

fn clause_count(expression: &Expr) -> usize {
    if let Some((left, right)) = compound_parts(expression) {
        clause_count(left) + clause_count(right)
    } else {
        1
    }
}

impl<'ast> Visit<'ast> for Checker {
    fn visit_arm(&mut self, arm: &'ast Arm) {
        if let Some((_, guard)) = &arm.guard {
            self.check_condition(guard);
        }

        visit::visit_arm(self, arm);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        match expression {
            Expr::Closure(closure) => self.check_result(&closure.body),
            Expr::If(conditional) => self.check_condition(&conditional.cond),
            Expr::Return(returned) => {
                if let Some(value) = &returned.expr {
                    self.check_result(value);
                }
            }
            Expr::While(conditional) => self.check_condition(&conditional.cond),
            _ => {}
        }

        visit::visit_expr(self, expression);
    }

    fn visit_impl_item_fn(&mut self, function: &'ast ImplItemFn) {
        self.check_block_result(&function.block);
        visit::visit_impl_item_fn(self, function);
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        self.check_block_result(&function.block);
        visit::visit_item_fn(self, function);
    }

    fn visit_trait_item_fn(&mut self, function: &'ast TraitItemFn) {
        if let Some(block) = &function.default {
            self.check_block_result(block);
        }

        visit::visit_trait_item_fn(self, function);
    }
}

fn unparenthesized(mut expression: &Expr) -> &Expr {
    loop {
        match expression {
            Expr::Group(group) => expression = &group.expr,
            Expr::Paren(parentheses) => expression = &parentheses.expr,
            _ => return expression,
        }
    }
}

fn compound_parts(expression: &Expr) -> Option<(&Expr, &Expr)> {
    match unparenthesized(expression) {
        Expr::Binary(binary) if matches!(binary.op, BinOp::And(_) | BinOp::Or(_)) => {
            Some((&binary.left, &binary.right))
        }
        Expr::Unary(unary) if matches!(unary.op, UnOp::Not(_)) => compound_parts(&unary.expr),
        _ => None,
    }
}

fn is_named(expression: &Expr) -> bool {
    match unparenthesized(expression) {
        Expr::Await(expression) => is_named(&expression.base),
        Expr::Call(call) => is_path(&call.func) && has_plain_arguments(&call.args),
        // A let chain can introduce bindings for later clauses.
        Expr::Let(_) | Expr::Lit(_) => true,
        Expr::Macro(expression) => expression
            .mac
            .path
            .segments
            .last()
            .is_some_and(|segment| ["matches", "cfg"].iter().any(|name| segment.ident == *name)),
        Expr::MethodCall(call) => {
            is_plain_argument(&call.receiver) && has_plain_arguments(&call.args)
        }
        Expr::Try(expression) => is_named(&expression.expr),
        Expr::Unary(unary) if matches!(unary.op, UnOp::Not(_)) => is_named(&unary.expr),
        expression => is_path(expression),
    }
}

fn has_plain_arguments<'a>(arguments: impl IntoIterator<Item = &'a Expr>) -> bool {
    arguments.into_iter().all(is_plain_argument)
}

fn is_plain_argument(expression: &Expr) -> bool {
    match unparenthesized(expression) {
        Expr::Lit(_) => true,
        Expr::Reference(reference) => is_plain_argument(&reference.expr),
        Expr::Unary(unary) if matches!(unary.op, UnOp::Neg(_)) => {
            matches!(unparenthesized(&unary.expr), Expr::Lit(_))
        }
        expression => is_path(expression),
    }
}

fn is_path(expression: &Expr) -> bool {
    match unparenthesized(expression) {
        Expr::Field(field) => is_path(&field.base),
        Expr::Index(index) => {
            let simple_index =
                matches!(unparenthesized(&index.index), Expr::Lit(_) | Expr::Path(_));
            is_path(&index.expr) && simple_index
        }
        Expr::Path(_) => true,
        Expr::Unary(unary) if matches!(unary.op, UnOp::Deref(_)) => is_path(&unary.expr),
        _ => false,
    }
}
