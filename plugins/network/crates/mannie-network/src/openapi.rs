//! Bounded `OpenAPI` v2 (Swagger 2.0) and v3 (3.0.x and 3.1.x) importer for
//! JSON and YAML specifications (ADR-0075, decision 7).
//!
//! YAML parsing uses `saphyr-parser` 0.4.0's event iterator with hard bounds
//! on input bytes (10 MiB), expanded bytes (32 MiB, including mapping key
//! bytes), nesting depth (64), total value nodes (200,000, matching JSON
//! value-node counting and including nodes added by alias expansion) and alias
//! expansions (1,000). Local `$ref` JSON pointers resolve with cycle and depth
//! guards; remote `$ref` URLs are never fetched. Imports cap at 2,000 endpoints
//! with a warning.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use saphyr_parser::{Event, Parser, ScalarStyle, Tag};
use serde_json::{Map, Number, Value};

use crate::model::{
    ApiKeyLocation, AuthConfig, BodyConfig, DEFAULT_TIMEOUT_SECONDS, HttpMethod, KeyValueEntry,
    MAX_SPEC_BYTES, MissingBaseUrlSource, NetworkError, OpenApiPreview, OpenApiPreviewEndpoint,
    OpenApiWarning, VariableEntry,
};
use crate::redact::{is_secret_header_name, is_secret_name};
use crate::template::sanitize_saved_url;

const MAX_YAML_DEPTH: usize = 64;
const MAX_YAML_NODES: usize = 200_000;
const MAX_YAML_ALIASES: usize = 1_000;
const MAX_YAML_EXPANDED_BYTES: usize = 32 * 1024 * 1024;
const NODE_OVERHEAD_BYTES: usize = 16;
const MAX_REF_DEPTH: usize = 32;
const MAX_SCHEMA_DEPTH: usize = 12;
const MAX_ENDPOINTS: usize = 2_000;

/// Parsed environment blueprint derived from `OpenAPI` 3 `servers`.
#[derive(Debug, Clone)]
pub(crate) struct ImportedEnvironment {
    pub(crate) name: String,
    pub(crate) variables: Vec<VariableEntry>,
}

/// Parsed saved-request blueprint derived from one `OpenAPI` operation.
#[derive(Debug, Clone)]
pub(crate) struct ImportedRequest {
    pub(crate) folder: Option<String>,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) method: HttpMethod,
    pub(crate) url_template: String,
    pub(crate) path_params: Vec<KeyValueEntry>,
    pub(crate) query_params: Vec<KeyValueEntry>,
    pub(crate) headers: Vec<KeyValueEntry>,
    pub(crate) auth: AuthConfig,
    pub(crate) body: BodyConfig,
    pub(crate) timeout_seconds: u32,
}

/// Complete imported collection blueprint kept in memory until confirmed.
#[derive(Debug, Clone)]
pub(crate) struct ImportedCollection {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) spec_version: String,
    pub(crate) base_url: String,
    pub(crate) auth: AuthConfig,
    pub(crate) query_params: Vec<KeyValueEntry>,
    pub(crate) variables: Vec<VariableEntry>,
    pub(crate) folders: Vec<String>,
    pub(crate) environments: Vec<ImportedEnvironment>,
    pub(crate) requests: Vec<ImportedRequest>,
    pub(crate) security_schemes: Vec<String>,
    pub(crate) warning_items: Vec<OpenApiWarning>,
    pub(crate) warnings: Vec<String>,
}

impl ImportedCollection {
    /// Builds the UI preview summary for this parsed `OpenAPI` specification.
    pub(crate) fn preview(&self, preview_id: String) -> OpenApiPreview {
        OpenApiPreview {
            preview_id,
            title: self.title.clone(),
            description: self.description.clone(),
            spec_version: self.spec_version.clone(),
            base_url: self.base_url.clone(),
            environments: self
                .environments
                .iter()
                .map(|env| env.name.clone())
                .collect(),
            folders: self.folders.clone(),
            endpoints: self
                .requests
                .iter()
                .map(|req| OpenApiPreviewEndpoint {
                    folder: req.folder.clone(),
                    name: req.name.clone(),
                    method: req.method,
                    path: req.url_template.clone(),
                    description: req.description.clone(),
                })
                .collect(),
            security_schemes: self.security_schemes.clone(),
            warnings: self.warnings.clone(),
            warning_items: self.warning_items.clone(),
        }
    }
}

fn format_legacy_warning(warning: &OpenApiWarning) -> String {
    match warning {
        OpenApiWarning::MissingBaseUrl {
            source: MissingBaseUrlSource::V2Host,
        } => "Specification did not declare a host; set the collection base URL before sending."
            .to_owned(),
        OpenApiWarning::MissingBaseUrl {
            source: MissingBaseUrlSource::V3Servers,
        } => "Specification declared no servers; set the collection base URL before sending."
            .to_owned(),
        OpenApiWarning::CookieParameter {
            endpoint,
            cookie_name,
        } => format!(
            "Endpoint \"{endpoint}\" uses cookie parameter \"{cookie_name}\"; manage it in the Cookie Jar."
        ),
        OpenApiWarning::CookieSecurityScheme {
            scheme,
            cookie_name,
        } => format!(
            "Security scheme \"{scheme}\" uses cookie \"{cookie_name}\"; add it in the Cookie Jar."
        ),
        OpenApiWarning::SkippedExternalRef { reference } => {
            format!("Skipped external $ref \"{reference}\" (remote references are never fetched).")
        }
        OpenApiWarning::CyclicRef { reference } => format!("Stopped cyclic $ref \"{reference}\"."),
        OpenApiWarning::UnresolvedLocalRef { reference } => {
            format!("Unresolved local $ref \"{reference}\".")
        }
        OpenApiWarning::DeepRefChain => "Stopped deeply nested $ref chain.".to_owned(),
        OpenApiWarning::EndpointLimitReached { max_endpoints } => {
            format!("Stopped after {max_endpoints} endpoints.")
        }
        OpenApiWarning::CombinedSecuritySchemes { context } => format!(
            "{context} combines multiple security schemes in one requirement; only the first supported scheme is attached automatically."
        ),
        OpenApiWarning::UnsupportedOperationSecurity { operation } => format!(
            "Endpoint \"{operation}\" requires a security scheme that cannot be attached automatically; configure its authentication manually."
        ),
    }
}

/// Parses an `OpenAPI` v2 or v3 document from JSON or YAML text.
pub(crate) fn parse_openapi(text: &str) -> Result<ImportedCollection, NetworkError> {
    if text.len() > MAX_SPEC_BYTES {
        return Err(NetworkError::SpecTooLarge);
    }

    let trimmed = text.trim();

    if trimmed.is_empty() {
        return Err(NetworkError::InvalidSpec {
            detail: "the document is empty".to_owned(),
        });
    }

    let root = parse_json_or_yaml(trimmed)?;
    let obj = root.as_object().ok_or_else(|| NetworkError::InvalidSpec {
        detail: "the root of the OpenAPI specification must be an object".to_owned(),
    })?;

    if let Some(swagger) = obj.get("swagger").and_then(Value::as_str) {
        if swagger == "2.0" {
            return import_swagger_v2(&root, obj);
        }

        return Err(NetworkError::UnsupportedSpecVersion {
            version: swagger.to_owned(),
        });
    }

    if let Some(openapi) = obj.get("openapi").and_then(Value::as_str) {
        if openapi.starts_with("3.0.")
            || openapi.starts_with("3.1.")
            || openapi == "3.0"
            || openapi == "3.1"
        {
            return import_openapi_v3(&root, obj, openapi);
        }

        return Err(NetworkError::UnsupportedSpecVersion {
            version: openapi.to_owned(),
        });
    }

    Err(NetworkError::UnsupportedSpecVersion {
        version: "unknown".to_owned(),
    })
}

/// Parses JSON when the document starts with `{`, otherwise parses YAML 1.2
/// with bounded depth, node count, expanded byte size and alias limits.
fn parse_json_or_yaml(text: &str) -> Result<Value, NetworkError> {
    if text.starts_with('{')
        && let Ok(value) = serde_json::from_str::<Value>(text)
    {
        let mut nodes = 0usize;
        let mut expanded_bytes = 0usize;
        check_value_bounds(&value, 0, &mut nodes, &mut expanded_bytes)?;
        return Ok(value);
    }

    parse_yaml_bounded(text)
}

fn charge_expanded_bytes(expanded_bytes: &mut usize, add: usize) -> Result<(), NetworkError> {
    *expanded_bytes = expanded_bytes.saturating_add(add);

    if *expanded_bytes > MAX_YAML_EXPANDED_BYTES {
        return Err(NetworkError::SpecTooComplex);
    }

    Ok(())
}

