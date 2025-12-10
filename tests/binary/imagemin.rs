use std::collections::HashMap;

use chrono::DateTime;
use futures::StreamExt;
use proxide::{
    binary::{BinaryEntry, BinarySource, imagemin::ImageminProvider},
    config::ImageminConfig,
};
use reqwest::Client;
use serde_json::Value;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path as match_path},
};

async fn setup_registry_mock() -> MockServer {
    let pkg = std::fs::read_to_string("tests/binary/resources/jpegtran_bin_pkg.json").unwrap();
    let pkg_json: Value = serde_json::from_str(&pkg).unwrap();

    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(match_path("/jpegtran-bin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pkg_json.clone()))
        .mount(&server)
        .await;

    let head_cases = [
        (
            "/imagemin/jpegtran-bin/7.0.0/vendor/macos/jpegtran",
            386_180u64,
        ),
        (
            "/imagemin/jpegtran-bin/7.0.0/vendor/sunos/x64/jpegtran",
            244_328u64,
        ),
        (
            "/imagemin/jpegtran-bin/7.0.0/vendor/sunos/x86/jpegtran",
            221_624u64,
        ),
        (
            "/imagemin/jpegtran-bin/7.0.0/vendor/win/x64/libjpeg-62.dll",
            588_639u64,
        ),
        (
            "/imagemin/jpegtran-bin/7.0.0/vendor/win/x64/jpegtran.exe",
            82_997u64,
        ),
        (
            "/imagemin/jpegtran-bin/7.0.0/vendor/win/x86/libjpeg-62.dll",
            581_390u64,
        ),
        (
            "/imagemin/jpegtran-bin/7.0.0/vendor/win/x86/jpegtran.exe",
            73_579u64,
        ),
        (
            "/imagemin/jpegtran-bin/7.0.0/vendor/freebsd/x64/jpegtran",
            34_432u64,
        ),
        (
            "/imagemin/jpegtran-bin/7.0.0/vendor/freebsd/x86/jpegtran",
            26_960u64,
        ),
        (
            "/imagemin/jpegtran-bin/7.0.0/vendor/linux/x64/jpegtran",
            306_001u64,
        ),
        (
            "/imagemin/jpegtran-bin/7.0.0/vendor/linux/x86/jpegtran",
            176_184u64,
        ),
        (
            "/imagemin/jpegtran-bin/4.0.0/vendor/macos/jpegtran",
            386_180u64,
        ),
        (
            "/imagemin/jpegtran-bin/4.0.0/vendor/sunos/x64/jpegtran",
            244_328u64,
        ),
        (
            "/imagemin/jpegtran-bin/4.0.0/vendor/sunos/x86/jpegtran",
            221_624u64,
        ),
        (
            "/imagemin/jpegtran-bin/4.0.0/vendor/win/x64/libjpeg-62.dll",
            588_639u64,
        ),
        (
            "/imagemin/jpegtran-bin/4.0.0/vendor/win/x64/jpegtran.exe",
            82_997u64,
        ),
        (
            "/imagemin/jpegtran-bin/4.0.0/vendor/win/x86/libjpeg-62.dll",
            581_390u64,
        ),
        (
            "/imagemin/jpegtran-bin/4.0.0/vendor/win/x86/jpegtran.exe",
            73_579u64,
        ),
        (
            "/imagemin/jpegtran-bin/4.0.0/vendor/freebsd/x64/jpegtran",
            34_432u64,
        ),
        (
            "/imagemin/jpegtran-bin/4.0.0/vendor/freebsd/x86/jpegtran",
            26_960u64,
        ),
        (
            "/imagemin/jpegtran-bin/4.0.0/vendor/linux/x64/jpegtran",
            306_001u64,
        ),
        (
            "/imagemin/jpegtran-bin/4.0.0/vendor/linux/x86/jpegtran",
            176_184u64,
        ),
    ];

    for (url, size) in head_cases {
        Mock::given(method("HEAD"))
            .and(match_path(url))
            .respond_with(
                ResponseTemplate::new(200).append_header("content-length", size.to_string()),
            )
            .mount(&server)
            .await;
    }

    server
}

#[tokio::test]
async fn test_imagemin_provider_list() {
    let mock_server = setup_registry_mock().await;

    let node_platforms = vec![
        "macos".to_string(),
        "linux".to_string(),
        "freebsd".to_string(),
        "sunos".to_string(),
        "win".to_string(),
    ];

    let mut node_archs = HashMap::new();
    node_archs.insert("macos".to_string(), vec![]);
    node_archs.insert(
        "linux".to_string(),
        vec!["x86".to_string(), "x64".to_string()],
    );
    node_archs.insert(
        "freebsd".to_string(),
        vec!["x86".to_string(), "x64".to_string()],
    );
    node_archs.insert(
        "sunos".to_string(),
        vec!["x86".to_string(), "x64".to_string()],
    );
    node_archs.insert(
        "win".to_string(),
        vec!["x86".to_string(), "x64".to_string()],
    );

    let mut bin_files = HashMap::new();
    bin_files.insert("macos".to_string(), vec!["jpegtran".to_string()]);
    bin_files.insert("linux".to_string(), vec!["jpegtran".to_string()]);
    bin_files.insert("freebsd".to_string(), vec!["jpegtran".to_string()]);
    bin_files.insert("sunos".to_string(), vec!["jpegtran".to_string()]);
    bin_files.insert(
        "win".to_string(),
        vec!["jpegtran.exe".to_string(), "libjpeg-62.dll".to_string()],
    );

    let provider = ImageminProvider::new(
        "jpegtran-bin",
        ImageminConfig {
            npm_registry_url: mock_server.uri(),
            dist_url: mock_server.uri(),
            repo: "imagemin/jpegtran-bin".to_string(),
            npm_package_name: None,
            node_platforms,
            node_archs,
            bin_files,
        },
        Client::default(),
    );

    let root = provider
        .list("/")
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await;

    let mut root_names: Vec<String> = root.into_iter().map(|e| e.name).collect();
    root_names.sort();
    assert_eq!(root_names, vec!["v4.0.0/", "v7.0.0/"]);

    let version = provider
        .list("/7.0.0/")
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await;
    assert_eq!(version.len(), 1);
    assert_eq!(version[0].name, "vendor/");

    let platforms = provider
        .list("/7.0.0/vendor/")
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await;
    let platform_names: Vec<String> = platforms.into_iter().map(|e| e.name).collect();
    assert_eq!(
        platform_names,
        vec!["macos/", "linux/", "freebsd/", "sunos/", "win/"]
    );

    let arches = provider
        .list("/7.0.0/vendor/linux/")
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await;

    let arch_names: Vec<String> = arches.into_iter().map(|e| e.name).collect();
    assert_eq!(arch_names, vec!["x86/", "x64/"]);
    let files = provider
        .list("/7.0.0/vendor/linux/x64/")
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await;
    assert_eq!(
        files[0],
        BinaryEntry {
            name: "jpegtran".to_string(),
            is_dir: false,
            url: Some(format!(
                "{}/imagemin/jpegtran-bin/7.0.0/vendor/linux/x64/jpegtran",
                mock_server.uri()
            )),
            size: Some(306_001u64),
            date: Some(
                DateTime::parse_from_rfc3339("2022-05-12T11:08:26.293Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc)
            ),
        }
    );
    let macos = provider
        .list("/7.0.0/vendor/macos/")
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await;
    assert_eq!(macos.len(), 1);
    assert_eq!(
        macos[0],
        BinaryEntry {
            name: "jpegtran".to_string(),
            is_dir: false,
            url: Some(format!(
                "{}/imagemin/jpegtran-bin/7.0.0/vendor/macos/jpegtran",
                mock_server.uri()
            )),
            size: Some(386_180u64),
            date: Some(
                DateTime::parse_from_rfc3339("2022-05-12T11:08:26.293Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc)
            ),
        }
    );
    let win = provider
        .list("/7.0.0/vendor/win/x64/")
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await;
    assert_eq!(win.len(), 2);
    assert_eq!(
        win[0],
        BinaryEntry {
            name: "jpegtran.exe".to_string(),
            is_dir: false,
            url: Some(format!(
                "{}/imagemin/jpegtran-bin/7.0.0/vendor/win/x64/jpegtran.exe",
                mock_server.uri()
            )),
            size: Some(82_997u64),
            date: Some(
                DateTime::parse_from_rfc3339("2022-05-12T11:08:26.293Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc)
            ),
        }
    );
    assert_eq!(
        win[1],
        BinaryEntry {
            name: "libjpeg-62.dll".to_string(),
            is_dir: false,
            url: Some(format!(
                "{}/imagemin/jpegtran-bin/7.0.0/vendor/win/x64/libjpeg-62.dll",
                mock_server.uri()
            )),
            size: Some(588_639u64),
            date: Some(
                DateTime::parse_from_rfc3339("2022-05-12T11:08:26.293Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc)
            ),
        }
    )
}
