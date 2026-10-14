use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use hedwig_server::{
    AppConfig, BlockStore, Database, build_app, create_backup, deliver_pending_webhooks,
    export_directory_tree, restore_backup, run_maintenance, run_scheduled_backup, verify_backup,
};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::Sha256;
use tempfile::TempDir;
use tower::ServiceExt;

async fn json_response(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn json_request(method: &str, path: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

fn share_request(
    method: &str,
    path: &str,
    token: &str,
    password: Option<&str>,
    body: Value,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Share {token}"));
    if let Some(password) = password {
        builder = builder.header("x-share-password", password);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

async fn upload_file(
    app: &axum::Router,
    token: &str,
    space_id: &str,
    path: &str,
    content: &[u8],
    key: &str,
) -> Value {
    let hash = blake3::hash(content).to_hex().to_string();
    let start = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/v1/spaces/{space_id}/uploads"),
            Some(token),
            json!({
                "path":path,"size":content.len(),"blocks":[{"hash":hash,"size":content.len()}],
                "idempotency_key":key
            }),
        ))
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::CREATED);
    let upload_id = json_response(start).await["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/uploads/{upload_id}/blocks/{hash}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(content.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::NO_CONTENT);
    json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/uploads/{upload_id}/commit"),
                Some(token),
                json!({}),
            ))
            .await
            .unwrap(),
    )
    .await
}

