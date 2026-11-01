//! Owner operations on the owner's SQLite files (ADR-0056, decision 8): an
//! operation that ran when Mannie ended is uncertain after the next start,
//! and never runs again.

use mannie_runtime::testing::Harness;
use mannie_store::dbfiles::{NewOperation, OperationError, OperationState};
use mannie_types::ids::{DatabaseKey, DatabaseOperationId};
use mannie_types::tools::StatementClass;

#[tokio::test]
async fn an_owner_operation_that_ran_when_mannie_ended_is_uncertain_after_the_start() {
    let harness = Harness::start().await;
    let operation = NewOperation {
        id: DatabaseOperationId::parse("0199e0f2-0000-7000-8000-000000000201").unwrap(),
        plugin: "workers".to_owned(),
        key: DatabaseKey::parse("0199e0f2-0000-7000-8000-000000000202").unwrap(),
        statement: "DELETE FROM \"items\" WHERE \"rowid\" = ?1".to_owned(),
        parameters_digest: "digest".to_owned(),
        class: StatementClass::Write,
    };
    harness.store.begin_db_operation(&operation).await.unwrap();

    let harness = harness.restart().await;

    assert_eq!(
        harness
            .store
            .db_operation_state(&operation.id)
            .await
            .unwrap(),
        Some(OperationState::Uncertain)
    );
    assert!(matches!(
        harness.store.begin_db_operation(&operation).await,
        Err(OperationError::AlreadyJournaled)
    ));
}
