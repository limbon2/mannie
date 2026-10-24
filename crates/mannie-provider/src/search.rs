//! The adapters of search profiles (ADR-0047): embeddings through the
//! server's `/embeddings` in OpenAI's shape, and reranking through its
//! `/rerank` in the shape of Jina and Cohere, each with one request and one
//! response. They share the chat adapters' client, error classes and refusal
//! of redirects, and the test of a search profile checks a server as the
//! provider test checks a chat profile.

use std::time::Duration;

use futures_util::StreamExt;
use mannie_types::provider::{
    ApiInterface, CheckNote, CheckOutcome, RerankProbe, SearchCheck, SearchCheckKind, SearchKind,
    SearchTestReport, SkipReason,
};
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;
use reqwest::Url;
use serde_json::{Value, json};

use crate::detect::{self, ModelList};
use crate::errors::{self, Context, ErrorDetails};
use crate::provider::ProviderError;
use crate::quirks::quirks_for;
use crate::stream::{loggable, read_error_body, timed_out, transport_failure};
use crate::transport::{HttpClient, endpoint};

/// The most texts that one call embeds, or passages that one call reranks.
pub const MAX_SEARCH_INPUTS: usize = 64;

/// The most bytes of one text that a call takes.
pub const MAX_SEARCH_INPUT_BYTES: usize = 32 * 1024;

/// The most bytes of a response that Mannie reads: 64 vectors of 4,096
/// numbers as JSON fit with room to spare.
const MAX_SEARCH_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// How long a response may take to arrive in full.
const SEARCH_RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);

/// The text of the embed check.
const EMBED_PROBE: &str = "Mannie checks that this server embeds text.";

/// The question of the rerank check, a passage that answers it and one that
/// does not.
const RERANK_QUESTION: &str = "What color is the sky on a clear day?";
const RERANK_ANSWER: &str = "On a clear day, the sky is blue.";
const RERANK_UNRELATED: &str = "Bread needs flour, water, salt and yeast.";

/// Where a call of a search profile goes.
#[derive(Debug, Clone)]
pub struct SearchTarget {
    /// The server's base URL.
    pub base_url: Url,
    /// The model.
    pub model: String,
    /// The credential, when the profile has one.
    pub credential: Option<Secret>,
}

/// Embeds texts as they are, and returns one vector for each, in their
/// order, all of one dimension. The caller adds the profile's prefix.
pub async fn embed(
    client: &HttpClient,
    target: &SearchTarget,
    inputs: &[String],
) -> Result<Vec<Vec<f32>>, ProviderError> {
    check_inputs(inputs)?;
    let body = json!({
        "model": target.model,
        "input": inputs,
        "encoding_format": "float",
    });
    let answer = post(client, target, &["embeddings"], &body).await?;
    read_embeddings(&answer, inputs.len()).ok_or_else(invalid_response)
}

/// Scores passages against a query, and returns one score for each, in
/// their order, from 0 to 1.
pub async fn rerank(
    client: &HttpClient,
    target: &SearchTarget,
    query: &str,
    documents: &[String],
) -> Result<Vec<f32>, ProviderError> {
    check_inputs(documents)?;
    if query.len() > MAX_SEARCH_INPUT_BYTES {
        return Err(ProviderError::Permanent(RunFailure::BadRequest {
            status: 413,
        }));
    }
    let body = json!({
        "model": target.model,
        "query": query,
        "documents": documents,
        "top_n": documents.len(),
    });
    let answer = post(client, target, &["rerank"], &body).await?;
    read_scores(&answer, documents.len()).ok_or_else(invalid_response)
}

/// Refuses a call with no texts, too many or a text that is too long, which
/// the service checks before; a call never goes out with them.
fn check_inputs(inputs: &[String]) -> Result<(), ProviderError> {
    let count_is_valid = !inputs.is_empty() && inputs.len() <= MAX_SEARCH_INPUTS;
    let sizes_are_valid = inputs
        .iter()
        .all(|input| input.len() <= MAX_SEARCH_INPUT_BYTES);
    if count_is_valid && sizes_are_valid {
        Ok(())
    } else {
        Err(ProviderError::Permanent(RunFailure::BadRequest {
            status: 413,
        }))
    }
}

fn invalid_response() -> ProviderError {
    ProviderError::Permanent(RunFailure::InvalidResponse)
}