async fn update_file(
    app: &axum::Router,
    token: &str,
    space_id: &str,
    path: &str,
    file_id: &str,
    base_version_id: &str,
    content: &[u8],
    key: &str,
) -> Value {
    let hash = blake3::hash(content).to_hex().to_string();
    let start = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/spaces/{space_id}/uploads"),
                Some(token),
                json!({"path":path,"file_id":file_id,"base_version_id":base_version_id,"size":content.len(),
                    "blocks":[{"hash":hash,"size":content.len()}],"idempotency_key":key}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let upload_id = start["upload_id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/v1/uploads/{upload_id}/blocks/{hash}"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(content.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT,
    );
    json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/uploads/{upload_id}/commit"),
                Some(token),
                json!({}),
            ))
            .await
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn setup_upload_commit_download_and_change_cursor() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();

    let setup = app.clone().oneshot(json_request("POST","/v1/setup",None,json!({
        "username":"hedwig","password":"correct horse battery staple","device_name":"Testgerät"
    }))).await.unwrap();
    assert_eq!(setup.status(), StatusCode::CREATED);
    let setup = json_response(setup).await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();

    let content = b"Eine echte, gepruefte Datei";
    let hash = blake3::hash(content).to_hex().to_string();
    let start = app.clone().oneshot(json_request("POST",&format!("/v1/spaces/{space_id}/uploads"),Some(token),json!({
        "path":"Dokumente/Test.txt","size":content.len(),"blocks":[{"hash":hash,"size":content.len()}],
        "idempotency_key":"test-upload-0001"
    }))).await.unwrap();
    assert_eq!(start.status(), StatusCode::CREATED);
    let start = json_response(start).await;
    assert_eq!(start["missing_blocks"], json!([hash]));
    let upload_id = start["upload_id"].as_str().unwrap();

    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/uploads/{upload_id}/blocks/{hash}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(content.as_slice()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::NO_CONTENT);

    let commit = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/v1/uploads/{upload_id}/commit"),
            Some(token),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(commit.status(), StatusCode::OK);
    let committed = json_response(commit).await;
    let file_id = committed["file_id"].as_str().unwrap();
    assert_eq!(committed["conflict"], false);

    let repeated = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/v1/uploads/{upload_id}/commit"),
            Some(token),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(repeated.status(), StatusCode::OK);
    assert_eq!(
        json_response(repeated).await["version_id"],
        committed["version_id"]
    );

    let versions = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{file_id}/versions"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(versions.status(), StatusCode::OK);
    let versions = json_response(versions).await;
    assert_eq!(versions["versions"].as_array().unwrap().len(), 1);
    assert_eq!(versions["versions"][0]["current"], true);

    let download = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{file_id}/content"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        download.into_body().collect().await.unwrap().to_bytes(),
        content.as_slice()
    );

    let moved = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/v1/files/{file_id}/move"),
            Some(token),
            json!({"path":"Archiv/Umbenannt.txt"}),
        ))
        .await
        .unwrap();
    assert_eq!(moved.status(), StatusCode::OK);
    assert_eq!(json_response(moved).await["path"], "Archiv/Umbenannt.txt");

    let deleted = app
        .clone()
        .oneshot(json_request(
            "DELETE",
            &format!("/v1/files/{file_id}"),
            Some(token),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);
    assert!(json_response(deleted).await["change_sequence"].is_number());
    let repeated_delete = app
        .clone()
        .oneshot(json_request(
            "DELETE",
            &format!("/v1/files/{file_id}"),
            Some(token),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(
        json_response(repeated_delete).await["change_sequence"],
        Value::Null
    );

    let restore = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/v1/files/{file_id}/restore"),
            Some(token),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(restore.status(), StatusCode::OK);
    assert_eq!(json_response(restore).await["path"], "Archiv/Umbenannt.txt");

    let changes = app
        .oneshot(
            Request::builder()
                .uri("/v1/changes?after=0")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let changes = json_response(changes).await;
    assert_eq!(changes["changes"].as_array().unwrap().len(), 4);
    assert_eq!(changes["changes"][0]["kind"], "content_changed");
    assert_eq!(changes["changes"][1]["kind"], "moved");
    assert_eq!(changes["changes"][2]["kind"], "deleted");
    assert_eq!(changes["changes"][3]["kind"], "restored");
}

#[tokio::test]
async fn incompatible_native_protocol_receives_an_explicit_update_requirement() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let legacy = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/compatibility")
                .header("x-cloud-sync-protocol", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::OK);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/compatibility")
                .header("x-hedwig-protocol", "0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
    let body = json_response(response).await;
    assert_eq!(body["error"]["code"], "client_update_required");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("aktualisieren")
    );
}

#[tokio::test]
async fn block_ids_do_not_grant_cross_space_access() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(app.clone().oneshot(json_request("POST","/v1/setup",None,json!({
        "username":"admin","password":"a very long secure password","device_name":"Browser"
    }))).await.unwrap()).await;
    let token = setup["token"].as_str().unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files/known-or-guessed-id/content")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn shared_space_roles_are_enforced_server_side() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let admin = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Admin"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let admin_token = admin["token"].as_str().unwrap();
    let user = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/admin/users",
                Some(admin_token),
                json!({"username":"viewer","password":"another secure password"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let user_id = user["id"].as_str().unwrap();
    let space = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/spaces",
                Some(admin_token),
                json!({"name":"Gemeinsam"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let space_id = space["id"].as_str().unwrap();
    let membership = app
        .clone()
        .oneshot(json_request(
            "PUT",
            &format!("/v1/spaces/{space_id}/members/{user_id}"),
            Some(admin_token),
            json!({"role":"viewer"}),
        ))
        .await
        .unwrap();
    assert_eq!(membership.status(), StatusCode::NO_CONTENT);

    let viewer = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/auth/login",
                None,
                json!({"username":"viewer","password":"another secure password","device_name":"Viewer"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let viewer_token = viewer["token"].as_str().unwrap();
    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/spaces/{space_id}/files"))
                .header(header::AUTHORIZATION, format!("Bearer {viewer_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let denied_upload = app
        .oneshot(json_request(
            "POST",
            &format!("/v1/spaces/{space_id}/uploads"),
            Some(viewer_token),
            json!({"path":"denied.txt","size":0,"blocks":[],"idempotency_key":"viewer-denied-01"}),
        ))
        .await
        .unwrap();
    assert_eq!(denied_upload.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn share_management_is_hidden_from_viewers_and_explicitly_enabled_for_editors() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let admin = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Admin"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let admin_token = admin["token"].as_str().unwrap();
    let user = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/admin/users",
                Some(admin_token),
                json!({"username":"editor","password":"another secure password"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let user_id = user["id"].as_str().unwrap();
    let space = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/spaces",
                Some(admin_token),
                json!({"name":"Freigaben"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let space_id = space["id"].as_str().unwrap();
    upload_file(
        &app,
        admin_token,
        space_id,
        "Dokumente/lesbar.txt",
        b"share",
        "editor-share-source",
    )
    .await;
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/spaces/{space_id}/members/{user_id}"),
                Some(admin_token),
                json!({"role":"editor"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let editor = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/auth/login",
                None,
                json!({"username":"editor","password":"another secure password","device_name":"Editor"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let editor_token = editor["token"].as_str().unwrap();
    let share_body = json!({"space_id":space_id,"target_path":"Dokumente","mode":"read"});
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/shares",
                Some(editor_token),
                share_body.clone(),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert!(
        json_response(
            app.clone()
                .oneshot(
                    Request::builder()
                        .uri("/v1/shares")
                        .header(header::AUTHORIZATION, format!("Bearer {editor_token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await["shares"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/spaces/{space_id}/settings"),
                Some(admin_token),
                json!({"editor_sharing":true}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let created = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/shares",
                Some(editor_token),
                share_body,
            ))
            .await
            .unwrap(),
    )
    .await;
    let share_id = created["share"]["id"].as_str().unwrap();
    assert_eq!(
        json_response(
            app.clone()
                .oneshot(
                    Request::builder()
                        .uri("/v1/shares")
                        .header(header::AUTHORIZATION, format!("Bearer {editor_token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await["shares"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/spaces/{space_id}/members/{user_id}"),
                Some(admin_token),
                json!({"role":"viewer"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.oneshot(json_request(
            "DELETE",
            &format!("/v1/shares/{share_id}"),
            Some(editor_token),
            json!({}),
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn deleting_a_shared_space_requires_exact_owner_confirmation_and_revokes_access() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Admin"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/spaces",
                Some(token),
                json!({"name":"Entfernen"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let space_id = space["id"].as_str().unwrap();
    let uploaded = upload_file(
        &app,
        token,
        space_id,
        "Datei.txt",
        b"retained in storage",
        "deleted-space-file",
    )
    .await;
    let file_id = uploaded["file_id"].as_str().unwrap();
    let share = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/shares",
                Some(token),
                json!({"space_id":space_id,"target_path":"Datei.txt","mode":"read"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let share_token = share["token"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/spaces/{space_id}/delete"),
                Some(token),
                json!({"name":"entfernen"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/spaces/{space_id}/delete"),
                Some(token),
                json!({"name":"Entfernen"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/files/{file_id}/content"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.oneshot(share_request(
            "GET",
            "/v1/public/share",
            share_token,
            None,
            json!({}),
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn password_share_scopes_downloads_and_revokes_immediately() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    let visible = upload_file(
        &app,
        token,
        space_id,
        "Dokumente/sichtbar.txt",
        b"freigegeben",
        "share-visible-01",
    )
    .await;
    let hidden = upload_file(
        &app,
        token,
        space_id,
        "Privat/geheim.txt",
        b"geheim",
        "share-hidden-001",
    )
    .await;
    let created = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/shares",
            Some(token),
            json!({
                "space_id":space_id,"target_path":"Dokumente","mode":"read",
                "password":"acht zeichen"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_response(created).await;
    let share_token = created["token"].as_str().unwrap();
    let share_id = created["share"]["id"].as_str().unwrap();
    assert!(
        created["url_fragment"]
            .as_str()
            .unwrap()
            .starts_with("#/share/cl_")
    );

    let missing_password = app
        .clone()
        .oneshot(share_request(
            "GET",
            "/v1/public/share",
            share_token,
            None,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(missing_password.status(), StatusCode::UNAUTHORIZED);
    let wrong_password = app
        .clone()
        .oneshot(share_request(
            "GET",
            "/v1/public/share",
            share_token,
            Some("ganz falsch"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(wrong_password.status(), StatusCode::UNAUTHORIZED);

    let listing = app
        .clone()
        .oneshot(share_request(
            "GET",
            "/v1/public/share",
            share_token,
            Some("acht zeichen"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(listing.status(), StatusCode::OK);
    let listing = json_response(listing).await;
    assert_eq!(listing["files"].as_array().unwrap().len(), 1);
    assert_eq!(listing["files"][0]["path"], "Dokumente/sichtbar.txt");

    let visible_download = app
        .clone()
        .oneshot(share_request(
            "GET",
            &format!(
                "/v1/public/share/files/{}/content",
                visible["file_id"].as_str().unwrap()
            ),
            share_token,
            Some("acht zeichen"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(visible_download.status(), StatusCode::OK);
    assert_eq!(
        visible_download
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
        b"freigegeben".as_slice()
    );
    let hidden_download = app
        .clone()
        .oneshot(share_request(
            "GET",
            &format!(
                "/v1/public/share/files/{}/content",
                hidden["file_id"].as_str().unwrap()
            ),
            share_token,
            Some("acht zeichen"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(hidden_download.status(), StatusCode::FORBIDDEN);

    let revoked = app
        .clone()
        .oneshot(json_request(
            "DELETE",
            &format!("/v1/shares/{share_id}"),
            Some(token),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    let denied_after_revoke = app
        .oneshot(share_request(
            "GET",
            "/v1/public/share",
            share_token,
            Some("acht zeichen"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(denied_after_revoke.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn upload_share_hides_files_avoids_overwrite_and_rechecks_on_commit() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    upload_file(
        &app,
        token,
        space_id,
        "Uploads/report.txt",
        b"original",
        "upload-existing-1",
    )
    .await;
    let created = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/shares",
                Some(token),
                json!({"space_id":space_id,"target_path":"Uploads","mode":"upload","max_upload_size":100}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let share_token = created["token"].as_str().unwrap();
    let share_id = created["share"]["id"].as_str().unwrap();
    let info = json_response(
        app.clone()
            .oneshot(share_request(
                "GET",
                "/v1/public/share",
                share_token,
                None,
                json!({}),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert!(info["files"].as_array().unwrap().is_empty());

    let too_large = app
        .clone()
        .oneshot(share_request(
            "POST",
            "/v1/public/share/uploads",
            share_token,
            None,
            json!({"path":"large.bin","size":101,"blocks":[],"idempotency_key":"guest-too-large"}),
        ))
        .await
        .unwrap();
    assert_eq!(too_large.status(), StatusCode::BAD_REQUEST);

    let guest_content = b"guest version";
    let hash = blake3::hash(guest_content).to_hex().to_string();
    let start = json_response(
        app.clone()
            .oneshot(share_request(
                "POST",
                "/v1/public/share/uploads",
                share_token,
                None,
                json!({"path":"report.txt","size":guest_content.len(),"blocks":[{"hash":hash,"size":guest_content.len()}],"idempotency_key":"guest-upload-001"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let upload_id = start["upload_id"].as_str().unwrap();
    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "/v1/public/share/uploads/{upload_id}/blocks/{hash}"
                ))
                .header(header::AUTHORIZATION, format!("Share {share_token}"))
                .body(Body::from(guest_content.as_slice()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::NO_CONTENT);
    let committed = app
        .clone()
        .oneshot(share_request(
            "POST",
            &format!("/v1/public/share/uploads/{upload_id}/commit"),
            share_token,
            None,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::OK);
    let committed = json_response(committed).await;
    assert_ne!(committed["path"], "Uploads/report.txt");
    assert!(committed["conflict"].as_bool().unwrap());

    let second = json_response(
        app.clone()
            .oneshot(share_request(
                "POST",
                "/v1/public/share/uploads",
                share_token,
                None,
                json!({"path":"later.txt","size":0,"blocks":[],"idempotency_key":"guest-upload-002"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let second_id = second["upload_id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "DELETE",
                &format!("/v1/shares/{share_id}"),
                Some(token),
                json!({}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let revoked_commit = app
        .oneshot(share_request(
            "POST",
            &format!("/v1/public/share/uploads/{second_id}/commit"),
            share_token,
            None,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(revoked_commit.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn live_backup_restores_to_fresh_server_and_detects_tampering() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("live");
    let backup = temp.path().join("backup-001");
    let restored = temp.path().join("restored");
    let app = build_app(AppConfig::new(data.clone())).await.unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    let committed = upload_file(
        &app,
        token,
        space_id,
        "Backup/wichtig.bin",
        b"dieser Inhalt muss den Restore ueberleben",
        "backup-source-01",
    )
    .await;
    let file_id = committed["file_id"].as_str().unwrap();

    assert!(create_backup(&data, &data.join("verboten")).is_err());
    let manifest = create_backup(&data, &backup).unwrap();
    assert!(
        manifest
            .entries
            .iter()
            .any(|entry| entry.path == "metadata.db")
    );
    assert!(
        manifest
            .entries
            .iter()
            .any(|entry| entry.path.starts_with("blocks/"))
    );
    verify_backup(&backup).unwrap();
    let occupied = temp.path().join("occupied");
    std::fs::create_dir(&occupied).unwrap();
    std::fs::write(occupied.join("do-not-overwrite"), b"bestehend").unwrap();
    assert!(restore_backup(&backup, &occupied).is_err());
    restore_backup(&backup, &restored).unwrap();

    let restored_app = build_app(AppConfig::new(restored)).await.unwrap();
    let download = restored_app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{file_id}/content"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        download.into_body().collect().await.unwrap().to_bytes(),
        b"dieser Inhalt muss den Restore ueberleben".as_slice()
    );

    let block = manifest
        .entries
        .iter()
        .find(|entry| entry.path.starts_with("blocks/"))
        .unwrap();
    std::fs::write(backup.join(&block.path), b"manipuliert").unwrap();
    assert!(verify_backup(&backup).is_err());
}

#[tokio::test]
async fn restoring_old_version_creates_new_current_version_and_keeps_history() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(app.clone().oneshot(json_request(
        "POST", "/v1/setup", None,
        json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
    )).await.unwrap()).await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    let first = upload_file(
        &app,
        token,
        space_id,
        "Text/notiz.txt",
        b"erste Fassung",
        "version-first-001",
    )
    .await;
    let file_id = first["file_id"].as_str().unwrap();
    let first_version = first["version_id"].as_str().unwrap();
    let second = update_file(
        &app,
        token,
        space_id,
        "Text/notiz.txt",
        file_id,
        first_version,
        b"zweite Fassung",
        "version-second-01",
    )
    .await;
    assert_ne!(second["version_id"], first["version_id"]);

    let restored = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/v1/files/{file_id}/versions/{first_version}/restore"),
            Some(token),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(restored.status(), StatusCode::OK);
    let restored = json_response(restored).await;
    assert_ne!(restored["version_id"], first["version_id"]);
    assert_ne!(restored["version_id"], second["version_id"]);

    let download = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{file_id}/content"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        download.into_body().collect().await.unwrap().to_bytes(),
        b"erste Fassung".as_slice()
    );

    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/files/{file_id}/versions/{first_version}/pin"),
                Some(token),
                json!({"name":"Vor Überarbeitung"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let versions = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/files/{file_id}/versions"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(versions["versions"].as_array().unwrap().len(), 3);
    assert!(
        versions["versions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|version| {
                version["id"] == first_version && version["pinned_name"] == "Vor Überarbeitung"
            })
    );
    assert_eq!(
        versions["versions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|version| version["current"] == true)
            .count(),
        1
    );
}

#[tokio::test]
async fn password_recovery_logout_and_device_revocation_are_enforced() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let first = json_response(app.clone().oneshot(json_request(
        "POST", "/v1/setup", None,
        json!({"username":"admin","password":"a very long secure password","device_name":"Erstes Gerät"}),
    )).await.unwrap()).await;
    let first_token = first["token"].as_str().unwrap();
    let second = json_response(app.clone().oneshot(json_request(
        "POST", "/v1/auth/login", None,
        json!({"username":"admin","password":"a very long secure password","device_name":"Zweites Gerät"}),
    )).await.unwrap()).await;
    let second_token = second["token"].as_str().unwrap();
    let devices = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/devices")
                    .header(header::AUTHORIZATION, format!("Bearer {first_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(devices["devices"].as_array().unwrap().len(), 2);
    assert!(
        devices["devices"]
            .as_array()
            .unwrap()
            .iter()
            .all(|device| device["active_sessions"] == 1)
    );

    let codes = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/auth/recovery-codes",
                Some(first_token),
                json!({}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let recovery_code = codes["codes"][0].as_str().unwrap();
    assert_eq!(codes["codes"].as_array().unwrap().len(), 10);

    assert_eq!(app.clone().oneshot(json_request(
        "PUT", "/v1/auth/password", Some(first_token),
        json!({"current_password":"a very long secure password","new_password":"a different secure password"}),
    )).await.unwrap().status(), StatusCode::NO_CONTENT);
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/spaces")
                    .header(header::AUTHORIZATION, format!("Bearer {second_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/auth/login",
                None,
                json!({"username":"admin","password":"a very long secure password"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let recovered_response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/auth/recover",
            None,
            json!({"username":"admin","recovery_code":recovery_code,"device_name":"Recovery"}),
        ))
        .await
        .unwrap();
    assert_eq!(recovered_response.status(), StatusCode::OK);
    let recovered = json_response(recovered_response).await;
    let recovered_token = recovered["token"].as_str().unwrap();
    let recovered_device = recovered["device_id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/auth/recover",
                None,
                json!({"username":"admin","recovery_code":recovery_code}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "DELETE",
                &format!("/v1/devices/{recovered_device}"),
                Some(first_token),
                json!({}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/spaces")
                    .header(header::AUTHORIZATION, format!("Bearer {recovered_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/auth/logout",
                Some(first_token),
                json!({}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/spaces")
                    .header(header::AUTHORIZATION, format!("Bearer {first_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn quotas_block_new_version_without_replacing_confirmed_content() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(app.clone().oneshot(json_request(
        "POST", "/v1/setup", None,
        json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
    )).await.unwrap()).await;
    let token = setup["token"].as_str().unwrap();
    let user_id = setup["user_id"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/admin/spaces/{space_id}/quota"),
                Some(token),
                json!({"quota_bytes":1048576}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );

    let first_content = vec![7_u8; 600_000];
    let first = upload_file(
        &app,
        token,
        space_id,
        "quota.bin",
        &first_content,
        "quota-first-0001",
    )
    .await;
    let file_id = first["file_id"].as_str().unwrap();
    let first_version = first["version_id"].as_str().unwrap();
    let second_content = vec![9_u8; 600_000];
    let hash = blake3::hash(&second_content).to_hex().to_string();
    let started = json_response(app.clone().oneshot(json_request(
        "POST", &format!("/v1/spaces/{space_id}/uploads"), Some(token),
        json!({"path":"quota.bin","file_id":file_id,"base_version_id":first_version,"size":second_content.len(),
            "blocks":[{"hash":hash,"size":second_content.len()}],"idempotency_key":"quota-second-001"}),
    )).await.unwrap()).await;
    let upload_id = started["upload_id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/v1/uploads/{upload_id}/blocks/{hash}"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(second_content))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/uploads/{upload_id}/commit"),
                Some(token),
                json!({}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );

    let download = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{file_id}/content"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        download.into_body().collect().await.unwrap().to_bytes(),
        first_content
    );
    let usage = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/spaces/{space_id}/usage"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(usage["current_bytes"], 600_000);
    assert_eq!(usage["version_bytes"], 600_000);
    assert_eq!(usage["physical_bytes"], 600_000);
    assert_eq!(usage["quota_bytes"], 1_048_576);

    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/admin/spaces/{space_id}/quota"),
                Some(token),
                json!({"quota_bytes":null}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/admin/users/{user_id}/quota"),
                Some(token),
                json!({"quota_bytes":1048576}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let shared = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/spaces",
                Some(token),
                json!({"name":"Weiterer Space"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let shared_id = shared["id"].as_str().unwrap();
    let content = vec![3_u8; 500_000];
    let hash = blake3::hash(&content).to_hex().to_string();
    let started = json_response(app.clone().oneshot(json_request(
        "POST", &format!("/v1/spaces/{shared_id}/uploads"), Some(token),
        json!({"path":"mehr.bin","size":content.len(),"blocks":[{"hash":hash,"size":content.len()}],"idempotency_key":"quota-user-0001"}),
    )).await.unwrap()).await;
    let upload_id = started["upload_id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/v1/uploads/{upload_id}/blocks/{hash}"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(content))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.oneshot(json_request(
            "POST",
            &format!("/v1/uploads/{upload_id}/commit"),
            Some(token),
            json!({}),
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn maintenance_removes_orphans_but_keeps_blocks_leased_by_open_uploads() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let app = build_app(AppConfig::new(data.clone())).await.unwrap();
    let setup = json_response(app.clone().oneshot(json_request(
        "POST", "/v1/setup", None,
        json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
    )).await.unwrap()).await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    let protected_content = b"laufender upload";
    let protected_hash = blake3::hash(protected_content).to_hex().to_string();
    let start = json_response(app.clone().oneshot(json_request(
        "POST", &format!("/v1/spaces/{space_id}/uploads"), Some(token),
        json!({"path":"offen.bin","size":protected_content.len(),"blocks":[{"hash":protected_hash,"size":protected_content.len()}],
            "idempotency_key":"maintenance-open"}),
    )).await.unwrap()).await;
    let upload_id = start["upload_id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/v1/uploads/{upload_id}/blocks/{protected_hash}"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(protected_content.as_slice()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let store = BlockStore::new(data.join("blocks")).await.unwrap();
    let orphan_content = b"verwaist";
    let orphan_hash = blake3::hash(orphan_content).to_hex().to_string();
    store
        .put_verified(space_id, &orphan_hash, orphan_content)
        .await
        .unwrap();
    assert!(
        store
            .contains(space_id, &protected_hash, protected_content.len() as u32)
            .await
            .unwrap()
    );
    assert!(
        store
            .contains(space_id, &orphan_hash, orphan_content.len() as u32)
            .await
            .unwrap()
    );

    let report = run_maintenance(&data).await.unwrap();
    assert_eq!(report.removed_blocks, 1);
    assert!(
        store
            .contains(space_id, &protected_hash, protected_content.len() as u32)
            .await
            .unwrap()
    );
    assert!(
        !store
            .contains(space_id, &orphan_hash, orphan_content.len() as u32)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn archive_protection_requires_admin_toggle_before_delete() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(app.clone().oneshot(json_request(
        "POST", "/v1/setup", None,
        json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
    )).await.unwrap()).await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    let file = upload_file(
        &app,
        token,
        space_id,
        "archiv.txt",
        b"bleibt",
        "archive-file-001",
    )
    .await;
    let file_id = file["file_id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/admin/spaces/{space_id}/archive-protection"),
                Some(token),
                json!({"enabled":true}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "DELETE",
                &format!("/v1/files/{file_id}"),
                Some(token),
                json!({}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/admin/spaces/{space_id}/archive-protection"),
                Some(token),
                json!({"enabled":false}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.oneshot(json_request(
            "DELETE",
            &format!("/v1/files/{file_id}"),
            Some(token),
            json!({}),
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn group_roles_are_effective_and_preserve_uploader_viewer_boundaries() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let admin = json_response(app.clone().oneshot(json_request(
        "POST", "/v1/setup", None,
        json!({"username":"admin","password":"a very long secure password","device_name":"Admin"}),
    )).await.unwrap()).await;
    let admin_token = admin["token"].as_str().unwrap();
    let user = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/admin/users",
                Some(admin_token),
                json!({"username":"group-user","password":"another secure password"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let user_id = user["id"].as_str().unwrap();
    let group = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/admin/groups",
                Some(admin_token),
                json!({"name":"Abgabe"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let group_id = group["id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/admin/groups/{group_id}/members/{user_id}"),
                Some(admin_token),
                json!({}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let space = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/spaces",
                Some(admin_token),
                json!({"name":"Gruppen-Space"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let space_id = space["id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/spaces/{space_id}/groups/{group_id}"),
                Some(admin_token),
                json!({"role":"uploader"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let member = json_response(app.clone().oneshot(json_request(
        "POST", "/v1/auth/login", None,
        json!({"username":"group-user","password":"another secure password","device_name":"Mitglied"}),
    )).await.unwrap()).await;
    let member_token = member["token"].as_str().unwrap();
    let spaces = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/spaces")
                    .header(header::AUTHORIZATION, format!("Bearer {member_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert!(
        spaces["spaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == space_id && item["role"] == "uploader")
    );
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/spaces/{space_id}/files"))
                    .header(header::AUTHORIZATION, format!("Bearer {member_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(app.clone().oneshot(json_request(
        "POST", &format!("/v1/spaces/{space_id}/uploads"), Some(member_token),
        json!({"path":"abgabe.txt","size":0,"blocks":[],"idempotency_key":"group-upload-001"}),
    )).await.unwrap().status(), StatusCode::CREATED);

    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/spaces/{space_id}/groups/{group_id}"),
                Some(admin_token),
                json!({"role":"viewer"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/spaces/{space_id}/files"))
                    .header(header::AUTHORIZATION, format!("Bearer {member_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(app.oneshot(json_request(
        "POST", &format!("/v1/spaces/{space_id}/uploads"), Some(member_token),
        json!({"path":"verboten.txt","size":0,"blocks":[],"idempotency_key":"group-upload-002"}),
    )).await.unwrap().status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn export_writes_current_and_pinned_history_as_verified_normal_files() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let destination = temp.path().join("export");
    let app = build_app(AppConfig::new(data.clone())).await.unwrap();
    let setup = json_response(app.clone().oneshot(json_request(
        "POST", "/v1/setup", None,
        json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
    )).await.unwrap()).await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    let first = upload_file(
        &app,
        token,
        space_id,
        "Docs/doc.txt",
        b"alte Fassung",
        "export-first-001",
    )
    .await;
    let file_id = first["file_id"].as_str().unwrap();
    let first_version = first["version_id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "PUT",
                &format!("/v1/files/{file_id}/versions/{first_version}/pin"),
                Some(token),
                json!({"name":"Ausgang"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    update_file(
        &app,
        token,
        space_id,
        "Docs/doc.txt",
        file_id,
        first_version,
        b"neue Fassung",
        "export-second-01",
    )
    .await;

    let report = export_directory_tree(&data, &destination, true)
        .await
        .unwrap();
    assert_eq!(report.current_files, 1);
    assert_eq!(report.historical_files, 1);
    let space_directory = destination.join(format!("Dateien von admin--{space_id}"));
    assert_eq!(
        std::fs::read(space_directory.join("current/Docs/doc.txt")).unwrap(),
        b"neue Fassung"
    );
    let history = std::fs::read_dir(space_directory.join("versions/Docs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(
        history
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("Ausgang")
    );
    assert_eq!(std::fs::read(history).unwrap(), b"alte Fassung");
}

#[tokio::test]
async fn same_space_copy_reuses_blocks_and_keeps_independent_file_identity() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    let content = vec![42_u8; 333_333];
    let original = upload_file(
        &app,
        token,
        space_id,
        "Quellen/original.bin",
        &content,
        "copy-source-001",
    )
    .await;
    let original_id = original["file_id"].as_str().unwrap();

    let copied_response = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/v1/files/{original_id}/copy"),
            Some(token),
            json!({"path":"Kopien/eigenstaendig.bin"}),
        ))
        .await
        .unwrap();
    assert_eq!(copied_response.status(), StatusCode::OK);
    let copied = json_response(copied_response).await;
    let copied_id = copied["file_id"].as_str().unwrap();
    assert_ne!(copied_id, original_id);

    let download = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{copied_id}/content"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        download.into_body().collect().await.unwrap().to_bytes(),
        content
    );

    let usage = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/spaces/{space_id}/usage"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(usage["current_bytes"], 666_666);
    assert_eq!(usage["version_bytes"], 666_666);
    assert_eq!(usage["physical_bytes"], 333_333);

    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "DELETE",
                &format!("/v1/files/{original_id}"),
                Some(token),
                json!({}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.oneshot(
            Request::builder()
                .uri(format!("/v1/files/{copied_id}/content"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes(),
        content
    );
}

#[tokio::test]
async fn empty_folders_are_persistent_and_recursive_delete_is_prefix_safe() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    let created = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/spaces/{space_id}/folders"),
                Some(token),
                json!({"path":"A_B/Leer"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let folder_id = created["id"].as_str().unwrap();
    upload_file(
        &app,
        token,
        space_id,
        "A_B/Leer/datei.txt",
        b"delete me",
        "folder-delete-01",
    )
    .await;
    let keep = upload_file(
        &app,
        token,
        space_id,
        "AXB/bleibt.txt",
        b"keep me",
        "folder-keep-0001",
    )
    .await;

    let folders = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/spaces/{space_id}/folders"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert!(
        folders["folders"]
            .as_array()
            .unwrap()
            .iter()
            .any(|folder| folder["path"] == "A_B/Leer")
    );

    let deleted = json_response(
        app.clone()
            .oneshot(json_request(
                "DELETE",
                &format!("/v1/folders/{folder_id}"),
                Some(token),
                json!({}),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(deleted["deleted_files"], 1);
    let keep_id = keep["file_id"].as_str().unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{keep_id}/content"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        b"keep me".as_slice()
    );
}

#[tokio::test]
async fn moving_a_folder_is_atomic_preserves_file_identity_and_rejects_collisions() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    let source = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/spaces/{space_id}/folders"),
                Some(token),
                json!({"path":"Quelle"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let source_id = source["id"].as_str().unwrap();
    for path in ["Quelle/Leer", "Belegt"] {
        assert_eq!(
            app.clone()
                .oneshot(json_request(
                    "POST",
                    &format!("/v1/spaces/{space_id}/folders"),
                    Some(token),
                    json!({"path":path}),
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
    }
    let uploaded = upload_file(
        &app,
        token,
        space_id,
        "Quelle/Unter/datei.txt",
        b"identity remains",
        "folder-move-identity",
    )
    .await;
    let file_id = uploaded["file_id"].as_str().unwrap();

    let collision = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/v1/folders/{source_id}/move"),
            Some(token),
            json!({"path":"Belegt"}),
        ))
        .await
        .unwrap();
    assert_eq!(collision.status(), StatusCode::CONFLICT);

    let moved = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/folders/{source_id}/move"),
                Some(token),
                json!({"path":"Archiv/Projekt"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(moved["id"], source_id);
    assert_eq!(moved["path"], "Archiv/Projekt");

    let files = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/spaces/{space_id}/files"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let file = files["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["id"] == file_id)
        .unwrap();
    assert_eq!(file["path"], "Archiv/Projekt/Unter/datei.txt");

    let folders = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/spaces/{space_id}/folders"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let paths: Vec<_> = folders["folders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|folder| folder["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"Archiv/Projekt"));
    assert!(paths.contains(&"Archiv/Projekt/Leer"));
    assert!(paths.contains(&"Archiv/Projekt/Unter"));
    assert!(!paths.iter().any(|path| path.starts_with("Quelle")));

    let content = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{file_id}/content"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(content, b"identity remains".as_slice());
}

#[tokio::test]
async fn nested_folders_create_parents_and_file_directory_collisions_are_rejected() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/spaces/{space_id}/folders"),
                Some(token),
                json!({"path":"Tiefe/Leere/Struktur"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let folders = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/spaces/{space_id}/folders"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let paths: Vec<_> = folders["folders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|folder| folder["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"Tiefe"));
    assert!(paths.contains(&"Tiefe/Leere"));
    assert!(paths.contains(&"Tiefe/Leere/Struktur"));

    let uploaded = upload_file(
        &app,
        token,
        space_id,
        "Sperre",
        b"file",
        "path-collision-file",
    )
    .await;
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/spaces/{space_id}/folders"),
                Some(token),
                json!({"path":"Sperre/Unter"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let file_id = uploaded["file_id"].as_str().unwrap();
    for operation in ["move", "copy"] {
        assert_eq!(
            app.clone()
                .oneshot(json_request(
                    "POST",
                    &format!("/v1/files/{file_id}/{operation}"),
                    Some(token),
                    json!({"path":"Tiefe"}),
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
    }
}

#[tokio::test]
async fn folder_archive_contains_empty_folders_and_rejects_corrupt_blocks() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/spaces/{space_id}/folders"),
                Some(token),
                json!({"path":"Projekt/Leer"}),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let content = b"archivierter Inhalt";
    upload_file(
        &app,
        token,
        space_id,
        "Projekt/Text/bericht.txt",
        content,
        "archive-file-001",
    )
    .await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/spaces/{space_id}/archive?path=Projekt"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/zip");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    assert!(zip.by_name("Leer/").unwrap().is_dir());
    let mut archived = Vec::new();
    std::io::Read::read_to_end(&mut zip.by_name("Text/bericht.txt").unwrap(), &mut archived)
        .unwrap();
    assert_eq!(archived, content);

    let hash = blake3::hash(content).to_hex().to_string();
    std::fs::write(
        temp.path()
            .join("blocks")
            .join(space_id)
            .join(&hash[..2])
            .join(&hash),
        b"manipuliert",
    )
    .unwrap();
    let corrupt = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/spaces/{space_id}/archive?path=Projekt"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(corrupt.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn folder_restore_previews_and_recreates_historical_state_without_losing_newer_versions() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    let first = upload_file(
        &app,
        token,
        space_id,
        "Projekt/a.txt",
        b"alte Fassung",
        "folder-time-first",
    )
    .await;
    let file_id = first["file_id"].as_str().unwrap();
    let first_version = first["version_id"].as_str().unwrap();
    let changes = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/changes?after=0")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let target_at = changes["changes"].as_array().unwrap().last().unwrap()["created_at"]
        .as_str()
        .unwrap()
        .to_owned();
    update_file(
        &app,
        token,
        space_id,
        "Projekt/a.txt",
        file_id,
        first_version,
        b"neuere Fassung",
        "folder-time-second",
    )
    .await;
    let newer = upload_file(
        &app,
        token,
        space_id,
        "Projekt/spaeter.txt",
        b"spaeter angelegt",
        "folder-time-third",
    )
    .await;
    let newer_id = newer["file_id"].as_str().unwrap();

    let preview = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/spaces/{space_id}/folder-restore?path=Projekt&at={}",
                        url::form_urlencoded::byte_serialize(target_at.as_bytes())
                            .collect::<String>()
                    ))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert!(preview["unavailable"].as_array().unwrap().is_empty());
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |action| action["path"] == "Projekt/a.txt" && action["action"] == "restore_version"
            )
    );
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["path"] == "Projekt/spaeter.txt" && action["action"] == "delete")
    );

    let restored = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/spaces/{space_id}/folder-restore"),
                Some(token),
                json!({"path":"Projekt","at":target_at}),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(restored["restored_files"], 1);
    assert_eq!(restored["deleted_files"], 1);
    let download = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{file_id}/content"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        download.into_body().collect().await.unwrap().to_bytes(),
        b"alte Fassung".as_slice()
    );
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/files/{newer_id}/content"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let versions = json_response(
        app.oneshot(
            Request::builder()
                .uri(format!("/v1/files/{file_id}/versions"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(versions["versions"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn scheduled_backups_are_verified_retained_and_reported_without_stale_run_state() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let backup_root = temp.path().join("backups");
    let app = build_app(AppConfig::new(data.clone())).await.unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    upload_file(
        &app,
        token,
        space_id,
        "backup.txt",
        b"sicherungsinhalt",
        "scheduled-backup-01",
    )
    .await;

    for _ in 0..3 {
        run_scheduled_backup(&data, &backup_root, 2).unwrap();
    }
    let backups = std::fs::read_dir(&backup_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 2);
    for backup in &backups {
        verify_backup(&backup.path()).unwrap();
        let snapshot = rusqlite::Connection::open(backup.path().join("metadata.db")).unwrap();
        let runs: i64 = snapshot
            .query_row("SELECT COUNT(*) FROM backup_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(runs, 0);
    }

    let status = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/admin/backup-status")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status["latest"]["status"], "completed");
    assert!(status["latest"]["entries"].as_i64().unwrap() >= 2);
    assert!(status["latest"]["bytes"].as_i64().unwrap() > 0);

    let diagnostics = app
        .oneshot(
            Request::builder()
                .uri("/v1/admin/diagnostics")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(diagnostics.status(), StatusCode::OK);
    let bytes = diagnostics.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!text.contains(token));
    assert!(!text.contains("backup.txt"));
    assert!(!text.contains("sicherungsinhalt"));
    let report: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["privacy"]["contains_tokens"], false);
    assert_eq!(report["database_integrity"], "ok");
}

#[tokio::test]
async fn admin_can_run_configured_backup_and_sees_same_filesystem_warning() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("live");
    let backup = temp.path().join("separate-directory");
    let mut config = AppConfig::new(data);
    config.backup_dir = Some(backup.clone());
    config.backup_interval_hours = 24;
    config.backup_keep = 3;
    let app = build_app(config).await.unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Admin"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let status = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/admin/backup-status")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status["configuration"]["keep"], 3);
    assert_eq!(status["configuration"]["same_filesystem"], cfg!(unix));
    let run = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/admin/backup-run",
                Some(token),
                json!({}),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(run["status"], "completed");
    assert!(run["entries"].as_u64().unwrap() >= 1);
    assert_eq!(
        std::fs::read_dir(backup)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("backup-manifest.json").is_file())
            .count(),
        1
    );
}

#[tokio::test]
async fn webhook_delivery_is_signed_and_uses_change_event_id() {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    let receiver_app = axum::Router::new().route(
        "/hook",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                let sender = sender.clone();
                async move {
                    sender.send((headers, body)).await.unwrap();
                    StatusCode::NO_CONTENT
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, receiver_app).await.unwrap() });

    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let app = build_app(AppConfig::new(data.clone())).await.unwrap();
    let setup = json_response(app.clone().oneshot(json_request(
        "POST", "/v1/setup", None,
        json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
    )).await.unwrap()).await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    let created = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                &format!("/v1/spaces/{space_id}/webhooks"),
                Some(token),
                json!({"url":format!("http://{address}/hook")}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let secret = created["signing_secret"].as_str().unwrap();
    upload_file(
        &app,
        token,
        space_id,
        "webhook.txt",
        b"event",
        "webhook-file-001",
    )
    .await;

    let database = Database::open(data.join("metadata.db")).unwrap();
    assert_eq!(deliver_pending_webhooks(&database).await.unwrap(), 1);
    let (headers, body) = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["type"], "content_changed");
    assert_eq!(
        headers["x-cloud-event-id"],
        payload["event_id"].as_str().unwrap()
    );
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(&body);
    assert_eq!(
        headers["x-cloud-signature"],
        format!("v1={}", hex::encode(mac.finalize().into_bytes()))
    );
    assert_eq!(deliver_pending_webhooks(&database).await.unwrap(), 0);
}

#[tokio::test]
async fn admin_upload_policy_blocks_device_and_guest_uploads_but_allows_other_files() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();

    let saved = app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/v1/admin/upload-policy",
            Some(token),
            json!({"patterns":"*.exe\nverboten/**\n"}),
        ))
        .await
        .unwrap();
    assert_eq!(saved.status(), StatusCode::OK);
    let loaded = json_response(
        app.clone()
            .oneshot(json_request(
                "GET",
                "/v1/admin/upload-policy",
                Some(token),
                json!({}),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(loaded["patterns"], "*.exe\nverboten/**\n");

    let allowed = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/v1/spaces/{space_id}/uploads"),
            Some(token),
            json!({"path":"Dokumente/Notiz.txt","size":0,"blocks":[],"idempotency_key":"policy-allowed-001"}),
        ))
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::CREATED);
    let blocked = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/v1/spaces/{space_id}/uploads"),
            Some(token),
            json!({"path":"Programme/Setup.exe","size":0,"blocks":[],"idempotency_key":"policy-blocked-001"}),
        ))
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::BAD_REQUEST);
    assert!(
        json_response(blocked).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("*.exe")
    );

    let share = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/shares",
                Some(token),
                json!({"space_id":space_id,"target_path":"Uploads","mode":"upload"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let share_token = share["token"].as_str().unwrap();
    let guest_blocked = app
        .oneshot(share_request(
            "POST",
            "/v1/public/share/uploads",
            share_token,
            None,
            json!({"path":"Gast.exe","size":0,"blocks":[],"idempotency_key":"policy-guest-001"}),
        ))
        .await
        .unwrap();
    assert_eq!(guest_blocked.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn configured_mirror_is_refreshed_atomically_and_reports_read_only_usage() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let mirror = temp.path().join("mirror");
    let mut config = AppConfig::new(data);
    config.mirror_dir = Some(mirror.clone());
    config.mirror_interval_minutes = 10;
    let app = build_app(config).await.unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let space_id = setup["personal_space_id"].as_str().unwrap();
    upload_file(
        &app,
        token,
        space_id,
        "Ordner/Spiegel.txt",
        b"sichtbarer Inhalt",
        "mirror-file-001",
    )
    .await;

    let refreshed = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/admin/mirror-run",
            Some(token),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(refreshed.status(), StatusCode::OK);
    let refreshed = json_response(refreshed).await;
    assert_eq!(refreshed["current_files"], 1);
    assert_eq!(refreshed["bytes_written"], b"sichtbarer Inhalt".len());
    let mirrored_file = walkdir::WalkDir::new(&mirror)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| entry.file_name() == "Spiegel.txt")
        .unwrap();
    assert_eq!(
        std::fs::read(mirrored_file.path()).unwrap(),
        b"sichtbarer Inhalt"
    );
    assert!(
        std::fs::metadata(mirrored_file.path())
            .unwrap()
            .permissions()
            .readonly()
    );

    let status = json_response(
        app.oneshot(json_request(
            "GET",
            "/v1/admin/mirror-status",
            Some(token),
            json!({}),
        ))
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(status["configuration"]["interval_minutes"], 10);
    assert_eq!(status["latest"]["current_files"], 1);
}

#[tokio::test]
async fn cross_space_copy_rechecks_target_rights_and_copies_blocks_into_target_space() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap();
    let source_space = setup["personal_space_id"].as_str().unwrap();
    let target = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/spaces",
                Some(token),
                json!({"name":"Zielbereich"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let target_space = target["id"].as_str().unwrap();
    let source = upload_file(
        &app,
        token,
        source_space,
        "Quelle.txt",
        b"space-isolierte Bloecke",
        "cross-copy-source-001",
    )
    .await;
    let source_file = source["file_id"].as_str().unwrap();

    let copied = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/v1/files/{source_file}/copy"),
            Some(token),
            json!({"path":"Kopien/Ziel.txt","space_id":target_space}),
        ))
        .await
        .unwrap();
    assert_eq!(copied.status(), StatusCode::OK);
    let copied = json_response(copied).await;
    assert_ne!(copied["file_id"], source_file);

    let files = json_response(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/spaces/{target_space}/files"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(files["files"][0]["path"], "Kopien/Ziel.txt");
    let copied_file = copied["file_id"].as_str().unwrap();
    let content = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{copied_file}/content"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        content.into_body().collect().await.unwrap().to_bytes(),
        &b"space-isolierte Bloecke"[..]
    );
}

#[tokio::test]
async fn authenticated_long_poll_wakes_after_successful_mutation() {
    let temp = TempDir::new().unwrap();
    let app = build_app(AppConfig::new(temp.path().to_path_buf()))
        .await
        .unwrap();
    let setup = json_response(
        app.clone()
            .oneshot(json_request(
                "POST",
                "/v1/setup",
                None,
                json!({"username":"admin","password":"a very long secure password","device_name":"Browser"}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let token = setup["token"].as_str().unwrap().to_owned();
    let space_id = setup["personal_space_id"].as_str().unwrap().to_owned();

    let unauthenticated = app
        .clone()
        .oneshot(json_request(
            "GET",
            "/v1/events/wait?timeout_seconds=1",
            None,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let wait_app = app.clone();
    let wait_token = token.clone();
    let waiter = tokio::spawn(async move {
        wait_app
            .oneshot(json_request(
                "GET",
                "/v1/events/wait?timeout_seconds=2",
                Some(&wait_token),
                json!({}),
            ))
            .await
            .unwrap()
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mutation = app
        .oneshot(json_request(
            "POST",
            &format!("/v1/spaces/{space_id}/folders"),
            Some(&token),
            json!({"path":"Live"}),
        ))
        .await
        .unwrap();
    assert_eq!(mutation.status(), StatusCode::CREATED);

    let wakeup = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .expect("long poll did not wake")
        .unwrap();
    assert_eq!(wakeup.status(), StatusCode::OK);
    assert_eq!(json_response(wakeup).await["changed"], true);
}