fn check_value_bounds(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
    expanded_bytes: &mut usize,
) -> Result<(), NetworkError> {
    if depth > MAX_YAML_DEPTH {
        return Err(NetworkError::SpecTooComplex);
    }

    *nodes = nodes.saturating_add(1);
    if *nodes > MAX_YAML_NODES {
        return Err(NetworkError::SpecTooComplex);
    }

    let scalar_bytes = match value {
        Value::String(text) => text.len(),
        _other => 0,
    };
    charge_expanded_bytes(
        expanded_bytes,
        NODE_OVERHEAD_BYTES.saturating_add(scalar_bytes),
    )?;

    match value {
        Value::Array(items) => {
            for item in items {
                check_value_bounds(item, depth + 1, nodes, expanded_bytes)?;
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                charge_expanded_bytes(expanded_bytes, key.len())?;
                check_value_bounds(item, depth + 1, nodes, expanded_bytes)?;
            }
        }
        _scalar => {}
    }
    Ok(())
}

enum Frame {
    Sequence {
        anchor: usize,
        items: Vec<Value>,
    },
    Mapping {
        anchor: usize,
        entries: Map<String, Value>,
        pending_key: Option<String>,
    },
}

#[derive(Default)]
struct YamlBuilder {
    stack: Vec<Frame>,
    anchors: HashMap<usize, Value>,
    root: Option<Value>,
    nodes: usize,
    aliases: usize,
    expanded_bytes: usize,
}

impl YamlBuilder {
    fn count_node(&mut self, payload_bytes: usize) -> Result<(), NetworkError> {
        self.nodes = self.nodes.saturating_add(1);

        if self.nodes > MAX_YAML_NODES {
            return Err(NetworkError::SpecTooComplex);
        }

        charge_expanded_bytes(
            &mut self.expanded_bytes,
            NODE_OVERHEAD_BYTES.saturating_add(payload_bytes),
        )
    }

    fn expecting_mapping_key(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(Frame::Mapping {
                pending_key: None,
                ..
            })
        )
    }

    fn push_container(&mut self, frame: Frame) -> Result<(), NetworkError> {
        if self.expecting_mapping_key() {
            return Err(NetworkError::InvalidSpec {
                detail: "YAML mapping keys must be scalar values".to_owned(),
            });
        }
        if self.stack.len() >= MAX_YAML_DEPTH {
            return Err(NetworkError::SpecTooComplex);
        }

        self.count_node(0)?;
        self.stack.push(frame);
        Ok(())
    }

    fn finish_value(&mut self, anchor: usize, value: Value) -> Result<(), NetworkError> {
        if anchor != 0 {
            self.anchors.insert(anchor, value.clone());
        }

        push_value(&mut self.stack, &mut self.root, value)
    }

    fn apply_scalar(
        &mut self,
        raw: &str,
        style: ScalarStyle,
        anchor: usize,
        tag: Option<&Tag>,
    ) -> Result<(), NetworkError> {
        let value = yaml_scalar_to_json(raw, style, tag);

        // Match `check_value_bounds` for JSON objects: count value nodes
        // against `MAX_YAML_NODES`, while charging every mapping key's byte
        // length to `expanded_bytes` (ADR-0075, T-116).
        if self.expecting_mapping_key() {
            let key_len = mapping_key_len(&value)?;
            let anchor_bytes = if anchor == 0 {
                0
            } else {
                NODE_OVERHEAD_BYTES.saturating_add(key_len)
            };

            charge_expanded_bytes(
                &mut self.expanded_bytes,
                key_len.saturating_add(anchor_bytes),
            )?;

            if anchor != 0 {
                self.anchors.insert(anchor, value.clone());
            }

            let key = value_to_mapping_key(value)?;

            if let Some(Frame::Mapping { pending_key, .. }) = self.stack.last_mut() {
                *pending_key = Some(key);
            }
            return Ok(());
        }

        let payload_bytes = match &value {
            Value::String(text) => text.len(),
            _other => 0,
        };

        self.count_node(payload_bytes)?;
        self.finish_value(anchor, value)
    }

    fn apply_alias(&mut self, anchor: usize) -> Result<(), NetworkError> {
        self.aliases = self.aliases.saturating_add(1);

        if self.aliases > MAX_YAML_ALIASES {
            return Err(NetworkError::SpecTooComplex);
        }

        let Some(target_ref) = self.anchors.get(&anchor) else {
            return Err(NetworkError::InvalidSpec {
                detail: "YAML alias references an unknown anchor".to_owned(),
            });
        };

        if self.expecting_mapping_key() {
            let key_len = mapping_key_len(target_ref)?;
            charge_expanded_bytes(&mut self.expanded_bytes, key_len)?;

            let key = value_to_mapping_key(target_ref.clone())?;

            if let Some(Frame::Mapping { pending_key, .. }) = self.stack.last_mut() {
                *pending_key = Some(key);
            }
            return Ok(());
        }

        // Account for the full node count, depth and expanded byte budget
        // BEFORE cloning the aliased subtree (ADR-0075, T-116).
        check_value_bounds(
            target_ref,
            self.stack.len(),
            &mut self.nodes,
            &mut self.expanded_bytes,
        )?;

        let target = target_ref.clone();
        push_value(&mut self.stack, &mut self.root, target)
    }

    fn apply_event(&mut self, event: Event<'_>) -> Result<bool, NetworkError> {
        match event {
            Event::Nothing | Event::StreamStart | Event::DocumentStart(_) => Ok(false),
            Event::DocumentEnd | Event::StreamEnd => Ok(self.root.is_some()),
            Event::SequenceStart(anchor, _tag) => {
                self.push_container(Frame::Sequence {
                    anchor,
                    items: Vec::new(),
                })?;
                Ok(false)
            }
            Event::SequenceEnd => {
                let Some(Frame::Sequence { anchor, items }) = self.stack.pop() else {
                    return Err(NetworkError::InvalidSpec {
                        detail: "unexpected end of YAML sequence".to_owned(),
                    });
                };
                self.finish_value(anchor, Value::Array(items))?;
                Ok(false)
            }
            Event::MappingStart(anchor, _tag) => {
                self.push_container(Frame::Mapping {
                    anchor,
                    entries: Map::new(),
                    pending_key: None,
                })?;
                Ok(false)
            }
            Event::MappingEnd => {
                let Some(Frame::Mapping {
                    anchor,
                    entries,
                    pending_key,
                }) = self.stack.pop()
                else {
                    return Err(NetworkError::InvalidSpec {
                        detail: "unexpected end of YAML mapping".to_owned(),
                    });
                };

                if pending_key.is_some() {
                    return Err(NetworkError::InvalidSpec {
                        detail: "dangling key in YAML mapping".to_owned(),
                    });
                }

                self.finish_value(anchor, Value::Object(entries))?;
                Ok(false)
            }
            Event::Scalar(raw, style, anchor, tag) => {
                self.apply_scalar(&raw, style, anchor, tag.as_deref())?;
                Ok(false)
            }
            Event::Alias(anchor) => {
                self.apply_alias(anchor)?;
                Ok(false)
            }
        }
    }
}

fn mapping_key_len(value: &Value) -> Result<usize, NetworkError> {
    match value {
        Value::String(text) => Ok(text.len()),
        Value::Number(num) => Ok(num.to_string().len()),
        Value::Bool(flag) => Ok(if *flag { 4 } else { 5 }),
        Value::Null => Ok(4),
        _other => Err(NetworkError::InvalidSpec {
            detail: "YAML mapping keys must be scalar values".to_owned(),
        }),
    }
}

