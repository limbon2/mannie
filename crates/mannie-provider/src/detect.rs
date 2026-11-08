//! Recognizing the server behind a base URL, and reading its list of models
//! (docs/features/chat/providers.md, quirk profiles).
//!
//! OpenAI and OpenRouter are recognized by their hosts. The servers that the
//! owner runs are recognized by endpoints that only one of them has. A probe
//! is trusted only once a recorded fixture shows it; until then an unknown
//! answer means an unknown server, which gets the generic behavior.

use std::time::Duration;

use futures_util::StreamExt;
use mannie_types::provider::{ReportedLimits, ServerKind};
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;
use reqwest::Url;
use serde_json::Value;

use crate::errors::{self, Context, ErrorDetails};
use crate::provider::ProviderError;
use crate::quirks::chatgpt_plan_quirks;
use crate::stream::{broken_stream, read_error_body, timed_out, transport_failure};
use crate::transport::{HttpClient, MAX_ERROR_BODY_BYTES, endpoint};

/// How long one request of the test may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// The most bytes of a model list that Mannie reads. OpenRouter lists
/// hundreds of models with long descriptions.
const MAX_LIST_BYTES: usize = 16 * 1024 * 1024;

/// A model as the server lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedModel {
    /// The model's name.
    pub id: String,
    /// The name that the owner reads, when the list has one, as the ChatGPT
    /// plan's catalog does.
    pub display_name: Option<String>,
    /// Its limits, when the list says.
    pub limits: Option<ReportedLimits>,
}

/// What asking for the list of models found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelList {
    /// The server listed its models.
    Listed(Vec<ListedModel>),
    /// The server has no list of models, but answered.
    Unavailable,
    /// The server could not be reached, or refused the credential.
    Failed(RunFailure),
}

/// The server that a host belongs to, for the hosts Mannie knows.
pub fn server_from_host(url: &Url) -> Option<ServerKind> {
    let host = url.host_str()?.to_ascii_lowercase();
    let is_openai = host == "api.openai.com";
    let is_openrouter = host == "openrouter.ai" || host.ends_with(".openrouter.ai");
    if is_openai {
        Some(ServerKind::OpenAi)
    } else if is_openrouter {
        Some(ServerKind::OpenRouter)
    } else {
        None
    }
}

/// An endpoint that only one kind of server has.
struct Probe {
    kind: ServerKind,
    path: &'static [&'static str],
    recognizes: fn(&Value) -> bool,
}

/// The probes, in the order they are tried.
const PROBES: [Probe; 4] = [
    Probe {
        kind: ServerKind::Ollama,
        path: &["api", "version"],
        recognizes: |body| body.get("version").is_some(),
    },
    Probe {
        kind: ServerKind::LmStudio,
        path: &["api", "v0", "models"],
        recognizes: |body| {
            body.get("data")
                .and_then(Value::as_array)
                .is_some_and(|models| models.iter().any(|model| model.get("state").is_some()))
        },
    },
    Probe {
        kind: ServerKind::Sglang,
        path: &["get_server_info"],
        recognizes: Value::is_object,
    },
    Probe {
        kind: ServerKind::Vllm,
        path: &["version"],
        recognizes: |body| body.get("version").is_some(),
    },
];

/// Recognizes the server: by its host, or by the endpoints only one server
/// has, asked without the credential.
pub async fn detect_server(client: &HttpClient, base_url: &Url) -> ServerKind {
    if let Some(kind) = server_from_host(base_url) {
        return kind;
    }
    let root = server_root(base_url);
    for probe in PROBES {
        let answer = get_json(
            client,
            endpoint(&root, probe.path),
            None,
            MAX_ERROR_BODY_BYTES,
        )
        .await;
        if let Ok(Some(body)) = answer
            && (probe.recognizes)(&body)
        {
            return probe.kind;
        }
    }
    ServerKind::Other
}

/// Asks the server for its list of models.
pub async fn list_models(
    client: &HttpClient,
    base_url: &Url,
    credential: Option<&Secret>,
) -> ModelList {
    let url = endpoint(base_url, &["models"]);
    match get_json(client, url, credential, MAX_LIST_BYTES).await {
        Ok(Some(body)) => ModelList::Listed(listed_models(&body)),
        Ok(None) => ModelList::Unavailable,
        Err(failure) => ModelList::Failed(failure),
    }
}