/// POSTs a JSON document and reads the JSON answer, classifying a refusal
/// as the chat adapters do.
async fn post(
    client: &HttpClient,
    target: &SearchTarget,
    path: &[&str],
    body: &Value,
) -> Result<Value, ProviderError> {
    let url = endpoint(&target.base_url, path);
    let bytes = serde_json::to_vec(body).map_err(|_unwritable| invalid_response())?;
    let request = client.post_json(url, target.credential.as_ref(), bytes);
    let response = tokio::time::timeout(client.timeouts().headers, request.send())
        .await
        .map_err(|_elapsed| timed_out())?
        .map_err(|error| transport_failure(&error))?;
    let status = response.status();
    if !status.is_success() {
        let headers = response.headers().clone();
        let error_body = read_error_body(response, client.timeouts().idle).await;
        let details = ErrorDetails::from_body(&error_body);
        let quirks = quirks_for(ApiInterface::ChatCompletions, None);
        let context = Context {
            model: &target.model,
            quirks: &quirks,
            now: client.clock().now(),
        };
        let error = errors::classify_status(status.as_u16(), &headers, &details, context);
        tracing::debug!(
            status = status.as_u16(),
            code = details.code.as_deref(),
            message = loggable(details.message.as_deref(), target.credential.as_ref()).as_deref(),
            "the search server refused the call"
        );
        return Err(error);
    }
    let mut answer = Vec::new();
    let mut chunks = response.bytes_stream();
    let reading = async {
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|_broken| ProviderError::Transient {
                cause: RunFailure::BrokenStream,
                retry_after: None,
            })?;
            if answer.len() + chunk.len() > MAX_SEARCH_RESPONSE_BYTES {
                return Err(invalid_response());
            }
            answer.extend_from_slice(&chunk);
        }
        Ok(())
    };
    tokio::time::timeout(SEARCH_RESPONSE_TIMEOUT, reading)
        .await
        .map_err(|_elapsed| timed_out())??;
    serde_json::from_slice(&answer).map_err(|_not_json| invalid_response())
}

/// The vectors of an answer in OpenAI's shape, `{"data": [{"index",
/// "embedding"}]}`, in the order of the inputs: one for each input, all of
/// one dimension, every number finite. An item without an index stands at
/// its place in the list.
pub(crate) fn read_embeddings(answer: &Value, inputs: usize) -> Option<Vec<Vec<f32>>> {
    let items = answer.get("data")?.as_array()?;
    if items.len() != inputs {
        return None;
    }
    let mut vectors: Vec<Option<Vec<f32>>> = vec![None; inputs];
    for (place, item) in items.iter().enumerate() {
        let index = match item.get("index") {
            Some(index) => usize::try_from(index.as_u64()?).ok()?,
            None => place,
        };
        let slot = vectors.get_mut(index)?;
        if slot.is_some() {
            return None;
        }
        *slot = Some(read_vector(item.get("embedding")?)?);
    }
    let vectors: Vec<Vec<f32>> = vectors.into_iter().collect::<Option<_>>()?;
    let dimensions = vectors.first()?.len();
    let same_dimension = vectors.iter().all(|vector| vector.len() == dimensions);
    (dimensions > 0 && same_dimension).then_some(vectors)
}

fn read_vector(value: &Value) -> Option<Vec<f32>> {
    value
        .as_array()?
        .iter()
        .map(|number| {
            let number = number.as_f64()?;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "embeddings are 32-bit floats, and servers send them as JSON numbers"
            )]
            let narrow = number as f32;
            narrow.is_finite().then_some(narrow)
        })
        .collect()
}

/// The scores of an answer, in the order of the documents: Jina's and
/// Cohere's `{"results": [{"index", "relevance_score"}]}`, or the bare array
/// of `{"index", "score"}` of Text Embeddings Inference. Every document must
/// have one. Scores outside 0 to 1 are logits, which some servers return as
/// they are, and a sigmoid turns them into probabilities.
pub(crate) fn read_scores(answer: &Value, documents: usize) -> Option<Vec<f32>> {
    let items = match answer {
        Value::Array(items) => items,
        _ => answer.get("results")?.as_array()?,
    };
    let mut scores: Vec<Option<f64>> = vec![None; documents];
    for item in items {
        let index = usize::try_from(item.get("index")?.as_u64()?).ok()?;
        let score = item
            .get("relevance_score")
            .or_else(|| item.get("score"))?
            .as_f64()?;
        if !score.is_finite() {
            return None;
        }
        let slot = scores.get_mut(index)?;
        if slot.is_some() {
            return None;
        }
        *slot = Some(score);
    }
    let scores: Vec<f64> = scores.into_iter().collect::<Option<_>>()?;
    let are_probabilities = scores.iter().all(|score| (0.0..=1.0).contains(score));
    let probabilities = scores.into_iter().map(|score| {
        let probability = if are_probabilities {
            score
        } else {
            1.0 / (1.0 + (-score).exp())
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a probability from 0 to 1 fits a 32-bit float"
        )]
        let narrow = probability as f32;
        narrow
    });
    Some(probabilities.collect())
}