fn value_to_mapping_key(value: Value) -> Result<String, NetworkError> {
    match value {
        Value::String(text) => Ok(text),
        Value::Number(num) => Ok(num.to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Null => Ok("null".to_owned()),
        _other => Err(NetworkError::InvalidSpec {
            detail: "YAML mapping keys must be scalar values".to_owned(),
        }),
    }
}

/// Streams YAML 1.2 events from `saphyr-parser` directly into
/// `serde_json::Value`, aborting before allocating past hard depth, node or
/// alias limits.
fn parse_yaml_bounded(text: &str) -> Result<Value, NetworkError> {
    let mut builder = YamlBuilder::default();

    for next in Parser::new_from_str(text) {
        let (event, _span) = next.map_err(|error| NetworkError::InvalidSpec {
            detail: error.to_string(),
        })?;

        if builder.apply_event(event)? {
            break;
        }
    }

    builder.root.ok_or_else(|| NetworkError::InvalidSpec {
        detail: "the YAML document contained no root value".to_owned(),
    })
}

fn push_value(
    stack: &mut [Frame],
    root: &mut Option<Value>,
    value: Value,
) -> Result<(), NetworkError> {
    match stack.last_mut() {
        None => {
            *root = Some(value);
            Ok(())
        }
        Some(Frame::Sequence { items, .. }) => {
            items.push(value);
            Ok(())
        }
        Some(Frame::Mapping {
            entries,
            pending_key,
            ..
        }) => {
            if let Some(key) = pending_key.take() {
                if key == "<<" {
                    merge_yaml_mapping(entries, value);
                } else {
                    entries.insert(key, value);
                }
            } else {
                let key = value_to_mapping_key(value)?;
                *pending_key = Some(key);
            }
            Ok(())
        }
    }
}

/// Supports YAML `<<` merge keys when an `OpenAPI` document uses anchors to share
/// response or parameter properties.
fn merge_yaml_mapping(entries: &mut Map<String, Value>, value: Value) {
    match value {
        Value::Object(source) => {
            for (k, v) in source {
                entries.entry(k).or_insert(v);
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Value::Object(source) = item {
                    for (k, v) in source {
                        entries.entry(k).or_insert(v);
                    }
                }
            }
        }
        _other => {}
    }
}

fn yaml_scalar_to_json(raw: &str, style: ScalarStyle, tag: Option<&Tag>) -> Value {
    if let Some(tag) = tag
        && tag.is_yaml_core_schema()
    {
        match tag.suffix.as_str() {
            "str" => return Value::String(raw.to_owned()),
            "null" => return Value::Null,
            "bool" => {
                if raw.eq_ignore_ascii_case("true") {
                    return Value::Bool(true);
                }
                if raw.eq_ignore_ascii_case("false") {
                    return Value::Bool(false);
                }
            }
            "int" => {
                if let Ok(int) = parse_yaml_int(raw) {
                    return int;
                }
            }
            "float" => {
                if let Ok(float) = raw.parse::<f64>()
                    && let Some(num) = Number::from_f64(float)
                {
                    return Value::Number(num);
                }
            }
            _other => {}
        }
    }

    if style != ScalarStyle::Plain {
        return Value::String(raw.to_owned());
    }

    match raw {
        "" | "~" | "null" | "Null" | "NULL" => Value::Null,
        "true" | "True" | "TRUE" => Value::Bool(true),
        "false" | "False" | "FALSE" => Value::Bool(false),
        other => {
            if let Ok(int) = parse_yaml_int(other) {
                return int;
            }
            if let Ok(float) = other.parse::<f64>()
                && float.is_finite()
                && let Some(num) = Number::from_f64(float)
            {
                return Value::Number(num);
            }
            Value::String(other.to_owned())
        }
    }
}

fn parse_yaml_int(raw: &str) -> Result<Value, ()> {
    if let Some(hex) = raw.strip_prefix("0x") {
        let value = i64::from_str_radix(hex, 16).map_err(|_err| ())?;
        return Ok(Value::Number(Number::from(value)));
    }
    if let Some(oct) = raw.strip_prefix("0o") {
        let value = i64::from_str_radix(oct, 8).map_err(|_err| ())?;
        return Ok(Value::Number(Number::from(value)));
    }

    if let Ok(signed) = raw.parse::<i64>() {
        return Ok(Value::Number(Number::from(signed)));
    }
    if let Ok(unsigned) = raw.parse::<u64>() {
        return Ok(Value::Number(Number::from(unsigned)));
    }

    Err(())
}

/// Resolves local JSON pointers (`#/...`) with depth and cycle guards.
/// External `$ref` pointers are never fetched and record a warning.
fn resolve_ref<'a>(
    root: &'a Value,
    value: &'a Value,
    warnings: &mut Vec<OpenApiWarning>,
) -> &'a Value {
    let mut current = value;
    let mut visited = HashSet::new();

    for _depth in 0..MAX_REF_DEPTH {
        let Some(ref_str) = current.get("$ref").and_then(Value::as_str) else {
            return current;
        };

        if !ref_str.starts_with("#/") {
            push_unique_warning(
                warnings,
                OpenApiWarning::SkippedExternalRef {
                    reference: ref_str.to_owned(),
                },
            );
            return current;
        }
        if !visited.insert(ref_str.to_owned()) {
            push_unique_warning(
                warnings,
                OpenApiWarning::CyclicRef {
                    reference: ref_str.to_owned(),
                },
            );
            return current;
        }

        if let Some(target) = root.pointer(&ref_str[1..]) {
            current = target;
        } else {
            push_unique_warning(
                warnings,
                OpenApiWarning::UnresolvedLocalRef {
                    reference: ref_str.to_owned(),
                },
            );
            return current;
        }
    }

    push_unique_warning(warnings, OpenApiWarning::DeepRefChain);
    current
}

fn push_unique_warning(warnings: &mut Vec<OpenApiWarning>, warning: OpenApiWarning) {
    if warnings.len() < 50 && !warnings.contains(&warning) {
        warnings.push(warning);
    }
}

fn derive_v2_base_url(
    obj: &Map<String, Value>,
    warnings: &mut Vec<OpenApiWarning>,
) -> Result<(String, Vec<KeyValueEntry>), NetworkError> {
    let schemes: Vec<&str> = obj
        .get("schemes")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let scheme = if schemes.contains(&"https") || schemes.is_empty() {
        "https"
    } else if schemes.contains(&"http") {
        "http"
    } else {
        "https"
    };
    let host = obj.get("host").and_then(Value::as_str).unwrap_or("").trim();
    let base_path = obj
        .get("basePath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let raw_base_url = if host.is_empty() {
        push_unique_warning(
            warnings,
            OpenApiWarning::MissingBaseUrl {
                source: MissingBaseUrlSource::V2Host,
            },
        );
        String::new()
    } else {
        format!("{scheme}://{host}{}", normalize_base_path(base_path))
    };

    sanitize_saved_url(&raw_base_url)
}

fn import_swagger_v2(
    root: &Value,
    obj: &Map<String, Value>,
) -> Result<ImportedCollection, NetworkError> {
    let mut warning_items = Vec::new();
    let (title, description) = read_info(obj);
    let (base_url, collection_query_params) = derive_v2_base_url(obj, &mut warning_items)?;

    let parsed_security =
        extract_v2_security(obj.get("securityDefinitions").and_then(Value::as_object));
    let root_auth =
        resolve_root_security(obj.get("security"), &parsed_security, &mut warning_items);

    let mut folders_set: BTreeSet<String> = BTreeSet::new();
    let mut requests = Vec::new();

    if let Some(paths) = obj.get("paths").and_then(Value::as_object) {
        for (raw_path, raw_item) in paths {
            let item = resolve_ref(root, raw_item, &mut warning_items);
            let Some(item_obj) = item.as_object() else {
                continue;
            };
            let path_params_raw = item_obj
                .get("parameters")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            for method_key in ["get", "post", "put", "patch", "delete", "head", "options"] {
                let Some(op_val) = item_obj.get(method_key) else {
                    continue;
                };
                let Some(op_obj) = resolve_ref(root, op_val, &mut warning_items).as_object() else {
                    continue;
                };
                let Some(method) = HttpMethod::parse(method_key) else {
                    continue;
                };

                if requests.len() >= MAX_ENDPOINTS {
                    push_unique_warning(
                        &mut warning_items,
                        OpenApiWarning::EndpointLimitReached {
                            max_endpoints: u32::try_from(MAX_ENDPOINTS).unwrap_or(u32::MAX),
                        },
                    );
                    break;
                }

                let req = import_v2_operation(
                    root,
                    raw_path,
                    method,
                    op_obj,
                    &path_params_raw,
                    &parsed_security,
                    &root_auth,
                    &mut warning_items,
                )?;
                if let Some(tag) = &req.folder {
                    folders_set.insert(tag.clone());
                }
                requests.push(req);
            }
        }
    }

    let warnings = warning_items.iter().map(format_legacy_warning).collect();

    Ok(ImportedCollection {
        title,
        description,
        spec_version: "2.0".to_owned(),
        base_url,
        auth: root_auth,
        query_params: collection_query_params,
        variables: parsed_security.variables,
        folders: folders_set.into_iter().collect(),
        environments: Vec::new(),
        requests,
        security_schemes: parsed_security.labels,
        warning_items,
        warnings,
    })
}

