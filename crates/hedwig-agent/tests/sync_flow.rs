use hedwig_agent::sync::{SyncConfig, SyncMode, synchronize};
use hedwig_client::CloudClient;
use hedwig_server::{AppConfig, build_app};
use serde_json::{Value, json};
use std::io::Write;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tempfile::TempDir;
use tokio::net::TcpListener;

async fn meter_block_uploads(
    axum::extract::State(counter): axum::extract::State<Arc<AtomicU64>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if request.method() == axum::http::Method::PUT
        && request.uri().path().contains("/blocks")
        && let Some(length) = request
            .headers()
            .get("x-hedwig-content-bytes")
            .or_else(|| request.headers().get(axum::http::header::CONTENT_LENGTH))
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
    {
        counter.fetch_add(length, Ordering::Relaxed);
    }
    next.run(request).await
}

async fn server() -> (String, String, String, TempDir) {
    let data = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(data.path().to_path_buf()))
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{address}/");
    let response=reqwest::Client::new().post(format!("{base}v1/setup")).json(&json!({
        "username":"sync-user","password":"correct horse battery staple","device_name":"Integration"
    })).send().await.unwrap();
    assert!(response.status().is_success());
    let setup: Value = response.json().await.unwrap();
    (
        base,
        setup["token"].as_str().unwrap().into(),
        setup["personal_space_id"].as_str().unwrap().into(),
        data,
    )
}

#[tokio::test]
async fn client_logout_revokes_the_current_session() {
    let (base, token, _space, _server_data) = server().await;
    let client = CloudClient::new(&base, token).unwrap();
    assert!(!client.spaces().await.unwrap().is_empty());

    client.logout().await.unwrap();

    let error = client.spaces().await.unwrap_err();
    assert!(
        error.is_unauthorized(),
        "abgemeldetes Token blieb gültig: {error}"
    );
}

