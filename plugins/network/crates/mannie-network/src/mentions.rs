//! Mention provider `network` (ADR-0032; ADR-0075, decision 4): `@` in the
//! chat message box suggests enabled collections and saved requests, and
//! resolves them to strictly redacted summaries.

use std::collections::BTreeMap;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{MentionEntry, MentionSummary};
use mannie_plugin_sdk::points::mentions::{
    MentionContent, MentionContext, MentionProvider, ProviderSuggestions, RenderedMention,
};

use crate::redact::{redact_auth, redact_key_values};
use crate::state::NetworkManager;
use crate::template::redact_url_template_for_agent;

/// Mention provider ID (equals the plugin ID).
pub(crate) const PROVIDER_ID: &str = "network";

/// Variant for a collection mention.
pub(crate) const COLLECTION_VARIANT: &str = "collection";

/// Variant for a saved request mention.
pub(crate) const REQUEST_VARIANT: &str = "request";

/// Failure code when a mentioned collection or request no longer exists or is
/// disabled.
pub(crate) const ITEM_NOT_FOUND: &str = "item_not_found";

/// Mentions of enabled collections and saved requests.
#[derive(Debug, Clone)]
pub(crate) struct NetworkMentions {
    manager: NetworkManager,
}

impl NetworkMentions {
    pub(crate) fn new(manager: NetworkManager) -> Self {
        Self { manager }
    }
}

impl MentionProvider for NetworkMentions {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn offered(&self, _context: &MentionContext) -> bool {
        self.manager
            .snapshot()
            .collections
            .iter()
            .any(|col| col.enabled)
    }

    fn suggest<'a>(
        &'a self,
        _context: &'a MentionContext,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>> {
        let snap = self.manager.snapshot();
        let needle = query.trim().to_ascii_lowercase();
        let mut entries = Vec::new();
        let mut incomplete = false;

        for col in snap.collections.iter().filter(|c| c.enabled) {
            let safe_base = self.manager.redacted_base_url(col);
            let col_matches = needle.is_empty()
                || col.name.to_ascii_lowercase().contains(&needle)
                || safe_base.to_ascii_lowercase().contains(&needle);

            if col_matches {
                if entries.len() >= limit {
                    incomplete = true;
                    break;
                }

                entries.push(MentionEntry {
                    key: format!("collection:{}", col.id),
                    label: col.name.clone(),
                    token: format!("@{}", col.name),
                    variant: COLLECTION_VARIANT.to_owned(),
                    warning: None,
                    expands: false,
                });
            }

            for req in &col.requests {
                let req_label = format!("{} {} ({})", req.method.as_str(), req.name, col.name);
                let safe_url = redact_url_template_for_agent(&req.url_template);
                let req_matches = needle.is_empty()
                    || req_label.to_ascii_lowercase().contains(&needle)
                    || safe_url.to_ascii_lowercase().contains(&needle);

                if req_matches {
                    if entries.len() >= limit {
                        incomplete = true;
                        break;
                    }

                    entries.push(MentionEntry {
                        key: format!("request:{}:{}", col.id, req.id),
                        label: req_label,
                        token: format!("@{}/{}", col.name, req.name),
                        variant: REQUEST_VARIANT.to_owned(),
                        warning: None,
                        expands: false,
                    });
                }
            }

            if incomplete {
                break;
            }
        }

        Box::pin(std::future::ready(Ok(ProviderSuggestions::new(
            entries, incomplete,
        ))))
    }

    fn resolve<'a>(
        &'a self,
        _context: &'a MentionContext,
        key: &'a str,
        max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        let snap = self.manager.snapshot();
        let resolved: Result<Box<dyn MentionContent>, String> = if let Some(col_id) =
            key.strip_prefix("collection:")
        {
            match snap
                .collections
                .iter()
                .find(|c| c.enabled && c.id == col_id)
            {
                Some(col) => {
                    let base = self.manager.redacted_base_url(col);
                    let req_lines: Vec<String> = col
                        .requests
                        .iter()
                        .take(40)
                        .map(|r| {
                            format!(
                                "- {} {} ({}) [request_id: {}]",
                                r.method.as_str(),
                                redact_url_template_for_agent(&r.url_template),
                                r.name,
                                r.id
                            )
                        })
                        .collect();
                    let text = cut_bytes(
                        &format!(
                            "The owner mentioned the Network Client collection \"{}\" (collection_id: {}, protocol: {}, base URL: {}).\nSaved requests:\n{}",
                            col.name,
                            col.id,
                            col.protocol.as_str(),
                            base,
                            if req_lines.is_empty() {
                                "(none)".to_owned()
                            } else {
                                req_lines.join("\n")
                            }
                        ),
                        max_bytes,
                    );
                    Ok(Box::new(MentionedItem {
                        label: col.name.clone(),
                        variant: COLLECTION_VARIANT.to_owned(),
                        text,
                    }))
                }
                None => Err(ITEM_NOT_FOUND.to_owned()),
            }
        } else if let Some(rest) = key.strip_prefix("request:")
            && let Some((col_id, req_id)) = rest.split_once(':')
        {
            let found = snap
                .collections
                .iter()
                .find(|c| c.enabled && c.id == col_id)
                .and_then(|c| c.requests.iter().find(|r| r.id == req_id).map(|r| (c, r)));

            match found {
                Some((col, req)) => {
                    let base = self.manager.redacted_base_url(col);
                    let safe_url = redact_url_template_for_agent(&req.url_template);
                    let headers = redact_key_values(&req.headers, true);
                    let query = redact_key_values(&req.query_params, false);
                    let auth = redact_auth(&req.auth);
                    let detail = serde_json::json!({
                        "collectionId": col.id,
                        "collectionName": col.name,
                        "baseUrl": base,
                        "requestId": req.id,
                        "name": req.name,
                        "protocol": req.protocol.as_str(),
                        "method": req.method.as_str(),
                        "urlTemplate": safe_url,
                        "headers": headers,
                        "queryParams": query,
                        "auth": auth,
                    });
                    let text = cut_bytes(
                        &format!(
                            "The owner mentioned the saved API request \"{}\" ({} {}) in collection \"{}\". Use network_send_request to run it.\n{}",
                            req.name,
                            req.method.as_str(),
                            safe_url,
                            col.name,
                            serde_json::to_string_pretty(&detail).unwrap_or_default()
                        ),
                        max_bytes,
                    );
                    Ok(Box::new(MentionedItem {
                        label: format!("{} {}", req.method.as_str(), req.name),
                        variant: REQUEST_VARIANT.to_owned(),
                        text,
                    }))
                }
                None => Err(ITEM_NOT_FOUND.to_owned()),
            }
        } else {
            Err(ITEM_NOT_FOUND.to_owned())
        };

        Box::pin(std::future::ready(resolved))
    }
}

#[derive(Debug)]
struct MentionedItem {
    label: String,
    variant: String,
    text: String,
}

impl MentionContent for MentionedItem {
    fn render(&self, limit: usize) -> RenderedMention {
        RenderedMention::new(
            cut_bytes(&self.text, limit),
            self.label.clone(),
            self.variant.clone(),
            MentionSummary {
                code: self.variant.clone(),
                values: BTreeMap::new(),
            },
        )
    }
}

fn cut_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }

    let end = text.floor_char_boundary(max_bytes);
    text[..end].to_owned()
}
