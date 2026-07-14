use serde_json::{json, Map, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tauri::{test::MockRuntime, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const FIXTURE_TARGET: &str = "fixture-test-target";
const FIXTURE_PAYLOAD: &[u8] = b"test";
const FIXTURE_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXkgRTc2MjBGMTg0MkI0RTgxRgpSV1FmNkxSQ0dBOWk1M21sWWVjTzRJelQ1MVRHUHB2V3VjTlNDaDFDQk0wUVRhTG43M1k3R0ZPMw==";
const FIXTURE_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIG1pbmlzaWduIHNlY3JldCBrZXkKUldRZjZMUkNHQTlpNTlTTE9GeHo2Tnh2QVNYREplUnR1Wnlrd1FlcGJERUd0ODdpZzFCTnBXYVZXdU5ybTczWWlJaUpicTcxV2krZFA5ZUtMOE9DMzUxdndJYXNTU2JYeHdBPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNTU1Nzc5OTY2CWZpbGU6dGVzdApRdEtNWFd5WWN3ZHBaQWxQRjd0RTJFTkprUmQxdWp2S2psajFtOVJ0SFRCblpQYTVXS1U1dVdSczVHb1A1TS9WcUU4MVFGdU1LSTVrL1NmTlFVYU9BQT09";
const UNRELATED_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYwODI4ODg4QjVDRjBDOEQKUldTTkRNKzFpSWlDWUpuK2ZyWkVScVMrUVlYRUJsakU0U0h0elRUQldDZGQrcmR3cE9Uc2hyVjgK";
const INVALID_SIGNATURE: &str = "bm90IGEgbWluaXNpZ24gc2lnbmF0dXJl";

#[derive(Clone)]
struct FixtureResponse {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

impl FixtureResponse {
    fn json(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            reason: "OK",
            content_type: "application/json",
            body: body.into(),
        }
    }

    fn bytes(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            reason: "OK",
            content_type: "application/octet-stream",
            body: body.into(),
        }
    }

    fn status(status: u16, reason: &'static str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/plain",
            body: Vec::new(),
        }
    }
}

struct FixtureServer {
    address: String,
    requests: Arc<Mutex<Vec<String>>>,
    shutdown: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl FixtureServer {
    fn start(build_routes: impl FnOnce(&str) -> HashMap<String, FixtureResponse>) -> FixtureServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind updater fixture server");
        listener
            .set_nonblocking(true)
            .expect("make updater fixture server nonblocking");
        let address = listener
            .local_addr()
            .expect("read fixture address")
            .to_string();
        let base_url = format!("http://{address}");
        let routes = build_routes(&base_url);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let worker_requests = Arc::clone(&requests);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker = thread::spawn(move || {
            while !worker_shutdown.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        serve_request(&mut stream, &routes, &worker_requests);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("updater fixture server failed: {error}"),
                }
            }
        });

        Self {
            address,
            requests,
            shutdown,
            worker: Some(worker),
        }
    }

    fn endpoint(&self) -> String {
        format!("http://{}/latest.json", self.address)
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("read fixture requests").clone()
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(&self.address);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("join updater fixture server");
        }
    }
}

fn serve_request(
    stream: &mut TcpStream,
    routes: &HashMap<String, FixtureResponse>,
    requests: &Mutex<Vec<String>>,
) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set fixture read timeout");
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 2048];
    while raw.len() < 16 * 1024 && !raw.ends_with(b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => raw.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => break,
            Err(error) => panic!("failed to read updater fixture request: {error}"),
        }
    }

    let request = String::from_utf8_lossy(&raw);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/")
        .to_string();
    requests
        .lock()
        .expect("record fixture request")
        .push(path.clone());
    let response = routes
        .get(&path)
        .cloned()
        .unwrap_or_else(|| FixtureResponse::status(404, "Not Found"));
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.reason,
        response.content_type,
        response.body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .and_then(|_| stream.write_all(&response.body))
        .expect("write updater fixture response");
}

