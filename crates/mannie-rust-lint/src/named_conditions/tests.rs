use super::check;

fn flagged_lines(source: &str) -> Vec<usize> {
    check(source)
        .unwrap()
        .into_iter()
        .map(|position| position.line)
        .collect()
}

#[test]
fn flags_the_comparison_and_iterator_chain_in_the_api_guard() {
    let source = r#"fn check(variable: Variable, names: &mut Names) {
    if variable.name.is_empty()
        || variable.name.len() > 128
        || !variable
            .name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
        || !names.insert(&variable.name)
    {}
}"#;

    let findings = check(source).unwrap();
    let positions: Vec<_> = findings
        .into_iter()
        .map(|position| (position.line, position.column + 1))
        .collect();

    assert_eq!(positions, [(3, 12), (4, 12)]);
}

#[test]
fn accepts_named_bindings_paths_calls_patterns_and_whole_predicates() {
    let source = "fn check(state: State, flags: &[bool], index: usize) -> bool {
    let permitted = index < flags.len() && flags[index];
    if !state.enabled || rejects(&state.owner, -1) || flags[index] {
        return false;
    }
    if let Some(token) = state.token && matches!(token.kind, Kind::Signed) && validates(token) {
        return permitted;
    }
    if index > 10 {
        return false;
    }
    permitted || cfg!(windows)
}";

    assert_eq!(flagged_lines(source), [0usize; 0]);
}

#[test]
fn checks_nested_parts_under_negation_and_parentheses() {
    let source = "fn check(values: &[bool], index: usize, enabled: bool) {
    if !(enabled || (index < values.len() && values[index + 1])) {}
}";

    assert_eq!(flagged_lines(source), [2, 2]);
}

#[test]
fn checks_while_match_guards_function_returns_and_closure_results() {
    let source = "fn check(limit: usize, enabled: bool, mut index: usize) -> bool {
    while index < limit && enabled { index += 1; }
    match index {
        index if index == limit || enabled => return limit > 1 && enabled,
        _ => {}
    }
    let valid = |index| index > limit && enabled;
    if valid(index) {
        index == 0 && enabled
    } else {
        match index {
            1 => index == limit || enabled,
            _ => enabled,
        }
    }
}";

    assert_eq!(flagged_lines(source), [2, 4, 4, 7, 9, 12]);
}

#[test]
fn checks_trait_and_impl_method_results() {
    let source = "trait Check {
    fn check(&self, count: usize, enabled: bool) -> bool {
        count > 2 && enabled
    }
}
impl Check for Value {
    fn check(&self, count: usize, enabled: bool) -> bool {
        (count > 5) && enabled
    }
}";

    assert_eq!(flagged_lines(source), [3, 8]);
}
