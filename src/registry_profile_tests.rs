use super::*;

async fn set(
    h: &mut Harness,
    profile: Value,
    credentials: Option<crate::ipc::CredentialSource>,
) -> Response {
    let request = Request::SetProjectProfile {
        project: "web".into(),
        profile,
        credentials,
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), h.request(request))
        .await
        .unwrap()
}

#[tokio::test]
async fn set_project_profile_writes_the_profile_and_copies_credentials_without_their_values() {
    let mut h = Harness::new(2);
    let source = h.directory.path().join("repo.env");
    std::fs::write(&source, "TEST_PASSWORD=made-up-secret\n").unwrap();
    let response = set(
        &mut h,
        json!({"install":"npm ci","start":"npm start","env":{"APP_ENV":"test"}}),
        Some(crate::ipc::CredentialSource {
            source,
            names: vec!["TEST_PASSWORD".into(), "TEST_TOKEN".into()],
        }),
    )
    .await;
    let shown = serde_json::to_string(&response).unwrap();
    assert!(!shown.contains("made-up-secret"), "{shown}");
    let Response::ProfileWritten {
        path,
        profile,
        credentials,
    } = response
    else {
        panic!("{response:?}");
    };
    let config = h.registry.dependencies.projects_config.clone();
    assert_eq!(path, config.join("web.toml"));
    assert!(profile.contains("npm ci"), "{profile}");
    let credentials = credentials.unwrap();
    assert_eq!(credentials.copied, ["TEST_PASSWORD"]);
    assert_eq!(credentials.missing, ["TEST_TOKEN"]);
    assert!(
        std::fs::read_to_string(config.join("web.env"))
            .unwrap()
            .contains("made-up-secret")
    );
}

#[tokio::test]
async fn set_project_profile_refuses_an_invalid_profile_and_keeps_the_old_one() {
    let mut h = Harness::new(2);
    let config = h.registry.dependencies.projects_config.clone();
    std::fs::write(config.join("web.toml"), "start = \"old\"\n").unwrap();
    let message = refused(set(&mut h, json!({"start":""}), None).await);
    assert!(message.contains("start is empty"), "{message}");
    assert_eq!(
        std::fs::read_to_string(config.join("web.toml")).unwrap(),
        "start = \"old\"\n"
    );
}

#[tokio::test]
async fn the_first_set_project_profile_with_credentials_creates_the_config_folder() {
    let mut h = Harness::new(2);
    let config = h.registry.dependencies.projects_config.clone();
    std::fs::remove_dir_all(&config).unwrap();
    let source = h.directory.path().join("repo.env");
    std::fs::write(&source, "TEST_PASSWORD=made-up-secret\n").unwrap();
    let response = set(
        &mut h,
        json!({"start":"npm start"}),
        Some(crate::ipc::CredentialSource {
            source,
            names: vec!["TEST_PASSWORD".into()],
        }),
    )
    .await;
    assert!(
        matches!(response, Response::ProfileWritten { .. }),
        "{response:?}"
    );
    assert!(config.join("web.env").is_file() && config.join("web.toml").is_file());
}

#[path = "registry_prepare_tests.rs"]
mod prepare;