fn manifest(version: &str, target: &str, payload_url: &str, signature: &str) -> String {
    let mut platforms = Map::new();
    platforms.insert(
        target.to_string(),
        json!({
            "url": payload_url,
            "signature": signature,
        }),
    );
    json!({
        "version": version,
        "platforms": Value::Object(platforms),
    })
    .to_string()
}

fn updater_app(endpoint: &str, public_key: &str) -> tauri::App<MockRuntime> {
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().plugins.0.insert(
        "updater".to_string(),
        json!({
            "pubkey": public_key,
            "endpoints": [endpoint],
        }),
    );
    tauri::test::mock_builder()
        .plugin(
            tauri_plugin_updater::Builder::new()
                .target(FIXTURE_TARGET)
                .build(),
        )
        .build(context)
        .expect("build updater fixture app")
}

fn check(app: &tauri::App<MockRuntime>) -> tauri_plugin_updater::Result<Option<Update>> {
    tauri::async_runtime::block_on(async {
        app.updater_builder().no_proxy().build()?.check().await
    })
}

fn download(update: &Update) -> tauri_plugin_updater::Result<Vec<u8>> {
    tauri::async_runtime::block_on(update.download(|_, _| {}, || {}))
}

#[test]
fn unavailable_missing_malformed_empty_and_wrong_target_metadata_fail_closed() {
    let cases = [
        ("unavailable", FixtureResponse::status(503, "Unavailable")),
        ("missing", FixtureResponse::status(404, "Not Found")),
        ("malformed", FixtureResponse::json(b"{not-json".to_vec())),
        ("empty", FixtureResponse::json(Vec::new())),
    ];

    for (name, metadata) in cases {
        let server =
            FixtureServer::start(|_| HashMap::from([("/latest.json".to_string(), metadata)]));
        let app = updater_app(&server.endpoint(), FIXTURE_PUBLIC_KEY);
        assert!(check(&app).is_err(), "{name} metadata must fail the check");
        assert_eq!(
            server.requests(),
            vec!["/latest.json"],
            "{name} metadata must not start a payload request"
        );
    }

    let server = FixtureServer::start(|base_url| {
        HashMap::from([
            (
                "/latest.json".to_string(),
                FixtureResponse::json(manifest(
                    "0.1.1",
                    "other-target",
                    &format!("{base_url}/payload.bin"),
                    FIXTURE_SIGNATURE,
                )),
            ),
            (
                "/payload.bin".to_string(),
                FixtureResponse::bytes(FIXTURE_PAYLOAD.to_vec()),
            ),
        ])
    });
    let app = updater_app(&server.endpoint(), FIXTURE_PUBLIC_KEY);
    assert!(check(&app).is_err(), "wrong-target metadata must fail");
    assert_eq!(server.requests(), vec!["/latest.json"]);
}

#[test]
fn no_content_and_equal_or_lower_versions_create_no_update_resource() {
    let no_content = FixtureServer::start(|_| {
        HashMap::from([(
            "/latest.json".to_string(),
            FixtureResponse::status(204, "No Content"),
        )])
    });
    let app = updater_app(&no_content.endpoint(), FIXTURE_PUBLIC_KEY);
    assert!(check(&app).expect("204 check succeeds").is_none());
    assert_eq!(no_content.requests(), vec!["/latest.json"]);

    for version in ["0.1.0", "0.0.9"] {
        let server = FixtureServer::start(|base_url| {
            HashMap::from([
                (
                    "/latest.json".to_string(),
                    FixtureResponse::json(manifest(
                        version,
                        FIXTURE_TARGET,
                        &format!("{base_url}/payload.bin"),
                        FIXTURE_SIGNATURE,
                    )),
                ),
                (
                    "/payload.bin".to_string(),
                    FixtureResponse::bytes(FIXTURE_PAYLOAD.to_vec()),
                ),
            ])
        });
        let app = updater_app(&server.endpoint(), FIXTURE_PUBLIC_KEY);
        assert_eq!(app.package_info().version.to_string(), "0.1.0");
        assert!(
            check(&app)
                .unwrap_or_else(|error| panic!("{version} check failed: {error}"))
                .is_none(),
            "{version} must not be offered over 0.1.0"
        );
        assert_eq!(server.requests(), vec!["/latest.json"]);
    }
}

