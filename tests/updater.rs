//! Stable-channel release selection and mandatory binary verification.
//!
//! The published ladder runs behind the working tree: every 0.x.x release is on
//! GitHub while the suites build 1.0.0 locally. A check that treated "tag
//! differs from my version" as an update would pull 0.9.0 binaries over a 1.0.0
//! install, so the direction of the difference is the whole contract here.
use fennec_rs::updater::{
    Asset, CHECKSUMS_ASSET, Release, checksum_for, download_asset, download_release_asset, is_newer,
};

#[test]
fn a_higher_release_is_an_update() {
    assert_eq!(is_newer("1.0.1", "1.0.0"), Some(true));
    assert_eq!(is_newer("1.1.0", "1.0.9"), Some(true));
    assert_eq!(is_newer("2.0.0", "1.9.9"), Some(true));
    assert_eq!(is_newer("0.10.0", "0.9.0"), Some(true), "0.10 is not 0.1");
}

#[test]
fn the_same_release_or_an_older_one_is_not() {
    assert_eq!(is_newer("1.0.0", "1.0.0"), Some(false));
    assert_eq!(
        is_newer("0.9.0", "1.0.0"),
        Some(false),
        "never downgrade an install"
    );
    assert_eq!(
        is_newer("0.9.0", "1.0.0-rc1"),
        None,
        "a pre-release is not a number"
    );
}

#[test]
fn a_tag_the_comparator_does_not_understand_offers_nothing() {
    assert_eq!(
        is_newer("v1.0.0", "1.0.0"),
        Some(false),
        "the leading v is the tag spelling"
    );
    assert_eq!(
        is_newer("dev-abc1234", "1.0.0"),
        None,
        "the experimental channel is not a version"
    );
    assert_eq!(is_newer("", "1.0.0"), None);
    assert_eq!(
        is_newer("1.0", "1.0.0"),
        Some(false),
        "a missing patch field reads as zero"
    );
}

#[test]
fn the_asset_name_follows_the_platform() {
    let platform = fennec_rs::updater::Platform {
        os: "windows".to_string(),
        arch: "x86_64".to_string(),
        ext: "exe".to_string(),
    };
    assert_eq!(
        platform.asset_name("fennec-cli"),
        "fennec-cli-windows-x86_64.exe"
    );
    let linux = fennec_rs::updater::Platform {
        os: "linux".to_string(),
        arch: "aarch64".to_string(),
        ext: "bin".to_string(),
    };
    assert_eq!(linux.asset_name("fennec-cli"), "fennec-cli-linux-aarch64");
}

const BINARY: &str = "fennec-cli-windows-x86_64.exe";

fn release(server_url: Option<String>) -> Release {
    let server_url = server_url.unwrap_or_else(|| "http://127.0.0.1".to_string());
    Release {
        version: "1.0.1".to_string(),
        tag: "v1.0.1".to_string(),
        published_at: "2026-11-24T00:00:00Z".to_string(),
        assets: vec![Asset {
            name: BINARY.to_string(),
            download_url: format!("{server_url}/binary"),
            size: 5,
        }],
        checksums_url: Some(format!("{server_url}/SHA256SUMS")),
    }
}

fn serve(
    responses: Vec<(&'static str, &'static str, Vec<u8>)>,
) -> (
    String,
    std::thread::JoinHandle<()>,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let paths = Arc::new(Mutex::new(Vec::new()));
    let requested = paths.clone();
    let handle = std::thread::spawn(move || {
        for (expected_path, status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or_default();
            requested.lock().unwrap().push(path.to_string());
            assert_eq!(path, expected_path);
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
        }
    });
    (format!("http://{address}"), handle, paths)
}

#[tokio::test]
async fn a_checksums_asset_is_required() {
    let mut missing = release(None);
    missing.checksums_url = None;
    let error = checksum_for(&missing, BINARY).await.unwrap_err();
    assert!(error.contains("missing SHA256SUMS"), "{error}");
    let error = download_release_asset(&missing, BINARY).await.unwrap_err();
    assert!(error.contains("missing SHA256SUMS"), "{error}");
}

