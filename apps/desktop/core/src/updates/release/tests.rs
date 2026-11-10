//! Tests of the release source. What the release script signs verifies with
//! the crate that the updater plugin verifies with, and the plugin's own
//! check, download and verification accept a release of the script and refuse
//! a changed one. The plugin runs on Tauri's mock runtime and reads a server
//! on this computer. No test installs anything.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use mannie_types::updates::UpdateFailureCode;
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use serde_json::json;
use tauri::test::MockRuntime;

use super::super::source::Source;
use super::{Installation, ReleaseSource, target};

/// The signed release in `tests/fixtures/updates/release.json`. The test of
/// the release script keeps the file equal to what the script signs
/// (`tests/scripts/release.test.mjs`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignedRelease {
    /// The installer's file name, which the signature names.
    file: String,
    /// The installer's content.
    installer: String,
    /// The installer's signature in minisign's legacy format, as a manifest
    /// would hold it.
    legacy_signature: String,
    /// The key as the app's configuration holds it.
    public_key: String,
    /// The installer's signature as the update manifest holds it.
    signature: String,
    version: String,
}

fn signed_release() -> SignedRelease {
    let text = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/updates/release.json"
    ));
    serde_json::from_str(text).unwrap()
}

/// The text that the configuration and the manifest hold as base64.
fn decoded(text: &str) -> String {
    String::from_utf8(STANDARD.decode(text).unwrap()).unwrap()
}

fn public_key(release: &SignedRelease) -> PublicKey {
    PublicKey::decode(&decoded(&release.public_key)).unwrap()
}

#[test]
fn a_signature_of_the_release_script_verifies() {
    let release = signed_release();
    let signature = Signature::decode(&decoded(&release.signature)).unwrap();

    public_key(&release)
        .verify(release.installer.as_bytes(), &signature, false)
        .unwrap();

    assert_eq!(
        signature.trusted_comment(),
        format!(
            "timestamp:1794000000\tfile:{}\tversion:{}",
            release.file, release.version
        )
    );
}

#[test]
fn a_changed_installer_does_not_verify() {
    let release = signed_release();
    let signature = Signature::decode(&decoded(&release.signature)).unwrap();
    let changed = format!("{}!", release.installer);

    public_key(&release)
        .verify(changed.as_bytes(), &signature, false)
        .unwrap_err();
}

#[test]
fn a_changed_trusted_comment_does_not_verify() {
    let release = signed_release();
    let signed = decoded(&release.signature);
    let changed = signed.replace(&format!("version:{}", release.version), "version:1.2.4");
    assert_ne!(changed, signed);
    let signature = Signature::decode(&changed).unwrap();

    public_key(&release)
        .verify(release.installer.as_bytes(), &signature, false)
        .unwrap_err();
}

#[test]
fn a_signature_in_the_legacy_format_does_not_verify() {
    let release = signed_release();
    let key = public_key(&release);
    let legacy = Signature::decode(&decoded(&release.legacy_signature)).unwrap();

    // The signature is a good one of its format, so only the format is what
    // the second call refuses.
    key.verify(release.installer.as_bytes(), &legacy, true)
        .unwrap();
    key.verify(release.installer.as_bytes(), &legacy, false)
        .unwrap_err();
}

/// The version of the app that the tests check from.
const RUNNING: &str = "1.0.0";

/// Where the test's server holds the manifest, as GitHub does for the latest
/// release.
const MANIFEST_PATH: &str = "/releases/latest/download/latest.json";

/// A server on this computer that answers as GitHub's releases do.
struct Server {
    listener: TcpListener,
    origin: String,
}

impl Server {
    fn bind() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        Self { listener, origin }
    }

    /// Answers a request for one of the paths with its content and every
    /// other request with "not found". The thread ends with the process of
    /// the tests.
    fn serve(self, files: HashMap<String, Vec<u8>>) {
        std::thread::spawn(move || {
            for mut stream in self.listener.incoming().flatten() {
                // A request that got no answer fails the check or the
                // download of its test.
                let _answered = answer(&mut stream, &files);
            }
        });
    }
}