#[test]
fn valid_update_reaches_real_download_verification_and_resource_close_boundary() {
    let server = FixtureServer::start(|base_url| {
        HashMap::from([
            (
                "/latest.json".to_string(),
                FixtureResponse::json(manifest(
                    "0.1.1",
                    FIXTURE_TARGET,
                    &format!("{base_url}/payload.bin"),
                    FIXTURE_SIGNATURE,
                )),
            ),
            (
                "/payload.bin".to_string(),
                FixtureResponse::bytes(FIXTURE_PAYLOAD.to_vec()),
            ),
        ])
    });
    let app = updater_app(&server.endpoint(), FIXTURE_PUBLIC_KEY);
    let update = check(&app)
        .expect("valid metadata check succeeds")
        .expect("forward update is offered");

    assert_eq!(
        download(&update).expect("valid payload verifies"),
        FIXTURE_PAYLOAD
    );
    assert_eq!(server.requests(), vec!["/latest.json", "/payload.bin"]);

    let webview = tauri::WebviewWindowBuilder::new(&app, "fixture", Default::default())
        .build()
        .expect("build fixture webview");
    let resource_id = webview.resources_table().add(update);
    assert!(webview.resources_table().get::<Update>(resource_id).is_ok());
    webview
        .resources_table()
        .close(resource_id)
        .expect("close selected update resource");
    assert!(webview
        .resources_table()
        .get::<Update>(resource_id)
        .is_err());
    assert!(webview.resources_table().close(resource_id).is_err());
}

#[test]
fn wrong_key_invalid_signature_and_tampered_bytes_fail_before_installation() {
    let cases = [
        (
            "wrong key",
            UNRELATED_PUBLIC_KEY,
            FIXTURE_SIGNATURE,
            FIXTURE_PAYLOAD.to_vec(),
        ),
        (
            "invalid signature",
            FIXTURE_PUBLIC_KEY,
            INVALID_SIGNATURE,
            FIXTURE_PAYLOAD.to_vec(),
        ),
        (
            "tampered bytes",
            FIXTURE_PUBLIC_KEY,
            FIXTURE_SIGNATURE,
            b"test-tampered".to_vec(),
        ),
    ];

    for (name, public_key, signature, payload) in cases {
        let server = FixtureServer::start(|base_url| {
            HashMap::from([
                (
                    "/latest.json".to_string(),
                    FixtureResponse::json(manifest(
                        "0.1.1",
                        FIXTURE_TARGET,
                        &format!("{base_url}/payload.bin"),
                        signature,
                    )),
                ),
                ("/payload.bin".to_string(), FixtureResponse::bytes(payload)),
            ])
        });
        let app = updater_app(&server.endpoint(), public_key);
        let update = check(&app)
            .unwrap_or_else(|error| panic!("{name} metadata check failed: {error}"))
            .expect("forward update metadata is selected before payload verification");

        assert!(
            download(&update).is_err(),
            "{name} payload must be rejected"
        );
        assert_eq!(server.requests(), vec!["/latest.json", "/payload.bin"]);
    }
}

#[test]
fn unreachable_loopback_endpoint_is_an_ordinary_check_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve offline fixture port");
    let endpoint = format!(
        "http://{}/latest.json",
        listener.local_addr().expect("read offline fixture address")
    );
    drop(listener);

    let app = updater_app(&endpoint, FIXTURE_PUBLIC_KEY);
    assert!(check(&app).is_err());
}
