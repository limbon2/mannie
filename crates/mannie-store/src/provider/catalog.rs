//! Managed profiles for models of the signed-in ChatGPT account. A catalog
//! refresh updates the list in one transaction, keeping existing profile IDs.

use mannie_types::ids::ProfileId;
use mannie_types::provider::{ApiInterface, ModelLimits, ProfileAccess, ReasoningEffort};
use rusqlite::params;

use super::{DOCUMENT_VERSION, ProfileFields, ProfileStoreError, document_json};
use crate::writer::TransactError;
use crate::{Durability, Store};

pub(super) const WITHDRAW: &str =
    "UPDATE provider_profiles SET catalog_position = NULL WHERE catalog_position IS NOT NULL";

const UPSERT: &str = "INSERT INTO provider_profiles \
    (id, name, interface, base_url, model, is_default, version, document, created_at, updated_at, \
    catalog_account, catalog_position, catalog_efforts) \
    VALUES (?1, ?2, 'responses', ?3, ?4, 0, ?5, ?6, ?7, ?7, ?8, ?9, ?10) \
    ON CONFLICT (model) WHERE catalog_account IS NOT NULL DO UPDATE SET \
    name = excluded.name, base_url = excluded.base_url, document = excluded.document, \
    updated_at = excluded.updated_at, catalog_account = excluded.catalog_account, \
    catalog_position = excluded.catalog_position, catalog_efforts = excluded.catalog_efforts";

/// The statements checked against the migrated schema.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[WITHDRAW, UPSERT];

/// The checked fields of a model in a fresh catalog.
#[derive(Debug)]
pub struct CatalogModel {
    /// The ID to use if Mannie has never seen this model.
    pub new_id: ProfileId,
    /// The model ID in requests.
    pub slug: String,
    /// The name to show in pickers.
    pub display_name: String,
    /// The validated local context and output budgets.
    pub limits: ModelLimits,
    /// The efforts the model advertises, when present.
    pub efforts: Option<Vec<ReasoningEffort>>,
}

impl Store {
    /// Reconciles the catalog while the caller holds the account's session
    /// gate. Withdrawn models remain readable for pinned conversations.
    pub async fn replace_chatgpt_catalog(
        &self,
        account_key: String,
        base_url: String,
        models: Vec<CatalogModel>,
    ) -> Result<(), ProfileStoreError> {
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            transaction.execute(WITHDRAW, [])?;

            let mut insert = transaction.prepare_cached(UPSERT)?;

            for (position, model) in (0_i64..).zip(models) {
                let fields = ProfileFields {
                    access: ProfileAccess::ChatGptPlan,
                    interface: ApiInterface::Responses,
                    name: model.display_name,
                    model: model.slug,
                    base_url: base_url.clone(),
                    credential_ref: None,
                    context_window: model.limits.context_window,
                    output_limit: model.limits.output_limit,
                    reasoning_effort: ReasoningEffort::ServerDefault,
                    use_tools: true,
                };
                let document = document_json(&fields, None).map_err(TransactError::Rejected)?;
                let efforts = model
                    .efforts
                    .map(|values| serde_json::to_string(&values))
                    .transpose()
                    .map_err(|error| {
                        TransactError::Rejected(ProfileStoreError::InvalidData {
                            what: error.to_string(),
                        })
                    })?;
                insert.execute(params![
                    model.new_id.as_str(),
                    fields.name,
                    base_url,
                    fields.model,
                    DOCUMENT_VERSION,
                    document,
                    now,
                    account_key,
                    position,
                    efforts
                ])?;
            }
            Ok(())
        })
        .await
        .map_err(ProfileStoreError::from)
    }
}