/// Asks for the catalog of models that the ChatGPT plan offers the signed-in
/// account (ADR-0062, decision 11): `{"models": [...]}`, of which the entries
/// with `visibility` `list` are offered, in the catalog's order. A refusal is
/// classified as a model call's would be, so that a refused token is
/// refreshed and a usage limit is shown.
pub async fn list_plan_models(
    client: &HttpClient,
    api_base: &Url,
    token: &Secret,
) -> Result<Vec<ListedModel>, ProviderError> {
    let url = endpoint(api_base, &["models"]);
    let response = tokio::time::timeout(PROBE_TIMEOUT, client.get(url, Some(token)).send())
        .await
        .map_err(|_elapsed| timed_out())?
        .map_err(|error| transport_failure(&error))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let headers = response.headers().clone();
        let body = read_error_body(response, PROBE_TIMEOUT).await;
        let quirks = chatgpt_plan_quirks();
        let context = Context {
            model: "",
            quirks: &quirks,
            now: client.clock().now(),
        };
        return Err(errors::classify_status(
            status,
            &headers,
            &ErrorDetails::from_body(&body),
            context,
        ));
    }
    // The catalog is read whole, under the limit of model lists: cut to the
    // size of an error body, a long catalog would not parse.
    let body = read_body(response, MAX_LIST_BYTES)
        .await
        .inspect_err(|error| {
            tracing::warn!(
                failure = ?error.failure(),
                "the ChatGPT plan's catalog could not be read"
            );
        })?;
    let catalog: Value = serde_json::from_slice(&body).map_err(|error| {
        // Where and how the catalog breaks, never its content, which
        // describes the owner's account.
        tracing::warn!(
            bytes = body.len(),
            line = error.line(),
            column = error.column(),
            kind = ?error.classify(),
            "the ChatGPT plan's catalog is not JSON"
        );
        ProviderError::Permanent(RunFailure::InvalidResponse)
    })?;
    Ok(plan_models(&catalog))
}

/// The models of the ChatGPT plan's catalog that it offers for a list.
fn plan_models(catalog: &Value) -> Vec<ListedModel> {
    let text = |item: &Value, key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    catalog
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("visibility").and_then(Value::as_str) == Some("list"))
        .filter_map(|item| {
            Some(ListedModel {
                id: text(item, "slug")?,
                display_name: text(item, "display_name"),
                limits: None,
            })
        })
        .collect()
}

/// The models in a list, in the shapes that the servers use: `{"data": [...]}`
/// or a bare array.
fn listed_models(body: &Value) -> Vec<ListedModel> {
    let items = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.as_array());
    items
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item.get("id").and_then(Value::as_str)?.to_owned();
            Some(ListedModel {
                id,
                display_name: None,
                limits: reported_limits(item),
            })
        })
        .collect()
}

/// The limits a server lists for a model: vLLM's `max_model_len`,
/// OpenRouter's `context_length` and `top_provider`, and the names that other
/// servers use.
fn reported_limits(item: &Value) -> Option<ReportedLimits> {
    let number = |pointer: &str| {
        item.pointer(pointer)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    };
    let context_window = [
        "/max_model_len",
        "/context_length",
        "/max_context_length",
        "/context_window",
    ]
    .into_iter()
    .find_map(number)?;
    let output_limit = [
        "/top_provider/max_completion_tokens",
        "/max_completion_tokens",
        "/max_output_tokens",
    ]
    .into_iter()
    .find_map(number);
    Some(ReportedLimits {
        context_window,
        output_limit,
    })
}

/// The server's root: the base URL without a final `v1`.
fn server_root(base_url: &Url) -> Url {
    let mut root = base_url.clone();
    if let Ok(mut segments) = root.path_segments_mut() {
        segments.pop_if_empty();
    }
    let ends_with_version = root
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .is_some_and(|last| last == "v1");
    if ends_with_version && let Ok(mut segments) = root.path_segments_mut() {
        segments.pop();
    }
    root
}