fn make_kv_param(
    name: &str,
    value: String,
    enabled: bool,
    header_rules: bool,
    description: String,
) -> KeyValueEntry {
    KeyValueEntry {
        key: name.to_owned(),
        value,
        enabled,
        secret: if header_rules {
            is_secret_header_name(name)
        } else {
            is_secret_name(name)
        },
        description,
    }
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "Swagger 2.0 parameter locations (path, query, header, formData, body) are decoded in one pass"
)]
fn import_v2_operation(
    root: &Value,
    raw_path: &str,
    method: HttpMethod,
    op_obj: &Map<String, Value>,
    path_params_raw: &[Value],
    parsed_security: &ParsedSecurity,
    root_auth: &AuthConfig,
    warnings: &mut Vec<OpenApiWarning>,
) -> Result<ImportedRequest, NetworkError> {
    let folder = first_tag(op_obj);
    let (clean_path, extracted_query) = sanitize_saved_url(raw_path)?;
    let name = operation_name(op_obj, method, &clean_path);
    let description = operation_description(op_obj);
    let auth = resolve_operation_security(
        op_obj.get("security"),
        parsed_security,
        root_auth,
        &name,
        warnings,
    );

    let mut combined_params = path_params_raw.to_vec();
    if let Some(op_params) = op_obj.get("parameters").and_then(Value::as_array) {
        combined_params.extend(op_params.iter().cloned());
    }

    let mut path_params = Vec::new();
    let mut query_params = extracted_query;
    let mut headers = Vec::new();
    let mut form_entries = Vec::new();
    let mut json_body: Option<String> = None;

    for param_raw in &combined_params {
        let param = resolve_ref(root, param_raw, warnings);
        let Some(p_obj) = param.as_object() else {
            continue;
        };
        let location = p_obj.get("in").and_then(Value::as_str).unwrap_or("");
        let p_name = p_obj
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let p_desc = p_obj
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        let required = p_obj
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let default_val = scalar_example(p_obj);
        let enabled = required || !default_val.is_empty();

        match location {
            "path" if !p_name.is_empty() => {
                upsert_kv(
                    &mut path_params,
                    make_kv_param(p_name, default_val, true, false, p_desc),
                );
            }
            "query" if !p_name.is_empty() => {
                upsert_kv(
                    &mut query_params,
                    make_kv_param(p_name, default_val, enabled, false, p_desc),
                );
            }
            "header" if !p_name.is_empty() => {
                upsert_kv(
                    &mut headers,
                    make_kv_param(p_name, default_val, enabled, true, p_desc),
                );
            }
            "formData" if !p_name.is_empty() => {
                upsert_kv(
                    &mut form_entries,
                    make_kv_param(p_name, default_val, enabled, false, p_desc),
                );
            }
            "body" => {
                if let Some(schema) = p_obj.get("schema") {
                    let synth = synthesize_example(root, schema, 0, warnings);
                    if let Ok(pretty) = serde_json::to_string_pretty(&synth) {
                        json_body = Some(pretty);
                    }
                }
            }
            _other => {}
        }
    }

    let body = if let Some(content) = json_body {
        BodyConfig::Json { content }
    } else if !form_entries.is_empty() {
        BodyConfig::FormUrlEncoded {
            entries: form_entries,
        }
    } else {
        BodyConfig::None
    };

    Ok(ImportedRequest {
        folder,
        name,
        description,
        method,
        url_template: clean_path,
        path_params,
        query_params,
        headers,
        auth,
        body,
        timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
    })
}