/// Reads one request and answers it.
fn answer(stream: &mut TcpStream, files: &HashMap<String, Vec<u8>>) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut part = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let length = stream.read(&mut part)?;
        if length == 0 {
            return Ok(());
        }
        request.extend_from_slice(&part[..length]);
    }
    let request = String::from_utf8_lossy(&request);
    let path = request.split(' ').nth(1).unwrap_or_default();
    match files.get(path) {
        Some(content) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                content.len()
            )?;
            stream.write_all(content)
        }
        None => stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
    }
}

/// An app on Tauri's mock runtime with the updater plugin, registered as
/// `run` registers it, and with a key in its configuration.
fn app(key: &str) -> tauri::App<MockRuntime> {
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.package_info_mut().version = RUNNING.parse().unwrap();
    context.config_mut().plugins.0.insert(
        "updater".to_owned(),
        json!({
            // The test's server answers over plain HTTP, which the plugin
            // refuses in a release build without this.
            "dangerousInsecureTransportProtocol": true,
            "pubkey": key,
            "requireSignedVersion": true,
        }),
    );
    tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap()
}

/// The source of an installation of the Windows installer's kind that reads
/// the server at an address.
fn source(app: &tauri::App<MockRuntime>, origin: &str) -> ReleaseSource<MockRuntime> {
    ReleaseSource {
        app: app.handle().clone(),
        installation: Installation::Nsis,
        manifest: format!("{origin}{MANIFEST_PATH}"),
        downloads: format!("{origin}/releases/download/"),
        close_log: Arc::new(|| {}),
    }
}

/// A source with the app that it runs in.
struct Reading {
    _app: tauri::App<MockRuntime>,
    source: ReleaseSource<MockRuntime>,
}

/// Starts the server with its files and gives the source that reads it.
fn reading(server: Server, key: &str, files: HashMap<String, Vec<u8>>) -> Reading {
    let app = app(key);
    let source = source(&app, &server.origin);
    server.serve(files);
    Reading { _app: app, source }
}

/// The fixture's release as the test's server publishes it. A test changes a
/// part of it before the server starts.
struct Published {
    server: Server,
    /// The key in the app's configuration.
    key: String,
    /// The version that the manifest announces.
    version: String,
    /// The manifest's key of the package.
    platform: String,
    /// Where the manifest says that the package downloads from.
    download: String,
    /// The package's signature in the manifest.
    signature: String,
    /// Where the server holds the installer.
    installer_path: String,
    /// The installer that the server holds.
    installer: Vec<u8>,
}

impl Published {
    fn of(release: &SignedRelease) -> Self {
        let server = Server::bind();
        let installer_path = format!("/releases/download/v{}/{}", release.version, release.file);
        Self {
            key: release.public_key.clone(),
            version: release.version.clone(),
            platform: target(Installation::Nsis).unwrap(),
            download: format!("{}{installer_path}", server.origin),
            signature: release.signature.clone(),
            installer_path,
            installer: release.installer.clone().into_bytes(),
            server,
        }
    }

    /// The manifest with the keys that the release script writes.
    fn manifest(&self) -> Vec<u8> {
        let manifest = json!({
            "notes": "What changed in this version.",
            "platforms": {
                (self.platform.as_str()): {
                    "signature": self.signature,
                    "url": self.download,
                },
            },
            "pub_date": "2026-11-10T00:00:00.000Z",
            "version": self.version,
        });
        manifest.to_string().into_bytes()
    }

    fn read(self) -> Reading {
        let files = HashMap::from([
            (MANIFEST_PATH.to_owned(), self.manifest()),
            (self.installer_path, self.installer),
        ]);
        reading(self.server, &self.key, files)
    }
}

#[tokio::test]
async fn the_updater_accepts_a_release_of_the_release_script() {
    let release = signed_release();
    let reading = Published::of(&release).read();

    let offer = reading.source.check().await.unwrap().unwrap();
    assert_eq!(offer.version(), release.version);

    let received = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&received);
    let package = offer
        .download(Box::new(move |part, _total| {
            counted.fetch_add(part, Ordering::Relaxed);
        }))
        .await
        .unwrap();

    assert_eq!(package.version(), release.version);
    assert_eq!(received.load(Ordering::Relaxed), release.installer.len());
}

#[tokio::test]
async fn the_updater_refuses_a_changed_installer() {
    let mut published = Published::of(&signed_release());
    published.installer.push(b'!');
    let reading = published.read();

    let offer = reading.source.check().await.unwrap().unwrap();
    let failure = offer.download(Box::new(|_part, _total| {})).await.err();

    assert_eq!(failure, Some(UpdateFailureCode::SignatureInvalid));
}

