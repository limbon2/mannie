use super::*;
use crate::StoredSkillSwitch;

fn switch(project_id: Option<&ProjectId>, name: &str, is_on: bool) -> StoredSkillSwitch {
    StoredSkillSwitch {
        project_id: project_id.cloned(),
        name: name.to_owned(),
        is_on,
        digest: None,
    }
}

#[tokio::test]
async fn a_switch_replaces_the_earlier_one_of_its_place_and_name() {
    let fixture = Fixture::new();
    let project = fixture.project("/work/mannie").await;
    let digest = "a".repeat(64);

    fixture
        .store
        .set_skill_switch(&switch(None, "pdf", false))
        .await
        .unwrap();
    fixture
        .store
        .set_skill_switch(&StoredSkillSwitch {
            digest: Some(digest.clone()),
            ..switch(Some(&project), "pdf", true)
        })
        .await
        .unwrap();
    fixture
        .store
        .set_skill_switch(&switch(None, "pdf", true))
        .await
        .unwrap();

    let switches = fixture.store.skill_switches().await.unwrap();
    assert_eq!(
        switches,
        [
            switch(None, "pdf", true),
            StoredSkillSwitch {
                digest: Some(digest),
                ..switch(Some(&project), "pdf", true)
            },
        ]
    );
}

#[tokio::test]
async fn a_project_s_switches_go_with_the_project() {
    let fixture = Fixture::new();
    let project = fixture.project("/work/mannie").await;
    fixture
        .store
        .set_skill_switch(&switch(Some(&project), "pdf", true))
        .await
        .unwrap();
    fixture
        .store
        .set_skill_switch(&switch(None, "docx", false))
        .await
        .unwrap();

    fixture.store.delete_project(&project).await.unwrap();

    assert_eq!(
        fixture.store.skill_switches().await.unwrap(),
        [switch(None, "docx", false)]
    );
}

#[tokio::test]
async fn a_switch_of_a_missing_project_is_refused_and_stores_nothing() {
    let fixture = Fixture::new();
    let missing = ProjectId::parse(&fixture.uuid()).unwrap();

    let refused = fixture
        .store
        .set_skill_switch(&switch(Some(&missing), "pdf", true))
        .await;

    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(fixture.store.skill_switches().await.unwrap(), []);
}