fn import_openapi_v3(
    root: &Value,
    obj: &Map<String, Value>,
    version: &str,
) -> Result<ImportedCollection, NetworkError> {
    let mut warning_items = Vec::new();
    let (title, description) = read_info(obj);

    let (base_url, collection_query_params, server_variables, environments) =
        extract_v3_servers(obj, &mut warning_items)?;
    let parsed_security = extract_v3_security(
        root,
        obj.get("components")
            .and_then(Value::as_object)
            .and_then(|c| c.get("securitySchemes"))
            .and_then(Value::as_object),
        &mut warning_items,
    );
    let root_auth =
        resolve_root_security(obj.get("security"), &parsed_security, &mut warning_items);
    let mut variables = parsed_security.variables.clone();

    for var in server_variables {
        if !variables.iter().any(|existing| existing.key == var.key) {
            variables.push(var);
        }
    }

    let mut folders_set: BTreeSet<String> = BTreeSet::new();
    let mut requests = Vec::new();

    if let Some(paths) = obj.get("paths").and_then(Value::as_object) {
        for (raw_path, raw_item) in paths {
            let item = resolve_ref(root, raw_item, &mut warning_items);
            let Some(item_obj) = item.as_object() else {
                continue;
            };
            let path_params_raw = item_obj
                .get("parameters")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            for method_key in ["get", "post", "put", "patch", "delete", "head", "options"] {
                let Some(op_val) = item_obj.get(method_key) else {
                    continue;
                };
                let Some(op_obj) = resolve_ref(root, op_val, &mut warning_items).as_object() else {
                    continue;
                };
                let Some(method) = HttpMethod::parse(method_key) else {
                    continue;
                };

                if requests.len() >= MAX_ENDPOINTS {
                    push_unique_warning(
                        &mut warning_items,
                        OpenApiWarning::EndpointLimitReached {
                            max_endpoints: u32::try_from(MAX_ENDPOINTS).unwrap_or(u32::MAX),
                        },
                    );
                    break;
                }

                let req = import_v3_operation(
                    root,
                    raw_path,
                    method,
                    op_obj,
                    &path_params_raw,
                    &parsed_security,
                    &root_auth,
                    &mut warning_items,
                )?;
                if let Some(tag) = &req.folder {
                    folders_set.insert(tag.clone());
                }
                requests.push(req);
            }
        }
    }

    let warnings = warning_items.iter().map(format_legacy_warning).collect();

    Ok(ImportedCollection {
        title,
        description,
        spec_version: version.to_owned(),
        base_url,
        auth: root_auth,
        query_params: collection_query_params,
        variables,
        folders: folders_set.into_iter().collect(),
        environments,
        requests,
        security_schemes: parsed_security.labels,
        warning_items,
        warnings,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "operation import receives the resolved spec context, security definitions and warnings"
)]
fn import_v3_operation(
    root: &Value,
    raw_path: &str,
    method: HttpMethod,
    op_obj: &Map<String, Value>,
    path_params_raw: &[Value],
    parsed_security: &ParsedSecurity,
    root_auth: &AuthConfig,
    warnings: &mut Vec<OpenApiWarning>,
) -> Result<ImportedRequest, NetworkError> {
    let folder = first_tag(op_obj);
    let (clean_path, extracted_query) = sanitize_saved_url(raw_path)?;
    let name = operation_name(op_obj, method, &clean_path);
    let description = operation_description(op_obj);
    let auth = resolve_operation_security(
        op_obj.get("security"),
        parsed_security,
        root_auth,
        &name,
        warnings,
    );

    let mut combined_params = path_params_raw.to_vec();
    if let Some(op_params) = op_obj.get("parameters").and_then(Value::as_array) {
        combined_params.extend(op_params.iter().cloned());
    }

    let mut path_params = Vec::new();
    let mut query_params = extracted_query;
    let mut headers = Vec::new();

    for param_raw in &combined_params {
        let param = resolve_ref(root, param_raw, warnings);
        let Some(p_obj) = param.as_object() else {
            continue;
        };
        let location = p_obj.get("in").and_then(Value::as_str).unwrap_or("");
        let p_name = p_obj
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let p_desc = p_obj
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        let required = p_obj
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let default_val = v3_param_example(root, p_obj, warnings);
        let has_default = !default_val.is_empty();

        match location {
            "path" if !p_name.is_empty() => upsert_kv(
                &mut path_params,
                KeyValueEntry {
                    key: p_name.to_owned(),
                    value: default_val,
                    enabled: true,
                    secret: is_secret_name(p_name),
                    description: p_desc,
                },
            ),
            "query" if !p_name.is_empty() => upsert_kv(
                &mut query_params,
                KeyValueEntry {
                    key: p_name.to_owned(),
                    value: default_val,
                    enabled: required || has_default,
                    secret: is_secret_name(p_name),
                    description: p_desc,
                },
            ),
            "header" if !p_name.is_empty() => upsert_kv(
                &mut headers,
                KeyValueEntry {
                    key: p_name.to_owned(),
                    value: default_val,
                    enabled: required || has_default,
                    secret: is_secret_header_name(p_name),
                    description: p_desc,
                },
            ),
            "cookie" if !p_name.is_empty() => push_unique_warning(
                warnings,
                OpenApiWarning::CookieParameter {
                    endpoint: name.clone(),
                    cookie_name: p_name.to_owned(),
                },
            ),
            _other => {}
        }
    }

    let body = extract_v3_body(root, op_obj, warnings);

    Ok(ImportedRequest {
        folder,
        name,
        description,
        method,
        url_template: clean_path,
        path_params,
        query_params,
        headers,
        auth,
        body,
        timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
    })
}

fn read_info(obj: &Map<String, Value>) -> (String, String) {
    let info = obj.get("info").and_then(Value::as_object);
    let raw_title = info
        .and_then(|i| i.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("Imported API")
        .trim();
    let title: String = raw_title
        .chars()
        .filter(|ch| !ch.is_control())
        .take(120)
        .collect();
    let title = if title.is_empty() {
        "Imported API".to_owned()
    } else {
        title
    };
    let description: String = info
        .and_then(|i| i.get("description"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .chars()
        .filter(|ch| *ch != '\0')
        .take(1_800)
        .collect();
    (title, description)
}

fn normalize_base_path(path: &str) -> String {
    if path.is_empty() || path == "/" {
        String::new()
    } else if path.starts_with('/') {
        path.trim_end_matches('/').to_owned()
    } else {
        format!("/{}", path.trim_end_matches('/'))
    }
}

type ExtractedServers = (
    String,
    Vec<KeyValueEntry>,
    Vec<VariableEntry>,
    Vec<ImportedEnvironment>,
);

#[expect(
    clippy::too_many_lines,
    reason = "server extraction handles empty, single-server and multi-server environment generation in sequence"
)]
fn extract_v3_servers(
    obj: &Map<String, Value>,
    warnings: &mut Vec<OpenApiWarning>,
) -> Result<ExtractedServers, NetworkError> {
    let Some(servers) = obj.get("servers").and_then(Value::as_array) else {
        push_unique_warning(
            warnings,
            OpenApiWarning::MissingBaseUrl {
                source: MissingBaseUrlSource::V3Servers,
            },
        );
        return Ok((String::new(), Vec::new(), Vec::new(), Vec::new()));
    };

    if servers.is_empty() {
        push_unique_warning(
            warnings,
            OpenApiWarning::MissingBaseUrl {
                source: MissingBaseUrlSource::V3Servers,
            },
        );
        return Ok((String::new(), Vec::new(), Vec::new(), Vec::new()));
    }

    let mut rendered_servers: Vec<(String, String, Vec<VariableEntry>, Vec<KeyValueEntry>)> =
        Vec::new();

    for (index, server) in servers.iter().enumerate() {
        let Some(server_obj) = server.as_object() else {
            continue;
        };
        let raw_url = server_obj
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();

        if raw_url.is_empty() {
            continue;
        }

        let (mut url_template, extracted_query) = sanitize_saved_url(raw_url)?;
        let mut vars = Vec::new();

        if let Some(var_map) = server_obj.get("variables").and_then(Value::as_object) {
            for (name, def) in var_map {
                let default = def
                    .get("default")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let desc = def
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                url_template =
                    url_template.replace(&format!("{{{name}}}"), &format!("{{{{{name}}}}}"));
                vars.push(VariableEntry {
                    key: name.clone(),
                    value: default,
                    enabled: true,
                    secret: is_secret_name(name),
                    description: desc,
                });
            }
        }

        let label = server_obj
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|desc| {
                !desc.is_empty()
                    && desc.chars().count() <= 80
                    && !desc.chars().any(char::is_control)
            })
            .map_or_else(|| format!("Server {}", index + 1), str::to_owned);
        rendered_servers.push((label, url_template, vars, extracted_query));
    }

    if rendered_servers.is_empty() {
        push_unique_warning(
            warnings,
            OpenApiWarning::MissingBaseUrl {
                source: MissingBaseUrlSource::V3Servers,
            },
        );
        return Ok((String::new(), Vec::new(), Vec::new(), Vec::new()));
    }

    if rendered_servers.len() == 1 {
        let (_, url, vars, extracted_query) = rendered_servers.remove(0);
        return Ok((url, extracted_query, vars, Vec::new()));
    }

    let mut used_names: BTreeSet<String> = BTreeSet::new();
    let mut environments = Vec::new();
    let mut collection_query = Vec::new();

    for (index, (label, url, mut vars, extracted_query)) in rendered_servers.into_iter().enumerate()
    {
        let mut env_name = label;

        if !used_names.insert(env_name.to_ascii_lowercase()) {
            env_name = format!("{env_name} ({})", index + 1);
            used_names.insert(env_name.to_ascii_lowercase());
        }

        for entry in extracted_query {
            upsert_kv(&mut collection_query, entry);
        }

        vars.insert(
            0,
            VariableEntry {
                key: "base_url".to_owned(),
                value: url,
                enabled: true,
                secret: false,
                description: "Server base URL from OpenAPI specification".to_owned(),
            },
        );
        environments.push(ImportedEnvironment {
            name: env_name,
            variables: vars,
        });
    }

    Ok((
        "{{base_url}}".to_owned(),
        collection_query,
        Vec::new(),
        environments,
    ))
}

#[derive(Default)]
struct ParsedSecurity {
    /// Map from scheme name to its `AuthConfig`, or `None` when the scheme is
    /// cookie-based or otherwise unsupported for automatic header attachment.
    schemes: BTreeMap<String, Option<AuthConfig>>,
    variables: Vec<VariableEntry>,
    labels: Vec<String>,
}

fn allocate_bearer_var(
    vars: &mut Vec<VariableEntry>,
    scheme_name: &str,
    description: &str,
) -> String {
    let key = if vars.iter().any(|existing| existing.key == "bearer_token") {
        let base = sanitize_var_key(scheme_name, "bearer");
        if base.ends_with("_token") {
            base
        } else {
            format!("{base}_token")
        }
    } else {
        "bearer_token".to_owned()
    };
    push_secret_placeholder(vars, &key, description);
    key
}

fn allocate_basic_vars(vars: &mut Vec<VariableEntry>, scheme_name: &str) -> (String, String) {
    let (user_key, pass_key) = if vars.iter().any(|existing| existing.key == "basic_username") {
        let base = sanitize_var_key(scheme_name, "basic");
        (format!("{base}_username"), format!("{base}_password"))
    } else {
        ("basic_username".to_owned(), "basic_password".to_owned())
    };
    push_plain_placeholder(vars, &user_key, "Basic auth username");
    push_secret_placeholder(vars, &pass_key, "Basic auth password");
    (user_key, pass_key)
}

fn extract_v2_security(defs: Option<&Map<String, Value>>) -> ParsedSecurity {
    let Some(defs) = defs else {
        return ParsedSecurity::default();
    };
    let mut parsed = ParsedSecurity::default();

    for (scheme_name, def) in defs {
        let Some(kind) = def.get("type").and_then(Value::as_str) else {
            continue;
        };

        match kind {
            "apiKey" => {
                let param_name = def
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("X-API-Key")
                    .to_owned();
                let location = if def.get("in").and_then(Value::as_str) == Some("query") {
                    ApiKeyLocation::Query
                } else {
                    ApiKeyLocation::Header
                };
                parsed
                    .labels
                    .push(format!("{scheme_name} (API key: {param_name})"));
                let var_key = sanitize_var_key(scheme_name, "api_key");
                push_secret_placeholder(
                    &mut parsed.variables,
                    &var_key,
                    "API key from OpenAPI securityDefinitions",
                );
                parsed.schemes.insert(
                    scheme_name.clone(),
                    Some(AuthConfig::ApiKey {
                        name: param_name,
                        value: format!("{{{{{var_key}}}}}"),
                        location,
                    }),
                );
            }
            "basic" => {
                parsed.labels.push(format!("{scheme_name} (Basic)"));
                let (user_key, pass_key) = allocate_basic_vars(&mut parsed.variables, scheme_name);
                parsed.schemes.insert(
                    scheme_name.clone(),
                    Some(AuthConfig::Basic {
                        username: format!("{{{{{user_key}}}}}"),
                        password: format!("{{{{{pass_key}}}}}"),
                    }),
                );
            }
            "oauth2" => {
                parsed.labels.push(format!("{scheme_name} (OAuth2 Bearer)"));
                let token_key =
                    allocate_bearer_var(&mut parsed.variables, scheme_name, "OAuth2 bearer token");
                parsed.schemes.insert(
                    scheme_name.clone(),
                    Some(AuthConfig::Bearer {
                        token: format!("{{{{{token_key}}}}}"),
                    }),
                );
            }
            _other => {
                parsed.schemes.insert(scheme_name.clone(), None);
            }
        }
    }

    parsed
}

#[expect(
    clippy::too_many_lines,
    reason = "OpenAPI 3 security schemes cover http bearer/basic, apiKey header/query/cookie, oauth2 and openIdConnect"
)]
fn extract_v3_security(
    root: &Value,
    schemes: Option<&Map<String, Value>>,
    warnings: &mut Vec<OpenApiWarning>,
) -> ParsedSecurity {
    let Some(schemes) = schemes else {
        return ParsedSecurity::default();
    };
    let mut parsed = ParsedSecurity::default();

    for (scheme_name, raw_def) in schemes {
        let def = resolve_ref(root, raw_def, warnings);
        let Some(kind) = def.get("type").and_then(Value::as_str) else {
            continue;
        };

        match kind {
            "http" => {
                let http_scheme = def
                    .get("scheme")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_ascii_lowercase();

                if http_scheme == "bearer" {
                    parsed.labels.push(format!("{scheme_name} (Bearer)"));
                    let token_key = allocate_bearer_var(
                        &mut parsed.variables,
                        scheme_name,
                        "Bearer access token",
                    );
                    parsed.schemes.insert(
                        scheme_name.clone(),
                        Some(AuthConfig::Bearer {
                            token: format!("{{{{{token_key}}}}}"),
                        }),
                    );
                } else if http_scheme == "basic" {
                    parsed.labels.push(format!("{scheme_name} (Basic)"));
                    let (user_key, pass_key) =
                        allocate_basic_vars(&mut parsed.variables, scheme_name);
                    parsed.schemes.insert(
                        scheme_name.clone(),
                        Some(AuthConfig::Basic {
                            username: format!("{{{{{user_key}}}}}"),
                            password: format!("{{{{{pass_key}}}}}"),
                        }),
                    );
                } else {
                    parsed.schemes.insert(scheme_name.clone(), None);
                }
            }
            "apiKey" => {
                let param_name = def
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("X-API-Key")
                    .to_owned();
                let loc = def.get("in").and_then(Value::as_str).unwrap_or("header");

                if loc == "cookie" {
                    parsed
                        .labels
                        .push(format!("{scheme_name} (Cookie: {param_name})"));
                    push_unique_warning(
                        warnings,
                        OpenApiWarning::CookieSecurityScheme {
                            scheme: scheme_name.clone(),
                            cookie_name: param_name,
                        },
                    );
                    parsed.schemes.insert(scheme_name.clone(), None);
                } else {
                    let location = if loc == "query" {
                        ApiKeyLocation::Query
                    } else {
                        ApiKeyLocation::Header
                    };
                    parsed
                        .labels
                        .push(format!("{scheme_name} (API key: {param_name})"));
                    let var_key = sanitize_var_key(scheme_name, "api_key");
                    push_secret_placeholder(
                        &mut parsed.variables,
                        &var_key,
                        "API key from OpenAPI securitySchemes",
                    );
                    parsed.schemes.insert(
                        scheme_name.clone(),
                        Some(AuthConfig::ApiKey {
                            name: param_name,
                            value: format!("{{{{{var_key}}}}}"),
                            location,
                        }),
                    );
                }
            }
            "oauth2" | "openIdConnect" => {
                parsed.labels.push(format!("{scheme_name} (Bearer token)"));
                let token_key =
                    allocate_bearer_var(&mut parsed.variables, scheme_name, "Access token");
                parsed.schemes.insert(
                    scheme_name.clone(),
                    Some(AuthConfig::Bearer {
                        token: format!("{{{{{token_key}}}}}"),
                    }),
                );
            }
            _other => {
                parsed.schemes.insert(scheme_name.clone(), None);
            }
        }
    }

    parsed
}