#[tokio::test]
async fn the_updater_refuses_a_signature_of_another_version() {
    let mut published = Published::of(&signed_release());
    "1.2.4".clone_into(&mut published.version);
    let reading = published.read();

    let offer = reading.source.check().await.unwrap().unwrap();
    assert_eq!(offer.version(), "1.2.4");
    let failure = offer.download(Box::new(|_part, _total| {})).await.err();

    assert_eq!(failure, Some(UpdateFailureCode::SignatureInvalid));
}

#[tokio::test]
async fn a_download_outside_of_the_releases_is_refused() {
    let release = signed_release();
    let mut published = Published::of(&release);
    published.download = format!("{}/elsewhere/{}", published.server.origin, release.file);
    let reading = published.read();

    let failure = reading.source.check().await.err();

    assert_eq!(failure, Some(UpdateFailureCode::NoUpdateInformation));
}

#[tokio::test]
async fn the_running_version_is_no_update() {
    let mut published = Published::of(&signed_release());
    RUNNING.clone_into(&mut published.version);
    let reading = published.read();

    let found = reading.source.check().await.unwrap();

    assert!(found.is_none());
}

#[tokio::test]
async fn an_older_version_is_no_update() {
    let mut published = Published::of(&signed_release());
    "0.9.0".clone_into(&mut published.version);
    let reading = published.read();

    let found = reading.source.check().await.unwrap();

    assert!(found.is_none());
}

#[tokio::test]
async fn a_release_without_a_manifest_has_no_update_information() {
    let release = signed_release();
    let reading = reading(Server::bind(), &release.public_key, HashMap::new());

    let failure = reading.source.check().await.err();

    assert_eq!(failure, Some(UpdateFailureCode::NoUpdateInformation));
}

#[tokio::test]
async fn a_page_in_place_of_the_manifest_has_no_update_information() {
    let release = signed_release();
    let page = b"<!doctype html><title>Sign in to the network</title>".to_vec();
    let files = HashMap::from([(MANIFEST_PATH.to_owned(), page)]);
    let reading = reading(Server::bind(), &release.public_key, files);

    let failure = reading.source.check().await.err();

    assert_eq!(failure, Some(UpdateFailureCode::NoUpdateInformation));
}

#[tokio::test]
async fn a_manifest_without_this_installation_has_no_package() {
    let mut published = Published::of(&signed_release());
    "another-system".clone_into(&mut published.platform);
    let reading = published.read();

    let failure = reading.source.check().await.err();

    assert_eq!(
        failure,
        Some(UpdateFailureCode::NoPackageForThisInstallation)
    );
}

#[tokio::test]
async fn a_server_that_does_not_answer_is_unreachable() {
    let release = signed_release();
    let server = Server::bind();
    let origin = server.origin.clone();
    // Nothing listens at the address from here on.
    drop(server);
    let app = app(&release.public_key);
    let source = source(&app, &origin);

    let failure = source.check().await.err();

    assert_eq!(failure, Some(UpdateFailureCode::Unreachable));
}

/// The capability files of the app: those in the folder and in its folders.
fn capability_files(folder: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        let is_json = path
            .extension()
            .is_some_and(|extension| extension == "json");
        if path.is_dir() {
            capability_files(&path, found);
        } else if is_json {
            found.push(path);
        }
    }
}

/// The plugin is registered without a capability, so the webview calls none
/// of its commands and reaches updates only through the core's own commands
/// (ADR-0063, decision 2).
#[test]
fn no_capability_grants_a_permission_of_the_updater() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src-tauri/capabilities");
    let mut files = Vec::new();
    capability_files(&folder, &mut files);
    assert!(
        !files.is_empty(),
        "{} holds no capability",
        folder.display()
    );

    for file in files {
        let capability: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        let permissions = capability["permissions"].as_array().unwrap();
        assert!(!permissions.is_empty(), "{} grants nothing", file.display());

        for permission in permissions {
            // A permission is its name, or an object that adds a scope to it.
            let name = permission
                .as_str()
                .or_else(|| permission["identifier"].as_str())
                .unwrap();
            assert!(
                !name.starts_with("updater:"),
                "{} grants {name}",
                file.display()
            );
        }
    }
}
