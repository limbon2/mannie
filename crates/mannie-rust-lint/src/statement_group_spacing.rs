//! Separate steps in a block when the work changes from setup, action or control flow.

use proc_macro2::LineColumn;
use syn::spanned::Spanned as _;
use syn::visit::{self, Visit};
use syn::{BinOp, Block, Expr, Local, Stmt};

#[cfg(test)]
mod tests;

pub(crate) fn check(source: &str) -> syn::Result<Vec<LineColumn>> {
    let file = syn::parse_file(source)?;
    let mut checker = Checker {
        lines: source.lines().collect(),
        findings: Vec::new(),
    };
    checker.visit_file(&file);
    checker.findings.sort_unstable();
    Ok(checker.findings)
}

#[derive(Clone, Copy)]
enum Group {
    Action,
    AwaitedOperation,
    Binding,
    Branch,
    BranchBinding,
    FallibleBinding,
    Loop,
    Other,
    Tail,
}

fn group(statement: &Stmt, last: bool) -> Group {
    match statement {
        Stmt::Local(local) => binding_group(local),
        Stmt::Expr(expression, terminator) => {
            let expression = unparenthesized(expression);
            let group = expression_group(expression);
            let tail_value =
                last && terminator.is_none() && matches!(group, Group::Action | Group::Other);
            let exits = matches!(
                expression,
                Expr::Break(_) | Expr::Continue(_) | Expr::Return(_)
            );

            if tail_value && !exits {
                Group::Tail
            } else {
                group
            }
        }
        _ => Group::Other,
    }
}

fn binding_group(local: &Local) -> Group {
    let Some(init) = &local.init else {
        return Group::Binding;
    };
    let expression = unparenthesized(&init.expr);

    if matches!(expression_group(expression), Group::Branch) {
        return Group::BranchBinding;
    }

    let span = expression.span();
    let multiline = span.start().line != span.end().line;
    let fallible = matches!(expression, Expr::Try(_));

    if multiline && fallible {
        return Group::FallibleBinding;
    }

    Group::Binding
}

fn unparenthesized(expression: &Expr) -> &Expr {
    match expression {
        Expr::Group(group) => unparenthesized(&group.expr),
        Expr::Paren(parentheses) => unparenthesized(&parentheses.expr),
        _ => expression,
    }
}

fn expression_group(expression: &Expr) -> Group {
    match unparenthesized(expression) {
        Expr::Assign(_) | Expr::Call(_) | Expr::MethodCall(_) => Group::Action,
        Expr::Await(_) => Group::AwaitedOperation,
        Expr::Binary(expression) if is_assignment(&expression.op) => Group::Action,
        Expr::ForLoop(_) | Expr::Loop(_) | Expr::While(_) => Group::Loop,
        Expr::If(_) | Expr::Match(_) => Group::Branch,
        Expr::Try(expression) => expression_group(&expression.expr),
        _ => Group::Other,
    }
}

fn is_assignment(operator: &BinOp) -> bool {
    matches!(
        operator,
        BinOp::AddAssign(_)
            | BinOp::BitAndAssign(_)
            | BinOp::BitOrAssign(_)
            | BinOp::BitXorAssign(_)
            | BinOp::DivAssign(_)
            | BinOp::MulAssign(_)
            | BinOp::RemAssign(_)
            | BinOp::ShlAssign(_)
            | BinOp::ShrAssign(_)
            | BinOp::SubAssign(_)
    )
}

fn is_action(group: Group) -> bool {
    matches!(group, Group::Action | Group::AwaitedOperation)
}

fn changes_group(previous: Group, current: Group, consecutive_actions: bool) -> bool {
    use Group::{
        Action, AwaitedOperation, Binding, Branch, BranchBinding, FallibleBinding, Loop, Tail,
    };

    match (previous, current) {
        // A single call and its following check can form one step.
        (Action, Branch) => consecutive_actions,
        (AwaitedOperation, Binding | Branch | BranchBinding | FallibleBinding | Loop)
        | (Binding | Branch | BranchBinding | FallibleBinding | Loop, AwaitedOperation)
        | (Binding | BranchBinding | FallibleBinding, Branch | Loop)
        | (Action | Branch | FallibleBinding, Binding | BranchBinding | FallibleBinding)
        | (Branch | BranchBinding | Loop, Action)
        | (Branch, Loop | Tail)
        | (Loop, Branch) => true,
        _ => false,
    }
}

struct Checker<'source> {
    lines: Vec<&'source str>,
    findings: Vec<LineColumn>,
}

impl Checker<'_> {
    fn has_blank_line(&self, first_after: usize, current_start: LineColumn) -> bool {
        let between = self
            .lines
            .get(first_after..current_start.line.saturating_sub(1));
        between.is_some_and(|lines| lines.iter().any(|line| line.trim().is_empty()))
    }
}

impl<'ast> Visit<'ast> for Checker<'_> {
    fn visit_block(&mut self, block: &'ast Block) {
        let mut consecutive_actions = false;

        for (index, pair) in block.stmts.windows(2).enumerate() {
            let [previous, current] = pair else {
                continue;
            };
            let last = index + 2 == block.stmts.len();
            let previous_group = group(previous, false);
            let current_group = group(current, last);
            let current_start = current.span().start();
            let separated = self.has_blank_line(previous.span().end().line, current_start);
            let needs_blank = changes_group(previous_group, current_group, consecutive_actions);

            if needs_blank && !separated {
                self.findings.push(current_start);
            }

            consecutive_actions =
                !separated && is_action(previous_group) && is_action(current_group);
        }

        visit::visit_block(self, block);
    }
}