fn resolve_security_list(
    requirements: &[Value],
    parsed: &ParsedSecurity,
    preferred: Option<&AuthConfig>,
    context_label: &str,
    warnings: &mut Vec<OpenApiWarning>,
) -> Option<AuthConfig> {
    let mut first_supported: Option<AuthConfig> = None;

    for req_val in requirements {
        let Some(req_obj) = req_val.as_object() else {
            continue;
        };

        if req_obj.is_empty() {
            continue;
        }

        if req_obj.len() > 1 {
            push_unique_warning(
                warnings,
                OpenApiWarning::CombinedSecuritySchemes {
                    context: context_label.to_owned(),
                },
            );
        }

        for scheme_name in req_obj.keys() {
            if let Some(Some(auth)) = parsed.schemes.get(scheme_name) {
                if preferred.is_some_and(|pref| pref == auth) {
                    return Some(auth.clone());
                }

                if first_supported.is_none() {
                    first_supported = Some(auth.clone());
                }

                break;
            }
        }
    }

    first_supported
}

fn resolve_root_security(
    root_security: Option<&Value>,
    parsed: &ParsedSecurity,
    warnings: &mut Vec<OpenApiWarning>,
) -> AuthConfig {
    let Some(requirements) = root_security.and_then(Value::as_array) else {
        return AuthConfig::None;
    };

    if requirements.is_empty() {
        return AuthConfig::None;
    }

    resolve_security_list(requirements, parsed, None, "Root security", warnings)
        .unwrap_or(AuthConfig::None)
}

fn resolve_operation_security(
    op_security: Option<&Value>,
    parsed: &ParsedSecurity,
    root_auth: &AuthConfig,
    operation_name: &str,
    warnings: &mut Vec<OpenApiWarning>,
) -> AuthConfig {
    let Some(val) = op_security else {
        return if matches!(root_auth, AuthConfig::None) {
            AuthConfig::None
        } else {
            AuthConfig::Inherit
        };
    };

    let Some(requirements) = val.as_array() else {
        return AuthConfig::None;
    };

    // An explicit empty `security: []` disables root authentication for this
    // operation (OpenAPI Operation Object).
    if requirements.is_empty() {
        return AuthConfig::None;
    }

    let preferred = (!matches!(root_auth, AuthConfig::None)).then_some(root_auth);
    let label = format!("Endpoint \"{operation_name}\"");

    if let Some(resolved) = resolve_security_list(requirements, parsed, preferred, &label, warnings)
    {
        if preferred.is_some_and(|pref| *pref == resolved) {
            AuthConfig::Inherit
        } else {
            resolved
        }
    } else {
        let allows_anonymous = requirements
            .iter()
            .any(|item| item.as_object().is_some_and(Map::is_empty));

        if !allows_anonymous {
            push_unique_warning(
                warnings,
                OpenApiWarning::UnsupportedOperationSecurity {
                    operation: operation_name.to_owned(),
                },
            );
        }

        AuthConfig::None
    }
}

