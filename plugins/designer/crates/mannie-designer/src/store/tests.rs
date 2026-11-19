use std::path::Path;

use futures_util::future::join;
use mannie_plugin_harness::runtime::{Harness, open_store_with};
use mannie_plugin_harness::store::{PluginModule, Store};
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::services::store::StoreModule;

use super::{
    DesignerStore, INSERT_PROJECT, MODULE, PREFIX, PROJECT_MIGRATION, ProjectRecord,
    SELECT_PROJECTS,
};
use crate::document::{DocumentElement, DocumentFailure};

const OLD_MODULE: StoreModule = StoreModule::new(
    PREFIX,
    &[PROJECT_MIGRATION],
    &[SELECT_PROJECTS, INSERT_PROJECT],
);
const FIRST: &str = "0199e0f2-0000-7000-8000-000000000001";
const SECOND: &str = "0199e0f2-0000-7000-8000-000000000002";

fn open(path: &Path, module: StoreModule) -> (Store, DesignerStore) {
    let id = PluginId::parse(PREFIX).unwrap();
    let store = open_store_with(path, vec![PluginModule::new(&id, module)]);
    let designer = DesignerStore::new(store.plugin_store(&id).unwrap());
    (store, designer)
}

async fn add_project(designer: &DesignerStore, id: &str) {
    designer
        .insert_project(&ProjectRecord {
            id: id.to_owned(),
            name: id.to_owned(),
            created_at: 1000,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn an_upgrade_and_concurrent_first_opens_keep_one_stable_rectangle() {
    let harness = Harness::start().await;
    let path = harness.data_folder().join("designer-upgrade.sqlite");
    {
        let (_store, previous) = open(&path, OLD_MODULE);

        add_project(&previous, FIRST).await;
    }

    let document = {
        let (_store, designer) = open(&path, MODULE);
        let (first, again) = join(
            designer.document(FIRST, 2000),
            designer.document(FIRST, 3000),
        )
        .await;
        let first = first.unwrap();
        assert_eq!(again.unwrap(), first);
        assert_eq!(first.revision, 1);
        assert_eq!(first.element.hue, 0);
        assert_eq!(
            [
                first.element.x,
                first.element.y,
                first.element.width,
                first.element.height
            ],
            [64.0, 64.0, 240.0, 160.0]
        );

        add_project(&designer, SECOND).await;

        let second = designer.document(SECOND, 4000).await.unwrap();
        assert_ne!(first.element.id, second.element.id);
        assert_eq!(
            designer.set_document_hue(SECOND, &first, 1).await,
            Err(DocumentFailure::StaleRevision)
        );

        let saved = designer.set_document_hue(FIRST, &first, 360).await.unwrap();
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.element.id, first.element.id);
        assert_eq!(
            designer.set_document_hue(FIRST, &first, 230).await,
            Err(DocumentFailure::StaleRevision)
        );
        assert_eq!(designer.document(SECOND, 5000).await.unwrap(), second);
        saved
    };

    let (_store, reopened) = open(&path, MODULE);
    assert_eq!(document.element.hue, 360);
    assert_eq!(reopened.document(FIRST, 6000).await.unwrap(), document);
}

#[tokio::test]
async fn failed_writes_roll_back_the_revision_and_reads_never_reseed_initialized_content() {
    let harness = Harness::start().await;
    let path = harness.data_folder().join("designer-failures.sqlite");
    let (store, designer) = open(&path, MODULE);
    assert_eq!(
        designer.document(FIRST, 1000).await,
        Err(DocumentFailure::ProjectMissing)
    );
    add_project(&designer, FIRST).await;

    store
        .execute_unlisted(
            "CREATE TRIGGER refuse_seed BEFORE INSERT ON designer_elements
            BEGIN SELECT RAISE(ABORT, 'test seed failure'); END",
        )
        .await
        .unwrap();
    assert_eq!(
        designer.document(FIRST, 1000).await,
        Err(DocumentFailure::StorageFailed)
    );
    store
        .execute_unlisted("DROP TRIGGER refuse_seed")
        .await
        .unwrap();

    let initial = designer.document(FIRST, 1000).await.unwrap();
    assert_eq!(initial.revision, 1);
    store
        .execute_unlisted(
            "CREATE TRIGGER refuse_hue BEFORE UPDATE OF hue ON designer_elements
            BEGIN SELECT RAISE(ABORT, 'test write failure'); END",
        )
        .await
        .unwrap();

    assert_eq!(
        designer.set_document_hue(FIRST, &initial, 240).await,
        Err(DocumentFailure::StorageFailed)
    );
    assert_eq!(designer.document(FIRST, 2000).await.unwrap(), initial);

    store
        .execute_unlisted("DROP TRIGGER refuse_hue")
        .await
        .unwrap();

    let saved = designer
        .set_document_hue(FIRST, &initial, 240)
        .await
        .unwrap();
    assert_eq!(saved.element.hue, 240);
    assert_eq!(saved.revision, 2);

    store
        .execute_unlisted("DELETE FROM designer_elements")
        .await
        .unwrap();
    assert_eq!(
        designer.document(FIRST, 3000).await,
        Err(DocumentFailure::StorageFailed)
    );
    let rows = store
        .query_unlisted("SELECT id FROM designer_elements")
        .await
        .unwrap();
    assert_eq!(rows, []);
}

#[test]
fn a_document_rectangles_edges_and_fill_stay_within_the_limits() {
    let mut element = DocumentElement::seed();
    element.x = -32768.0;
    element.y = 32767.0;
    element.width = 65536.0;
    element.height = 1.0;
    element.hue = 360;
    assert!(element.valid());

    for invalid in [
        DocumentElement {
            x: -32769.0,
            ..element.clone()
        },
        DocumentElement {
            width: 65537.0,
            ..element.clone()
        },
        DocumentElement {
            height: 2.0,
            ..element.clone()
        },
        DocumentElement {
            width: 0.9,
            ..element.clone()
        },
        DocumentElement {
            height: f64::NAN,
            ..element.clone()
        },
        DocumentElement {
            x: f64::INFINITY,
            ..element.clone()
        },
        DocumentElement {
            hue: 361,
            ..element.clone()
        },
        DocumentElement {
            id: "invalid id".to_owned(),
            ..element
        },
    ] {
        assert!(!invalid.valid(), "{invalid:?}");
    }
}