/// What the test of a search profile checks.
#[derive(Debug, Clone)]
pub struct SearchTestInput {
    /// What the model does.
    pub kind: SearchKind,
    /// Where its calls go.
    pub target: SearchTarget,
    /// The outcome of reading the credential, which the caller checked.
    pub credential_check: CheckOutcome,
}

/// Runs the test of a search profile: the credential, the server and the
/// model, then a short embedding or a rerank of a passage that answers a
/// question and one that does not.
pub async fn test_search_profile(client: &HttpClient, input: SearchTestInput) -> SearchTestReport {
    let work = match input.kind {
        SearchKind::Embeddings => SearchCheckKind::Embed,
        SearchKind::Rerank => SearchCheckKind::Rerank,
    };
    let mut report = SearchTestReport {
        checks: vec![SearchCheck {
            check: SearchCheckKind::Credential,
            outcome: input.credential_check.clone(),
        }],
        server: None,
        dimensions: None,
        scores: None,
    };
    if matches!(input.credential_check, CheckOutcome::Failed { .. }) {
        for check in [SearchCheckKind::Server, SearchCheckKind::Model, work] {
            report
                .checks
                .push(skipped(check, SkipReason::EarlierCheckFailed));
        }
        return report;
    }

    let target = &input.target;
    let list = detect::list_models(client, &target.base_url, target.credential.as_ref()).await;
    report.server = match &list {
        ModelList::Failed(_) => detect::server_from_host(&target.base_url),
        ModelList::Listed(_) | ModelList::Unavailable => {
            Some(detect::detect_server(client, &target.base_url).await)
        }
    };
    let (server, model) = server_and_model(&list, &target.model);
    let server_failed = matches!(server, CheckOutcome::Failed { .. });
    report.checks.push(SearchCheck {
        check: SearchCheckKind::Server,
        outcome: server,
    });
    report.checks.push(SearchCheck {
        check: SearchCheckKind::Model,
        outcome: model,
    });
    if server_failed {
        report
            .checks
            .push(skipped(work, SkipReason::EarlierCheckFailed));
        return report;
    }

    let outcome = match input.kind {
        SearchKind::Embeddings => match embed(client, target, &[EMBED_PROBE.to_owned()]).await {
            Ok(vectors) => {
                report.dimensions = vectors
                    .first()
                    .and_then(|vector| u32::try_from(vector.len()).ok());
                CheckOutcome::Passed
            }
            Err(error) => CheckOutcome::Failed {
                failure: error.failure(),
            },
        },
        SearchKind::Rerank => {
            let documents = [RERANK_ANSWER.to_owned(), RERANK_UNRELATED.to_owned()];
            match rerank(client, target, RERANK_QUESTION, &documents).await {
                Ok(scores) => rerank_outcome(&scores, &mut report),
                Err(error) => CheckOutcome::Failed {
                    failure: error.failure(),
                },
            }
        }
    };
    report.checks.push(SearchCheck {
        check: work,
        outcome,
    });
    report
}

/// The outcomes of the server check and the model check from the list of
/// models, as the provider test reads it.
fn server_and_model(list: &ModelList, model: &str) -> (CheckOutcome, CheckOutcome) {
    match list {
        ModelList::Listed(models) => {
            let is_listed = models.iter().any(|listed| listed.id == model);
            let model = if is_listed {
                CheckOutcome::Passed
            } else {
                CheckOutcome::Warning {
                    note: CheckNote::ModelNotListed,
                }
            };
            (CheckOutcome::Passed, model)
        }
        ModelList::Unavailable => (
            CheckOutcome::Warning {
                note: CheckNote::ModelListUnavailable,
            },
            CheckOutcome::Skipped {
                reason: SkipReason::NoModelList,
            },
        ),
        ModelList::Failed(failure) => (
            CheckOutcome::Failed {
                failure: failure.clone(),
            },
            CheckOutcome::Skipped {
                reason: SkipReason::EarlierCheckFailed,
            },
        ),
    }
}

