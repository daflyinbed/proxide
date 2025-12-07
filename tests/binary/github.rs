use futures::StreamExt;
use proxide::{
    binary::{BinarySource, github::GithubProvider},
    config::GithubConfig,
};
use serde_json::Value;
use std::fs;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};
const OWNER: &str = "sass";
const REPO: &str = "node-sass";

async fn setup_api() -> MockServer {
    let content1 = fs::read_to_string("tests/binary/resources/github_releases_1.json").unwrap();
    let content2 = fs::read_to_string("tests/binary/resources/github_releases_2.json").unwrap();

    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/repos/{OWNER}/{REPO}/releases")))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::from_str::<Value>(&content1).unwrap()),
        )
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/repos/{OWNER}/{REPO}/releases")))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::from_str::<Value>(&content2).unwrap()),
        )
        .mount(&mock_server)
        .await;
    mock_server
}

#[tokio::test]
async fn test_github_provider() {
    setup_api().await;
    let provider = GithubProvider::new(GithubConfig {
        repo: REPO.to_string(),
        owner: OWNER.to_string(),
    })
    .unwrap();
    let root = provider
        .list("/")
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(root.len(), 121);
    let inner = provider
        .list("/v9.0.0/")
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(inner.len(), 30);
}
