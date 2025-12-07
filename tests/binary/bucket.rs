use futures::StreamExt;
use proxide::{
    binary::{BinarySource, bucket::BucketProvider},
    config::BucketConfig,
};
use std::fs;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};
async fn setup_api() -> MockServer {
    let content1 = fs::read_to_string("tests/binary/resources/bucket_1.xml").unwrap();
    let content2 = fs::read_to_string("tests/binary/resources/bucket_2.xml").unwrap();

    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(query_param("delimiter", "/"))
        .and(query_param("prefix", ""))
        .respond_with(ResponseTemplate::new(200).set_body_string(content1.clone()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/"))
        .and(query_param("delimiter", "/"))
        .and(query_param("prefix", "96.0.4664.45/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(content2.clone()))
        .mount(&mock_server)
        .await;
    mock_server
}

#[tokio::test]
async fn test_bucket_provider() {
    let mock_server = setup_api().await;
    let provider = BucketProvider::new(
        BucketConfig {
            dist_url: format!("{}/", mock_server.uri().trim_end_matches('/')),
            ignore_dirs: vec!["/96.0.4664.45/".to_string()],
        },
        reqwest::Client::default(),
    );
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
    assert_eq!(root.len(), 240);
    let inner = provider
        .list("/96.0.4664.45/")
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(inner.len(), 5);
}