fn sanitize_var_key(raw: &str, fallback: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('_');

    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn push_secret_placeholder(vars: &mut Vec<VariableEntry>, key: &str, description: &str) {
    if !vars.iter().any(|existing| existing.key == key) {
        vars.push(VariableEntry {
            key: key.to_owned(),
            value: String::new(),
            enabled: true,
            secret: true,
            description: description.to_owned(),
        });
    }
}

fn push_plain_placeholder(vars: &mut Vec<VariableEntry>, key: &str, description: &str) {
    if !vars.iter().any(|existing| existing.key == key) {
        vars.push(VariableEntry {
            key: key.to_owned(),
            value: String::new(),
            enabled: true,
            secret: false,
            description: description.to_owned(),
        });
    }
}

fn first_tag(op_obj: &Map<String, Value>) -> Option<String> {
    let raw = op_obj
        .get("tags")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(Value::as_str)?;
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(80)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn operation_name(op_obj: &Map<String, Value>, method: HttpMethod, path: &str) -> String {
    let candidate = op_obj
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .or_else(|| {
            op_obj
                .get("operationId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
        });
    let fallback = format!("{} {path}", method.as_str());
    let chosen = candidate.unwrap_or(&fallback);
    let cleaned: String = chosen
        .chars()
        .filter(|ch| !ch.is_control())
        .take(120)
        .collect();

    if cleaned.is_empty() {
        fallback
    } else {
        cleaned
    }
}

fn operation_description(op_obj: &Map<String, Value>) -> String {
    op_obj
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .chars()
        .filter(|ch| *ch != '\0')
        .take(1_500)
        .collect()
}

fn upsert_kv(list: &mut Vec<KeyValueEntry>, entry: KeyValueEntry) {
    if let Some(existing) = list.iter_mut().find(|item| item.key == entry.key) {
        *existing = entry;
    } else {
        list.push(entry);
    }
}

fn scalar_example(obj: &Map<String, Value>) -> String {
    for key in ["example", "default", "x-example"] {
        if let Some(value) = obj.get(key) {
            return scalar_value_to_string(value);
        }
    }
    String::new()
}

fn v3_param_example(
    root: &Value,
    p_obj: &Map<String, Value>,
    warnings: &mut Vec<OpenApiWarning>,
) -> String {
    if let Some(value) = p_obj.get("example") {
        return scalar_value_to_string(value);
    }
    if let Some(schema) = p_obj.get("schema") {
        let resolved = resolve_ref(root, schema, warnings);
        if let Some(s_obj) = resolved.as_object() {
            let ex = scalar_example(s_obj);
            if !ex.is_empty() {
                return ex;
            }
            if let Some(first) = s_obj
                .get("examples")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
            {
                return scalar_value_to_string(first);
            }
            if let Some(first) = s_obj
                .get("enum")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
            {
                return scalar_value_to_string(first);
            }
        }
    }
    String::new()
}

fn scalar_value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(num) => num.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _other => String::new(),
    }
}

fn extract_v3_body(
    root: &Value,
    op_obj: &Map<String, Value>,
    warnings: &mut Vec<OpenApiWarning>,
) -> BodyConfig {
    let Some(raw_body) = op_obj.get("requestBody") else {
        return BodyConfig::None;
    };
    let body_obj = resolve_ref(root, raw_body, warnings);
    let Some(content) = body_obj.get("content").and_then(Value::as_object) else {
        return BodyConfig::None;
    };

    if let Some((_, media)) = content
        .iter()
        .find(|(mt, _)| mt.to_ascii_lowercase().contains("json"))
    {
        let example = media_example(root, media, warnings);
        let content_text =
            serde_json::to_string_pretty(&example).unwrap_or_else(|_| "{}".to_owned());
        return BodyConfig::Json {
            content: content_text,
        };
    }

    if let Some(media) = content.get("application/x-www-form-urlencoded") {
        let mut entries = Vec::new();
        if let Some(schema) = media.get("schema") {
            let resolved = resolve_ref(root, schema, warnings);
            if let Some(props) = resolved.get("properties").and_then(Value::as_object) {
                for (key, prop) in props {
                    let prop_resolved = resolve_ref(root, prop, warnings);
                    let value = prop_resolved
                        .as_object()
                        .map(scalar_example)
                        .unwrap_or_default();
                    let desc = prop_resolved
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_owned();
                    entries.push(KeyValueEntry {
                        key: key.clone(),
                        value,
                        enabled: true,
                        secret: is_secret_name(key),
                        description: desc,
                    });
                }
            }
        }
        return BodyConfig::FormUrlEncoded { entries };
    }

    if let Some((media_type, media)) = content.iter().next() {
        let example = media_example(root, media, warnings);
        let text = match example {
            Value::String(s) => s,
            other => serde_json::to_string_pretty(&other).unwrap_or_default(),
        };
        return BodyConfig::Text {
            content: text,
            content_type: media_type.clone(),
        };
    }

    BodyConfig::None
}

fn media_example(root: &Value, media: &Value, warnings: &mut Vec<OpenApiWarning>) -> Value {
    if let Some(example) = media.get("example") {
        return example.clone();
    }
    if let Some(examples) = media.get("examples").and_then(Value::as_object)
        && let Some(first) = examples.values().next()
    {
        let resolved = resolve_ref(root, first, warnings);
        if let Some(val) = resolved.get("value") {
            return val.clone();
        }
    }
    if let Some(schema) = media.get("schema") {
        return synthesize_example(root, schema, 0, warnings);
    }
    Value::Object(Map::new())
}

fn synthesize_example(
    root: &Value,
    schema: &Value,
    depth: usize,
    warnings: &mut Vec<OpenApiWarning>,
) -> Value {
    if depth >= MAX_SCHEMA_DEPTH {
        return Value::Null;
    }

    let resolved = resolve_ref(root, schema, warnings);
    let Some(obj) = resolved.as_object() else {
        return Value::Null;
    };

    if let Some(example) = obj.get("example") {
        return example.clone();
    }
    if let Some(default) = obj.get("default") {
        return default.clone();
    }
    if let Some(first_example) = obj
        .get("examples")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
    {
        return first_example.clone();
    }
    if let Some(first_enum) = obj
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
    {
        return first_enum.clone();
    }

    // Handle `allOf` by merging object properties.
    if let Some(all_of) = obj.get("allOf").and_then(Value::as_array) {
        let mut merged = Map::new();

        for part in all_of {
            if let Value::Object(part_map) = synthesize_example(root, part, depth + 1, warnings) {
                for (k, v) in part_map {
                    merged.insert(k, v);
                }
            }
        }
        return Value::Object(merged);
    }

    // Handle `oneOf` / `anyOf` by picking the first branch.
    for combinator in ["oneOf", "anyOf"] {
        if let Some(first) = obj
            .get(combinator)
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
        {
            return synthesize_example(root, first, depth + 1, warnings);
        }
    }

    let kind = schema_primary_type(obj);

    match kind {
        Some("object") => synthesize_object_properties(root, obj, depth, warnings),
        Some("array") => {
            let item = obj.get("items").map_or(Value::Null, |items| {
                synthesize_example(root, items, depth + 1, warnings)
            });
            Value::Array(vec![item])
        }
        Some("integer") => Value::Number(Number::from(0)),
        Some("number") => {
            Number::from_f64(0.0).map_or(Value::Number(Number::from(0)), Value::Number)
        }
        Some("boolean") => Value::Bool(false),
        Some("string") => {
            let sample = match obj.get("format").and_then(Value::as_str) {
                Some("email") => "user@example.com",
                Some("date") => "2026-01-01",
                Some("date-time") => "2026-01-01T00:00:00Z",
                Some("uuid") => "00000000-0000-0000-0000-000000000000",
                Some("uri" | "url") => "https://example.com",
                _other => "",
            };
            Value::String(sample.to_owned())
        }
        _other if obj.contains_key("properties") => {
            synthesize_object_properties(root, obj, depth, warnings)
        }
        _other => Value::Null,
    }
}

fn synthesize_object_properties(
    root: &Value,
    obj: &Map<String, Value>,
    depth: usize,
    warnings: &mut Vec<OpenApiWarning>,
) -> Value {
    let mut result = Map::new();

    if let Some(props) = obj.get("properties").and_then(Value::as_object) {
        let ordered: BTreeMap<&String, &Value> = props.iter().collect();

        for (k, prop_schema) in ordered {
            result.insert(
                k.clone(),
                synthesize_example(root, prop_schema, depth + 1, warnings),
            );
        }
    }
    Value::Object(result)
}

/// Supports both `OpenAPI` 3.0 (`type: "string"`) and `OpenAPI` 3.1 JSON Schema
/// arrays (`type: ["string", "null"]`).
fn schema_primary_type(obj: &Map<String, Value>) -> Option<&str> {
    match obj.get("type") {
        Some(Value::String(kind)) => Some(kind.as_str()),
        Some(Value::Array(kinds)) => kinds
            .iter()
            .filter_map(Value::as_str)
            .find(|kind| *kind != "null"),
        _other => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{AuthConfig, BodyConfig, HttpMethod, NetworkError};

    use super::parse_openapi;

    #[test]
    fn imports_swagger_v2_json_with_refs_and_security() {
        let spec = r##"{
          "swagger": "2.0",
          "info": { "title": "Pet Store", "description": "Pets API" },
          "host": "pets.example.com",
          "basePath": "/v2",
          "schemes": ["https"],
          "securityDefinitions": {
            "api_key": { "type": "apiKey", "name": "X-API-Key", "in": "header" }
          },
          "security": [{ "api_key": [] }],
          "definitions": {
            "Pet": {
              "type": "object",
              "properties": {
                "name": { "type": "string", "example": "Milo" }
              }
            }
          },
          "paths": {
            "/pets/{id}": {
              "parameters": [
                { "name": "id", "in": "path", "required": true, "type": "string", "example": "pet-1" }
              ],
              "get": {
                "tags": ["Pets"],
                "summary": "Get pet",
                "parameters": [
                  { "name": "expand", "in": "query", "type": "boolean", "default": true }
                ],
                "responses": { "200": { "description": "ok" } }
              },
              "post": {
                "tags": ["Pets"],
                "summary": "Update pet",
                "parameters": [
                  { "name": "body", "in": "body", "schema": { "$ref": "#/definitions/Pet" } }
                ],
                "responses": { "200": { "description": "ok" } }
              }
            }
          }
        }"##;

        let imported = parse_openapi(spec).unwrap();
        assert_eq!(imported.title, "Pet Store");
        assert_eq!(imported.spec_version, "2.0");
        assert_eq!(imported.base_url, "https://pets.example.com/v2");
        assert_eq!(imported.folders, vec!["Pets"]);
        assert_eq!(imported.requests.len(), 2);
        assert!(matches!(imported.auth, AuthConfig::ApiKey { .. }));
        assert!(
            imported
                .variables
                .iter()
                .any(|v| v.key == "api_key" && v.secret)
        );

        let post = imported
            .requests
            .iter()
            .find(|r| r.method == HttpMethod::Post)
            .unwrap();
        assert!(matches!(&post.body, BodyConfig::Json { content } if content.contains("Milo")));
    }

    #[test]
    fn imports_openapi_v3_1_yaml_with_multiple_servers_and_nullable_types() {
        let yaml = r##"
openapi: 3.1.0
info:
  title: Orders API
  description: Manage customer orders
servers:
  - url: https://staging.orders.example.com/api
    description: Staging
  - url: https://orders.example.com/api
    description: Production
security:
  - bearerAuth: []
components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
  schemas:
    OrderInput:
      type: object
      properties:
        customerEmail:
          type: ["string", "null"]
          format: email
        quantity:
          type: integer
          default: 2
paths:
  /orders:
    post:
      tags: [Orders]
      summary: Create order
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/OrderInput"
      responses:
        "201":
          description: Created
"##;

        let imported = parse_openapi(yaml).unwrap();
        assert_eq!(imported.title, "Orders API");
        assert_eq!(imported.spec_version, "3.1.0");
        assert_eq!(imported.base_url, "{{base_url}}");
        assert_eq!(imported.environments.len(), 2);
        assert_eq!(imported.environments[0].name, "Staging");
        assert_eq!(imported.environments[1].name, "Production");
        assert!(matches!(imported.auth, AuthConfig::Bearer { .. }));
        assert_eq!(imported.requests.len(), 1);
        assert!(matches!(
            &imported.requests[0].body,
            BodyConfig::Json { content }
                if content.contains("user@example.com") && content.contains("\"quantity\": 2")
        ));
    }

    #[test]
    fn rejects_yaml_anchor_bombs_and_guards_cyclic_and_remote_refs() {
        let bomb = "\
a: &a [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
b: &b [*a, *a, *a, *a, *a, *a, *a, *a, *a, *a]
c: &c [*b, *b, *b, *b, *b, *b, *b, *b, *b, *b]
d: &d [*c, *c, *c, *c, *c, *c, *c, *c, *c, *c]
e: &e [*d, *d, *d, *d, *d, *d, *d, *d, *d, *d]
f: &f [*e, *e, *e, *e, *e, *e, *e, *e, *e, *e]
";
        assert_eq!(
            parse_openapi(bomb).unwrap_err(),
            NetworkError::SpecTooComplex
        );

        let cyclic_and_remote = r##"{
          "openapi": "3.0.3",
          "info": { "title": "Edge Cases" },
          "servers": [{ "url": "https://api.example.com" }],
          "components": {
            "schemas": {
              "Loop": { "$ref": "#/components/schemas/Loop" }
            }
          },
          "paths": {
            "/loop": {
              "post": {
                "summary": "Loop",
                "requestBody": {
                  "content": {
                    "application/json": {
                      "schema": { "$ref": "#/components/schemas/Loop" }
                    }
                  }
                }
              }
            },
            "/remote": {
              "get": {
                "summary": "Remote",
                "parameters": [{ "$ref": "https://evil.example.com/param.json" }]
              }
            }
          }
        }"##;
        let imported = parse_openapi(cyclic_and_remote).unwrap();
        assert_eq!(imported.requests.len(), 2);
        assert!(imported.warnings.iter().any(|w| w.contains("cyclic")));
        assert!(
            imported
                .warnings
                .iter()
                .any(|w| w.contains("external $ref"))
        );
    }

    #[test]
    fn rejects_large_aliased_scalar_yaml_before_memory_blowup() {
        let payload = "a".repeat(256 * 1024);
        let mut yaml = format!("big: &big \"{payload}\"\nrefs:\n");

        for _ in 0..200 {
            yaml.push_str("  - *big\n");
        }

        assert_eq!(
            parse_openapi(&yaml).unwrap_err(),
            NetworkError::SpecTooComplex
        );
    }

    #[test]
    fn honors_root_and_per_operation_security_in_v2_and_v3() {
        let v3_spec = r#"{
          "openapi": "3.1.0",
          "info": { "title": "Auth Matrix" },
          "servers": [{ "url": "https://api.example.com" }],
          "security": [{ "bearerAuth": [] }],
          "components": {
            "securitySchemes": {
              "bearerAuth": { "type": "http", "scheme": "bearer" },
              "adminKey": { "type": "apiKey", "name": "X-Admin-Key", "in": "header" }
            }
          },
          "paths": {
            "/health": {
              "get": {
                "summary": "Public health",
                "security": []
              }
            },
            "/profile": {
              "get": {
                "summary": "Default bearer"
              }
            },
            "/admin": {
              "post": {
                "summary": "Admin action",
                "security": [{ "adminKey": [] }]
              }
            }
          }
        }"#;
        let imported = parse_openapi(v3_spec).unwrap();
        assert!(matches!(imported.auth, AuthConfig::Bearer { .. }));

        let health = imported
            .requests
            .iter()
            .find(|r| r.name == "Public health")
            .unwrap();
        assert_eq!(health.auth, AuthConfig::None);

        let profile = imported
            .requests
            .iter()
            .find(|r| r.name == "Default bearer")
            .unwrap();
        assert_eq!(profile.auth, AuthConfig::Inherit);

        let admin = imported
            .requests
            .iter()
            .find(|r| r.name == "Admin action")
            .unwrap();
        assert!(matches!(
            &admin.auth,
            AuthConfig::ApiKey { name, .. } if name == "X-Admin-Key"
        ));

        // When root security is omitted, definitions alone never attach auth to
        // operations that do not opt in.
        let v2_no_root = r#"{
          "swagger": "2.0",
          "info": { "title": "Opt-In V2" },
          "host": "api.example.com",
          "securityDefinitions": {
            "api_key": { "type": "apiKey", "name": "X-Key", "in": "header" }
          },
          "paths": {
            "/public": {
              "get": { "summary": "Public" }
            },
            "/private": {
              "get": {
                "summary": "Private",
                "security": [{ "api_key": [] }]
              }
            }
          }
        }"#;
        let v2 = parse_openapi(v2_no_root).unwrap();
        assert_eq!(v2.auth, AuthConfig::None);
        assert_eq!(
            v2.requests
                .iter()
                .find(|r| r.name == "Public")
                .unwrap()
                .auth,
            AuthConfig::None
        );
        assert!(matches!(
            v2.requests
                .iter()
                .find(|r| r.name == "Private")
                .unwrap()
                .auth,
            AuthConfig::ApiKey { .. }
        ));
    }

    #[test]
    fn emits_structured_missing_base_url_warning_with_empty_base_url() {
        use crate::model::{MissingBaseUrlSource, OpenApiWarning};

        let v3_no_servers = r#"{
          "openapi": "3.0.3",
          "info": { "title": "Serverless Spec" },
          "paths": {
            "/ping": { "get": { "summary": "Ping" } }
          }
        }"#;
        let v3 = parse_openapi(v3_no_servers).unwrap();
        assert_eq!(v3.base_url, "");
        assert!(v3.warning_items.contains(&OpenApiWarning::MissingBaseUrl {
            source: MissingBaseUrlSource::V3Servers,
        }));

        let v2_no_host = r#"{
          "swagger": "2.0",
          "info": { "title": "Hostless Spec" },
          "paths": {
            "/ping": { "get": { "summary": "Ping" } }
          }
        }"#;
        let v2 = parse_openapi(v2_no_host).unwrap();
        assert_eq!(v2.base_url, "");
        assert!(v2.warning_items.contains(&OpenApiWarning::MissingBaseUrl {
            source: MissingBaseUrlSource::V2Host,
        }));
    }

    #[test]
    fn yaml_counts_value_nodes_consistently_with_json_for_large_mapping_specs() {
        use std::fmt::Write as _;

        use super::{YamlBuilder, check_value_bounds};
        use saphyr_parser::Parser;

        // Build an OpenAPI 3.0 YAML spec whose total YAML events (containers +
        // mapping keys + values) exceed MAX_YAML_NODES (200,000), while its
        // JSON value node count stays within the 200,000-node budget, matching
        // large real-world specs such as GitLab's `openapi_v3.yaml`.
        let mut yaml = String::from(
            "openapi: 3.0.0\ninfo:\n  title: Large Mapping API\nservers:\n  - url: https://api.example.com\npaths:\n",
        );

        for index in 0..1_700 {
            writeln!(yaml, "  /api/v4/items/{index}:").unwrap();
            yaml.push_str("    get:\n");
            writeln!(yaml, "      summary: Get item {index}").unwrap();
            yaml.push_str("      tags:\n        - Items\n");
            yaml.push_str("      parameters:\n");

            for param in 0..9 {
                writeln!(yaml, "        - name: p_{param}").unwrap();
                yaml.push_str("          in: query\n");
                yaml.push_str("          required: false\n");
                yaml.push_str("          description: Filter parameter\n");
                yaml.push_str("          schema:\n");
                yaml.push_str("            type: string\n");
                yaml.push_str("            example: sample\n");
            }
        }

        let mut builder = YamlBuilder::default();
        let mut yaml_node_events = 0usize;

        for next in Parser::new_from_str(&yaml) {
            let (event, _span) = next.unwrap();

            if matches!(
                event,
                saphyr_parser::Event::MappingStart(..)
                    | saphyr_parser::Event::SequenceStart(..)
                    | saphyr_parser::Event::Scalar(..)
            ) {
                yaml_node_events += 1;
            }

            if builder.apply_event(event).unwrap() {
                break;
            }
        }

        let value = builder.root.unwrap();
        let mut json_nodes = 0usize;
        let mut json_bytes = 0usize;
        check_value_bounds(&value, 0, &mut json_nodes, &mut json_bytes).unwrap();

        assert_eq!(builder.nodes, json_nodes);
        assert_eq!(builder.expanded_bytes, json_bytes);
        assert!(yaml_node_events > super::MAX_YAML_NODES);
        assert!(json_nodes > 120_000 && json_nodes <= super::MAX_YAML_NODES);

        let imported = parse_openapi(&yaml).unwrap();
        assert_eq!(imported.title, "Large Mapping API");
        assert_eq!(imported.requests.len(), 1_700);
    }

    #[test]
    fn rejects_excessive_yaml_value_nodes_large_aliased_keys_and_non_scalar_keys() {
        use std::fmt::Write as _;

        // Exceeding MAX_YAML_NODES in actual value nodes still fails.
        let mut too_many_nodes = String::from("items:\n");

        for index in 0..=super::MAX_YAML_NODES {
            writeln!(too_many_nodes, "  - {index}").unwrap();
        }

        assert_eq!(
            parse_openapi(&too_many_nodes).unwrap_err(),
            NetworkError::SpecTooComplex
        );

        // Large aliased scalar keys still hit MAX_YAML_EXPANDED_BYTES before
        // unbounded allocation.
        let big_key = "k".repeat(256 * 1024);
        let mut aliased_keys = format!("seed: &big \"{big_key}\"\nentries:\n");

        for index in 0..200 {
            writeln!(aliased_keys, "  - *big : {index}").unwrap();
        }

        assert_eq!(
            parse_openapi(&aliased_keys).unwrap_err(),
            NetworkError::SpecTooComplex
        );

        // Non-scalar YAML mapping keys (literal or aliased) are refused.
        let sequence_key = "?\n  - a\n  - b\n: 1\n";
        assert!(matches!(
            parse_openapi(sequence_key),
            Err(NetworkError::InvalidSpec { .. })
        ));

        let aliased_seq_key = "arr: &arr [1, 2]\nmap:\n  *arr : 1\n";
        assert!(matches!(
            parse_openapi(aliased_seq_key),
            Err(NetworkError::InvalidSpec { .. })
        ));

        // Anchored and aliased scalar mapping keys remain supported.
        let anchored_scalar_key = "\
openapi: 3.0.3
info:
  &title_key title: Anchored Key API
servers:
  - url: https://api.example.com
paths:
  /ping:
    get:
      summary: Ping
x-copy:
  *title_key : mirrored
";
        let imported = parse_openapi(anchored_scalar_key).unwrap();
        assert_eq!(imported.title, "Anchored Key API");
        assert_eq!(imported.requests.len(), 1);
    }
}