async fn metered_server() -> (String, String, String, Arc<AtomicU64>, TempDir) {
    let data = TempDir::new().unwrap();
    let counter = Arc::new(AtomicU64::new(0));
    let app = build_app(AppConfig::new(data.path().to_path_buf()))
        .await
        .unwrap()
        .layer(axum::middleware::from_fn_with_state(
            counter.clone(),
            meter_block_uploads,
        ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{address}/");
    let response = reqwest::Client::new()
        .post(format!("{base}v1/setup"))
        .json(&json!({
            "username":"sync-user","password":"correct horse battery staple","device_name":"Integration"
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let setup: Value = response.json().await.unwrap();
    (
        base,
        setup["token"].as_str().unwrap().into(),
        setup["personal_space_id"].as_str().unwrap().into(),
        counter,
        data,
    )
}

fn config(space: &str, root: &TempDir, state: &TempDir) -> SyncConfig {
    SyncConfig {
        space_id: space.into(),
        root: root.path().to_path_buf(),
        state_dir: state.path().to_path_buf(),
        mode: SyncMode::TwoWay,
        delete_limit: 25,
        delete_percent: 20,
        allow_deletions_once: false,
        selective_excluded: Vec::new(),
        parallel_transfers: 16,
    }
}

#[tokio::test]
async fn two_devices_upload_download_and_preserve_conflict() {
    let (base, token, space, _server_data) = server().await;
    let first_root = TempDir::new().unwrap();
    let first_state = TempDir::new().unwrap();
    let second_root = TempDir::new().unwrap();
    let second_state = TempDir::new().unwrap();
    std::fs::write(first_root.path().join("Notiz.txt"), b"erste Fassung").unwrap();
    std::fs::write(
        first_root.path().join("debug.log"),
        b"wertvoll bis ignoriert",
    )
    .unwrap();
    std::fs::write(first_root.path().join(".hedwigignore"), b"*.log\n").unwrap();

    let first = CloudClient::new(&base, token.clone()).unwrap();
    let report = synchronize(first, config(&space, &first_root, &first_state))
        .await
        .unwrap();
    assert_eq!(
        report.uploaded, 2,
        "Datei und .hedwigignore werden übertragen"
    );
    assert_eq!(report.excluded, 1);

    let second = CloudClient::new(&base, token.clone()).unwrap();
    let report = synchronize(second, config(&space, &second_root, &second_state))
        .await
        .unwrap();
    assert_eq!(report.downloaded, 2);
    assert_eq!(
        std::fs::read(second_root.path().join("Notiz.txt")).unwrap(),
        b"erste Fassung"
    );
    assert!(!second_root.path().join("debug.log").exists());

    std::fs::write(
        second_root.path().join("Notiz.txt"),
        b"Aenderung von Geraet B",
    )
    .unwrap();
    let second = CloudClient::new(&base, token.clone()).unwrap();
    let report = synchronize(second, config(&space, &second_root, &second_state))
        .await
        .unwrap();
    assert_eq!(report.uploaded, 1);

    std::fs::write(
        first_root.path().join("Notiz.txt"),
        b"gleichzeitige Aenderung von A",
    )
    .unwrap();
    let first = CloudClient::new(&base, token).unwrap();
    let report = synchronize(first, config(&space, &first_root, &first_state))
        .await
        .unwrap();
    assert_eq!(report.conflicts, 1);
    assert_eq!(report.downloaded, 1);
    assert_eq!(
        std::fs::read(first_root.path().join("Notiz.txt")).unwrap(),
        b"Aenderung von Geraet B"
    );
    let conflict = std::fs::read_dir(first_root.path())
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().contains("Konflikt"))
        .expect("Konfliktkopie fehlt");
    assert_eq!(
        std::fs::read(conflict.path()).unwrap(),
        b"gleichzeitige Aenderung von A"
    );
}

#[tokio::test]
async fn local_rename_moves_remote_identity_without_uploading_content() {
    let (base, token, space, _server_data) = server().await;
    let root = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    std::fs::write(root.path().join("Vorher.txt"), b"unveraenderter Inhalt").unwrap();
    let client = CloudClient::new(&base, token.clone()).unwrap();
    let report = synchronize(client, config(&space, &root, &state))
        .await
        .unwrap();
    assert_eq!(report.uploaded, 1);
    let before = CloudClient::new(&base, token.clone())
        .unwrap()
        .files(&space, false)
        .await
        .unwrap()
        .pop()
        .unwrap();

    std::fs::rename(
        root.path().join("Vorher.txt"),
        root.path().join("Nachher.txt"),
    )
    .unwrap();
    let client = CloudClient::new(&base, token.clone()).unwrap();
    let report = synchronize(client, config(&space, &root, &state))
        .await
        .unwrap();
    assert_eq!(report.renamed_remote, 1);
    assert_eq!(report.uploaded, 0, "Rename darf keinen Inhalt hochladen");
    assert_eq!(report.removed_remote, 0);

    let after = CloudClient::new(&base, token)
        .unwrap()
        .files(&space, false)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(after.id, before.id);
    assert_eq!(after.current_version_id, before.current_version_id);
    assert_eq!(after.path, "Nachher.txt");
}

#[tokio::test]
async fn simultaneous_different_renames_preserve_both_names() {
    let (base, token, space, _server_data) = server().await;
    let first_root = TempDir::new().unwrap();
    let first_state = TempDir::new().unwrap();
    let second_root = TempDir::new().unwrap();
    let second_state = TempDir::new().unwrap();
    std::fs::write(first_root.path().join("Ausgang.txt"), b"gleicher Inhalt").unwrap();
    synchronize(
        CloudClient::new(&base, token.clone()).unwrap(),
        config(&space, &first_root, &first_state),
    )
    .await
    .unwrap();
    synchronize(
        CloudClient::new(&base, token.clone()).unwrap(),
        config(&space, &second_root, &second_state),
    )
    .await
    .unwrap();

    std::fs::rename(
        second_root.path().join("Ausgang.txt"),
        second_root.path().join("Name B.txt"),
    )
    .unwrap();
    synchronize(
        CloudClient::new(&base, token.clone()).unwrap(),
        config(&space, &second_root, &second_state),
    )
    .await
    .unwrap();

    std::fs::rename(
        first_root.path().join("Ausgang.txt"),
        first_root.path().join("Name A.txt"),
    )
    .unwrap();
    let report = synchronize(
        CloudClient::new(&base, token.clone()).unwrap(),
        config(&space, &first_root, &first_state),
    )
    .await
    .unwrap();
    assert!(report.conflicts >= 1);
    assert!(first_root.path().join("Name A.txt").exists());
    assert!(first_root.path().join("Name B.txt").exists());
    let mut paths: Vec<_> = CloudClient::new(&base, token)
        .unwrap()
        .files(&space, false)
        .await
        .unwrap()
        .into_iter()
        .map(|file| file.path)
        .collect();
    paths.sort();
    assert_eq!(paths, vec!["Name A.txt", "Name B.txt"]);
}

#[tokio::test]
async fn suspicious_local_deletion_pauses_remote_deletes() {
    let (base, token, space, _server_data) = server().await;
    let root = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    for number in 0..5 {
        std::fs::write(
            root.path().join(format!("Datei-{number}.txt")),
            format!("Inhalt {number}"),
        )
        .unwrap();
    }
    let client = CloudClient::new(&base, token.clone()).unwrap();
    let mut initial = config(&space, &root, &state);
    initial.delete_limit = 1;
    initial.delete_percent = 20;
    assert_eq!(
        synchronize(client, initial.clone()).await.unwrap().uploaded,
        5
    );

    std::fs::remove_file(root.path().join("Datei-0.txt")).unwrap();
    std::fs::remove_file(root.path().join("Datei-1.txt")).unwrap();
    let client = CloudClient::new(&base, token.clone()).unwrap();
    let report = synchronize(client, initial).await.unwrap();
    assert!(report.deletion_pause);
    assert_eq!(report.pending_deletions, 2);
    assert_eq!(report.removed_remote, 0);

    let client = CloudClient::new(&base, token.clone()).unwrap();
    let mut confirmed = config(&space, &root, &state);
    confirmed.delete_limit = 1;
    confirmed.delete_percent = 20;
    confirmed.allow_deletions_once = true;
    let report = synchronize(client, confirmed).await.unwrap();
    assert!(!report.deletion_pause);
    assert_eq!(report.removed_remote, 2);

    let other_root = TempDir::new().unwrap();
    let other_state = TempDir::new().unwrap();
    let client = CloudClient::new(&base, token).unwrap();
    let report = synchronize(client, config(&space, &other_root, &other_state))
        .await
        .unwrap();
    assert_eq!(
        report.downloaded, 3,
        "Nur die ausdrücklich bestätigte Löschserie darf angewendet werden"
    );
}

#[tokio::test]
async fn selective_sync_preserves_local_work_and_never_deletes_remote_files() {
    let (base, token, space, _server_data) = server().await;
    let source_root = TempDir::new().unwrap();
    let source_state = TempDir::new().unwrap();
    let selective_root = TempDir::new().unwrap();
    let selective_state = TempDir::new().unwrap();
    std::fs::create_dir_all(source_root.path().join("Archiv")).unwrap();
    std::fs::write(source_root.path().join("Normal.txt"), b"normal").unwrap();
    std::fs::write(source_root.path().join("Archiv/A.txt"), b"archiviert").unwrap();

    let uploaded = synchronize(
        CloudClient::new(&base, token.clone()).unwrap(),
        config(&space, &source_root, &source_state),
    )
    .await
    .unwrap();
    assert_eq!(uploaded.uploaded, 2);

    let mut selected = config(&space, &selective_root, &selective_state);
    selected.selective_excluded = vec!["Archiv".into()];
    let report = synchronize(
        CloudClient::new(&base, token.clone()).unwrap(),
        selected.clone(),
    )
    .await
    .unwrap();
    assert_eq!(report.downloaded, 1);
    assert!(selective_root.path().join("Normal.txt").exists());
    assert!(!selective_root.path().join("Archiv/A.txt").exists());

    std::fs::create_dir_all(selective_root.path().join("Archiv")).unwrap();
    std::fs::write(
        selective_root.path().join("Archiv/lokale-Arbeit.txt"),
        b"noch nicht uebertragen",
    )
    .unwrap();
    let report = synchronize(CloudClient::new(&base, token.clone()).unwrap(), selected)
        .await
        .unwrap();
    assert_eq!(report.selective_preserved, 1);
    assert!(
        !selective_root
            .path()
            .join("Archiv/lokale-Arbeit.txt")
            .exists()
    );
    let preserved = walkdir::WalkDir::new(selective_state.path().join("selective-preserved"))
        .into_iter()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name() == "lokale-Arbeit.txt");
    assert!(
        preserved,
        "lokale Datei muss außerhalb des Sync-Ordners erhalten bleiben"
    );

    let remote_paths: Vec<_> = CloudClient::new(&base, token.clone())
        .unwrap()
        .files(&space, false)
        .await
        .unwrap()
        .into_iter()
        .map(|file| file.path)
        .collect();
    assert!(remote_paths.contains(&"Archiv/A.txt".to_string()));
    assert!(!remote_paths.contains(&"Archiv/lokale-Arbeit.txt".to_string()));

    let report = synchronize(
        CloudClient::new(&base, token).unwrap(),
        config(&space, &selective_root, &selective_state),
    )
    .await
    .unwrap();
    assert_eq!(report.downloaded, 1);
    assert_eq!(report.removed_remote, 0);
    assert_eq!(
        std::fs::read(selective_root.path().join("Archiv/A.txt")).unwrap(),
        b"archiviert"
    );
}

#[tokio::test]
async fn large_edit_transfers_only_changed_chunks_and_rename_transfers_zero_content_bytes() {
    let (base, token, space, transferred, _server_data) = metered_server().await;
    let root = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let path = root.path().join("Gross.bin");
    let mut content = vec![0_u8; 8 * 1024 * 1024];
    let mut value = 0x1234_5678_u32;
    for byte in &mut content {
        value = value.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *byte = (value >> 24) as u8;
    }
    std::fs::write(&path, &content).unwrap();

    let report = synchronize(
        CloudClient::new(&base, token.clone()).unwrap(),
        config(&space, &root, &state),
    )
    .await
    .unwrap();
    assert_eq!(report.uploaded, 1);
    let initial_bytes = transferred.swap(0, Ordering::Relaxed);
    assert_eq!(initial_bytes, content.len() as u64);

    for byte in &mut content[4 * 1024 * 1024..4 * 1024 * 1024 + 32 * 1024] {
        *byte ^= 0x5a;
    }
    std::fs::write(&path, &content).unwrap();
    let report = synchronize(
        CloudClient::new(&base, token.clone()).unwrap(),
        config(&space, &root, &state),
    )
    .await
    .unwrap();
    assert_eq!(report.uploaded, 1);
    let edit_bytes = transferred.swap(0, Ordering::Relaxed);
    println!(
        "gemessene Inhaltsbytes: Erstupload={initial_bytes}, kleine Änderung={edit_bytes}, Rename=0"
    );
    assert!(edit_bytes > 0);
    assert!(
        edit_bytes < initial_bytes / 2,
        "kleine Änderung übertrug {edit_bytes} von ursprünglich {initial_bytes} Inhaltsbytes"
    );

    std::fs::rename(&path, root.path().join("Umbenannt.bin")).unwrap();
    let report = synchronize(
        CloudClient::new(&base, token).unwrap(),
        config(&space, &root, &state),
    )
    .await
    .unwrap();
    assert_eq!(report.renamed_remote, 1);
    assert_eq!(report.uploaded, 0);
    assert_eq!(transferred.load(Ordering::Relaxed), 0);
}

#[tokio::test]
#[ignore = "manueller Lastlauf mit 2000 kleinen Dateien und 128 MiB"]
async fn benchmark_many_small_and_large_files() {
    let (base, token, space, transferred, _server_data) = metered_server().await;
    let source = TempDir::new().unwrap();
    let source_state = TempDir::new().unwrap();
    let target = TempDir::new().unwrap();
    let target_state = TempDir::new().unwrap();
    let small = source.path().join("klein");
    std::fs::create_dir(&small).unwrap();
    for number in 0..2_000 {
        std::fs::write(
            small.join(format!("Datei-{number:04}.txt")),
            format!("kleine Testdatei {number:04}\n").repeat(32),
        )
        .unwrap();
    }
    let large_path = source.path().join("Gross-128MiB.bin");
    let mut large = std::io::BufWriter::new(std::fs::File::create(&large_path).unwrap());
    let mut block = vec![0_u8; 1024 * 1024];
    let mut value = 0x8f13_ab77_u32;
    for _ in 0..128 {
        for byte in &mut block {
            value = value.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *byte = (value >> 24) as u8;
        }
        large.write_all(&block).unwrap();
    }
    large.flush().unwrap();
    drop(large);

    let upload_started = std::time::Instant::now();
    let uploaded = synchronize(
        CloudClient::new(&base, token.clone()).unwrap(),
        config(&space, &source, &source_state),
    )
    .await
    .unwrap();
    let upload_elapsed = upload_started.elapsed();
    let payload_bytes = transferred.swap(0, Ordering::Relaxed);

    let download_started = std::time::Instant::now();
    let downloaded = synchronize(
        CloudClient::new(&base, token).unwrap(),
        config(&space, &target, &target_state),
    )
    .await
    .unwrap();
    let download_elapsed = download_started.elapsed();
    assert_eq!(uploaded.uploaded, 2_001);
    assert_eq!(downloaded.downloaded, 2_001);
    assert_eq!(
        std::fs::metadata(target.path().join("Gross-128MiB.bin"))
            .unwrap()
            .len(),
        128 * 1024 * 1024
    );
    println!(
        "LASTMESSUNG small_files=2000 large_bytes={} upload_payload_bytes={payload_bytes} upload_seconds={:.3} download_seconds={:.3}",
        128_u64 * 1024 * 1024,
        upload_elapsed.as_secs_f64(),
        download_elapsed.as_secs_f64(),
    );
}
