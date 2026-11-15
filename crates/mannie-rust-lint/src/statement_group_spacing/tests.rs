use super::check;

fn flagged_lines(source: &str) -> Vec<usize> {
    check(source)
        .unwrap()
        .into_iter()
        .map(|position| position.line)
        .collect()
}

#[test]
fn reports_the_five_boundaries_in_an_unpadded_vault_save() {
    let source = "async fn save() -> Result<(), ApiError> {
    cleanup(collection).await?;
    let pieces = split(value.expose());
    let record = CredentialRecord {
        parts: pieces.len(),
    };
    repository
        .stage_credential(collection, &record)
        .await?;
    for (part, piece) in (0..record.parts).zip(pieces) {
        if secrets.save(part, piece).await.is_err() {
            remove(collection, &record).await?;
            return Err(ApiError::SecretsUnavailable);
        }
    }
    if let Err(error) = repository.keep_credential(collection, &record).await {
        remove(collection, &record).await?;
        return Err(error);
    }
    cleanup(collection).await
}";

    assert_eq!(flagged_lines(source), [3, 7, 10, 16, 20]);
}

#[test]
fn accepts_spaced_sections_and_compact_cleanup_with_return() {
    let source = "async fn save() -> Result<(), ApiError> {
    cleanup(collection).await?;

    let pieces = split(value.expose());
    let record = build_record(&pieces);

    stage(&record).await?;
    confirm_staged().await?;

    for piece in pieces {
        save_piece(piece).await?;
    }

    if let Err(error) = keep_credential(&record).await {
        remove(&record).await?;
        return Err(error);
    }

    cleanup(collection).await
}";

    assert_eq!(flagged_lines(source), [0usize; 0]);
}

#[test]
fn checks_nested_blocks_without_splitting_synchronous_setup() {
    let source = "async fn update() {
    let current = read();

    if current {
        while is_active() {
            prepare().await;
            let next = change();
            publish(next);
        }
        match current {
            true => on_change(),
            false => on_quit(),
        }
    }
}";

    assert_eq!(flagged_lines(source), [7, 10]);
}

#[test]
fn recognizes_a_whitespace_only_line_with_windows_endings() {
    let source =
        "async fn update() {\r\n    let record = read();\r\n  \r\n    save(record).await;\r\n}";

    assert_eq!(flagged_lines(source), [0usize; 0]);
}

#[test]
fn a_comment_alone_does_not_separate_statement_groups() {
    let source = "async fn update() {
    let record = read();
    // Keep the operation tied to its guard.
    save(record).await;
}";

    assert_eq!(flagged_lines(source), [4]);
}

#[test]
fn separates_the_eleven_boundaries_in_expand() {
    let source = r#"fn expand(
    &self,
    text: &str,
    active: &mut BTreeSet<String>,
    depth: usize,
) -> Result<Resolved, ApiError> {
    if depth > 32 {
        return Err(invalid("variableCycle"));
    }
    let mut output = String::new();
    let mut remaining = text;
    let mut secret = false;
    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let after = &remaining[start + 2..];
        let end = after
            .find("}}")
            .ok_or_else(|| invalid("variableReference"))?;
        let name = after[..end].trim();
        if !active.insert(name.to_owned()) {
            return Err(invalid("variableCycle"));
        }
        let value = match self
            .values
            .get(name)
            .ok_or_else(|| invalid("missingVariable"))?
        {
            VariableValue::Plain { text } => self.expand(text, active, depth + 1)?,
            VariableValue::Secret { id } => Resolved {
                value: self.secrets.get(id).ok_or(ApiError::MissingSecret)?.clone(),
                secret: true,
            },
        };
        output.push_str(value.value.expose());
        secret |= value.secret;
        active.remove(name);
        if output.len() > MAX_BODY_BYTES {
            return Err(ApiError::TooLarge);
        }
        remaining = &after[end + 2..];
    }
    output.push_str(remaining);
    if output.len() > MAX_BODY_BYTES {
        return Err(ApiError::TooLarge);
    }
    Ok(Resolved {
        value: Secret::new(output),
        secret,
    })
}"#;
    let expected = [10, 13, 15, 19, 20, 23, 34, 37, 40, 42, 46];

    assert_eq!(flagged_lines(source), expected);

    let mut spaced = String::new();

    for (index, line) in source.lines().enumerate() {
        if expected.contains(&(index + 1)) {
            spaced.push('\n');
        }

        spaced.push_str(line);
        spaced.push('\n');
    }

    assert_eq!(flagged_lines(&spaced), [0usize; 0]);
}

#[test]
fn keeps_a_single_action_with_its_guard_after_an_already_spaced_action() {
    let source = "fn save() {
    prepare();

    cleanup();
    if need_retry() {
        return;
    }
}";

    assert_eq!(flagged_lines(source), [0usize; 0]);
}