#[tokio::test]
async fn manifest_entry_must_match_the_exact_basename_and_is_normalized() {
    let digest = "2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824";
    let body = format!("{}  other.exe\n{} *{BINARY}\n", "a".repeat(64), digest);
    let (url, server, _) = serve(vec![("/SHA256SUMS", "200 OK", body.into_bytes())]);
    let checksum = checksum_for(&release(Some(url)), BINARY).await.unwrap();
    server.join().unwrap();
    assert_eq!(
        checksum.as_deref(),
        Some(digest.to_ascii_lowercase().as_str())
    );

    for file in [
        format!("./{BINARY}"),
        format!("dir/{BINARY}"),
        format!("{BINARY}.bak"),
        format!("prefix-{BINARY}"),
    ] {
        let body = format!("{digest}  {file}\n");
        let (url, server, _) = serve(vec![("/SHA256SUMS", "200 OK", body.into_bytes())]);
        let checksum = checksum_for(&release(Some(url)), BINARY).await.unwrap();
        server.join().unwrap();
        assert_eq!(checksum, None, "{file}");
    }
}

#[tokio::test]
async fn malformed_or_ambiguous_manifests_are_errors() {
    for body in [
        format!("{}  {BINARY}\n", "a".repeat(63)),
        format!("{}  {BINARY}\n", "a".repeat(65)),
        format!("{}  {BINARY}\n", "g".repeat(64)),
        "missing-fields\n".to_string(),
        format!("{}  other.exe extra\n", "a".repeat(64)),
        format!(
            "{}  {BINARY}\n{} *{BINARY}\n",
            "a".repeat(64),
            "b".repeat(64)
        ),
    ] {
        let (url, server, _) = serve(vec![("/SHA256SUMS", "200 OK", body.into_bytes())]);
        assert!(checksum_for(&release(Some(url)), BINARY).await.is_err());
        server.join().unwrap();
    }
}

#[tokio::test]
async fn manifest_fetch_and_utf8_parse_failures_are_errors() {
    for (status, body) in [
        ("404 Not Found", b"missing".to_vec()),
        ("500 Internal Server Error", b"failed".to_vec()),
        ("200 OK", vec![0xff, 0xfe]),
    ] {
        let (url, server, _) = serve(vec![("/SHA256SUMS", status, body)]);
        let error = checksum_for(&release(Some(url)), BINARY).await.unwrap_err();
        server.join().unwrap();
        assert!(error.contains(CHECKSUMS_ASSET), "{error}");
    }
}

#[tokio::test]
async fn a_missing_entry_stops_before_the_binary_download() {
    let body = format!("{}  other.exe\n", "a".repeat(64));
    let (url, server, paths) = serve(vec![("/SHA256SUMS", "200 OK", body.into_bytes())]);
    let error = download_release_asset(&release(Some(url)), BINARY)
        .await
        .unwrap_err();
    server.join().unwrap();
    assert!(error.contains("no entry for"), "{error}");
    assert_eq!(*paths.lock().unwrap(), ["/SHA256SUMS"]);
}

#[tokio::test]
async fn a_mismatched_binary_is_never_returned_for_installation() {
    let manifest = format!(
        "{}  {BINARY}\n",
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    let (url, server, paths) = serve(vec![
        ("/SHA256SUMS", "200 OK", manifest.into_bytes()),
        ("/binary", "200 OK", b"tampered".to_vec()),
    ]);
    let error = download_release_asset(&release(Some(url)), BINARY)
        .await
        .unwrap_err();
    server.join().unwrap();
    assert!(error.contains("Checksum mismatch"), "{error}");
    assert_eq!(*paths.lock().unwrap(), ["/SHA256SUMS", "/binary"]);
}

#[tokio::test]
async fn a_matching_uppercase_digest_allows_the_binary() {
    let manifest = format!(
        "{} *{BINARY}\n",
        "2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824"
    );
    let (url, server, paths) = serve(vec![
        ("/SHA256SUMS", "200 OK", manifest.into_bytes()),
        ("/binary", "200 OK", b"hello".to_vec()),
    ]);
    let bytes = download_release_asset(&release(Some(url)), BINARY)
        .await
        .unwrap();
    server.join().unwrap();
    assert_eq!(bytes, b"hello");
    assert_eq!(*paths.lock().unwrap(), ["/SHA256SUMS", "/binary"]);
}

#[tokio::test]
async fn a_supplied_digest_must_be_well_formed() {
    let error = download_asset("http://127.0.0.1:0", Some("not-a-digest"))
        .await
        .unwrap_err();
    assert!(error.contains("64 hexadecimal"), "{error}");
}