/// The rerank check passes when the passage that answers scores higher than
/// the one that does not.
fn rerank_outcome(scores: &[f32], report: &mut SearchTestReport) -> CheckOutcome {
    let [answering, unrelated] = scores else {
        return CheckOutcome::Failed {
            failure: RunFailure::InvalidResponse,
        };
    };
    report.scores = Some(RerankProbe {
        answering: *answering,
        unrelated: *unrelated,
    });
    if answering > unrelated {
        CheckOutcome::Passed
    } else {
        CheckOutcome::Warning {
            note: CheckNote::UnexpectedOrder,
        }
    }
}

fn skipped(check: SearchCheckKind, reason: SkipReason) -> SearchCheck {
    SearchCheck {
        check,
        outcome: CheckOutcome::Skipped { reason },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{read_embeddings, read_scores};

    #[test]
    fn embeddings_come_back_in_the_order_of_the_inputs() {
        let answer = json!({ "data": [
            { "index": 1, "embedding": [0.5, -0.5] },
            { "index": 0, "embedding": [1.0, 0.0] },
        ] });

        assert_eq!(
            read_embeddings(&answer, 2),
            Some(vec![vec![1.0, 0.0], vec![0.5, -0.5]])
        );
    }

    #[test]
    fn embeddings_without_indexes_keep_their_places() {
        let answer = json!({ "data": [{ "embedding": [1.0] }, { "embedding": [2.0] }] });

        assert_eq!(
            read_embeddings(&answer, 2),
            Some(vec![vec![1.0], vec![2.0]])
        );
    }

    #[test]
    fn embeddings_that_do_not_fit_the_inputs_are_refused() {
        let missing = json!({ "data": [{ "index": 0, "embedding": [1.0] }] });
        let twice = json!({ "data": [
            { "index": 0, "embedding": [1.0] },
            { "index": 0, "embedding": [2.0] },
        ] });
        let ragged = json!({ "data": [
            { "index": 0, "embedding": [1.0, 2.0] },
            { "index": 1, "embedding": [1.0] },
        ] });
        let empty = json!({ "data": [{ "index": 0, "embedding": [] }] });
        let not_numbers = json!({ "data": [{ "index": 0, "embedding": ["a"] }] });
        let out_of_range = json!({ "data": [{ "index": 3, "embedding": [1.0] }] });

        assert_eq!(read_embeddings(&missing, 2), None);
        assert_eq!(read_embeddings(&twice, 2), None);
        assert_eq!(read_embeddings(&ragged, 2), None);
        assert_eq!(read_embeddings(&empty, 1), None);
        assert_eq!(read_embeddings(&not_numbers, 1), None);
        assert_eq!(read_embeddings(&out_of_range, 1), None);
        assert_eq!(read_embeddings(&json!({ "object": "list" }), 1), None);
    }

    #[test]
    fn scores_come_back_in_the_order_of_the_documents_in_both_shapes() {
        let jina = json!({ "results": [
            { "index": 1, "relevance_score": 0.25 },
            { "index": 0, "relevance_score": 0.75 },
        ] });
        let tei = json!([{ "index": 0, "score": 0.5 }, { "index": 1, "score": 0.125 }]);

        assert_eq!(read_scores(&jina, 2), Some(vec![0.75, 0.25]));
        assert_eq!(read_scores(&tei, 2), Some(vec![0.5, 0.125]));
    }

    #[test]
    fn logits_become_probabilities() {
        let logits = json!({ "results": [
            { "index": 0, "relevance_score": 0.0 },
            { "index": 1, "relevance_score": -2.0 },
            { "index": 2, "relevance_score": 3.5 },
        ] });

        let scores = read_scores(&logits, 3).unwrap();

        assert!((scores[0] - 0.5).abs() < 1e-6);
        assert!((scores[1] - 0.119_203).abs() < 1e-5);
        assert!((scores[2] - 0.970_688).abs() < 1e-5);
    }

    #[test]
    fn scores_that_miss_a_document_or_name_one_twice_are_refused() {
        let missing = json!({ "results": [{ "index": 0, "relevance_score": 0.5 }] });
        let twice = json!({ "results": [
            { "index": 0, "relevance_score": 0.5 },
            { "index": 0, "relevance_score": 0.4 },
        ] });
        let no_score = json!({ "results": [{ "index": 0 }] });

        assert_eq!(read_scores(&missing, 2), None);
        assert_eq!(read_scores(&twice, 2), None);
        assert_eq!(read_scores(&no_score, 1), None);
    }
}