/// GETs a JSON document. `Ok(None)` means the server answered that it has no
/// such resource, or answered with something other than JSON.
async fn get_json(
    client: &HttpClient,
    url: Url,
    credential: Option<&Secret>,
    max_bytes: usize,
) -> Result<Option<Value>, RunFailure> {
    let response = tokio::time::timeout(PROBE_TIMEOUT, client.get(url, credential).send())
        .await
        .map_err(|_elapsed| RunFailure::TimedOut)?
        .map_err(|error| transport_failure(&error).failure())?;
    let status = response.status().as_u16();
    match status {
        200..=299 => {}
        401 => return Err(RunFailure::InvalidCredential),
        403 => return Err(RunFailure::PermissionDenied),
        500..=599 => return Err(RunFailure::ServerError { status }),
        _ => return Ok(None),
    }
    let body = read_body(response, max_bytes)
        .await
        .map_err(|error| error.failure())?;
    Ok(serde_json::from_slice(&body).ok())
}

/// Reads a whole body of at most `max_bytes`, within the time of one request
/// of the test. A larger body is refused, never cut.
async fn read_body(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, ProviderError> {
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    let reading = async {
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|_broken| broken_stream())?;
            if body.len() + chunk.len() > max_bytes {
                return Err(ProviderError::Permanent(RunFailure::InvalidResponse));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(())
    };
    tokio::time::timeout(PROBE_TIMEOUT, reading)
        .await
        .map_err(|_elapsed| timed_out())??;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use mannie_types::provider::{ReportedLimits, ServerKind};
    use reqwest::Url;
    use serde_json::json;

    use super::{listed_models, server_from_host, server_root};

    #[test]
    fn known_hosts_are_recognized() {
        let url = |text: &str| Url::parse(text).unwrap();

        assert_eq!(
            server_from_host(&url("https://api.openai.com/v1")),
            Some(ServerKind::OpenAi)
        );
        assert_eq!(
            server_from_host(&url("https://openrouter.ai/api/v1")),
            Some(ServerKind::OpenRouter)
        );
        assert_eq!(server_from_host(&url("http://127.0.0.1:8000/v1")), None);
    }

    #[test]
    fn the_root_drops_a_final_version_segment() {
        let root = |text: &str| server_root(&Url::parse(text).unwrap()).to_string();

        assert_eq!(root("http://127.0.0.1:11434/v1"), "http://127.0.0.1:11434/");
        assert_eq!(root("http://host/proxy/vllm/v1/"), "http://host/proxy/vllm");
        assert_eq!(root("http://host:1234"), "http://host:1234/");
    }

    #[test]
    fn model_lists_give_their_limits() {
        let vllm = json!({ "data": [{ "id": "qwen3", "max_model_len": 32_768 }] });
        let openrouter = json!({ "data": [{
            "id": "openai/gpt-5-mini",
            "context_length": 400_000,
            "top_provider": { "max_completion_tokens": 128_000 }
        }] });
        let bare = json!([{ "id": "plain" }]);

        assert_eq!(
            listed_models(&vllm)[0].limits,
            Some(ReportedLimits {
                context_window: 32_768,
                output_limit: None
            })
        );
        assert_eq!(
            listed_models(&openrouter)[0].limits,
            Some(ReportedLimits {
                context_window: 400_000,
                output_limit: Some(128_000)
            })
        );
        assert_eq!(listed_models(&bare)[0].id, "plain");
        assert_eq!(listed_models(&bare)[0].limits, None);
    }

    #[test]
    fn plan_catalogs_keep_listed_models_in_server_order() {
        let catalog = serde_json::json!({
            "models": [
                { "slug": "gpt-6.1-sol", "display_name": "GPT-6.1 Sol", "visibility": "list" },
                { "slug": "internal-eval", "display_name": "Eval", "visibility": "hide" },
                { "slug": "gpt-6-mini", "visibility": "list" },
                { "display_name": "No slug", "visibility": "list" },
                { "slug": "codex-6", "display_name": " ", "visibility": "list" }
            ]
        });

        let models = super::plan_models(&catalog);

        let names: Vec<(&str, Option<&str>)> = models
            .iter()
            .map(|model| (model.id.as_str(), model.display_name.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                ("gpt-6.1-sol", Some("GPT-6.1 Sol")),
                ("gpt-6-mini", None),
                ("codex-6", None)
            ]
        );
        assert_eq!(
            super::plan_models(&serde_json::json!({ "data": [] })),
            Vec::<super::ListedModel>::new()
        );
    }
}
