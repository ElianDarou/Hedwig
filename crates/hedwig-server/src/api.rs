use crate::{
    AppState,
    backup::run_scheduled_backup,
    database::{AuthContext, DbError, NewUpload, StoredBlock},
    read_mirror_status, refresh_read_only_mirror,
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use hedwig_core::CloudPath;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::{collections::HashMap, sync::Arc};
use tokio_util::io::ReaderStream;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

pub fn build_router(state: Arc<AppState>, web_dir: Option<PathBuf>) -> Router {
    let router = Router::new()
        .route("/health", get(health))
        .route("/v1/compatibility", get(compatibility))
        .route("/v1/setup/status", get(setup_status))
        .route("/v1/setup", post(setup))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/auth/password", put(change_password))
        .route("/v1/auth/recovery-codes", post(regenerate_recovery_codes))
        .route("/v1/auth/recover", post(recover))
        .route("/v1/auth/2fa", get(two_factor_status))
        .route("/v1/auth/2fa/begin", post(begin_two_factor))
        .route("/v1/auth/2fa/confirm", post(confirm_two_factor))
        .route("/v1/auth/2fa/disable", post(disable_two_factor))
        .route("/v1/devices", get(list_devices))
        .route(
            "/v1/devices/{device_id}",
            axum::routing::delete(revoke_device),
        )
        .route("/v1/spaces", get(list_spaces).post(create_space))
        .route("/v1/spaces/{space_id}/delete", post(delete_space))
        .route("/v1/spaces/{space_id}/settings", put(set_space_settings))
        .route("/v1/spaces/{space_id}/access", get(space_access))
        .route("/v1/admin/users", get(list_users).post(create_user))
        .route("/v1/admin/spaces", get(list_admin_spaces))
        .route("/v1/admin/system", get(system_status))
        .route("/v1/admin/backup-status", get(backup_status))
        .route("/v1/admin/backup-run", post(run_backup_now))
        .route("/v1/admin/mirror-status", get(mirror_status))
        .route("/v1/admin/mirror-run", post(run_mirror_now))
        .route(
            "/v1/admin/upload-policy",
            get(get_upload_policy).put(set_upload_policy),
        )
        .route("/v1/admin/diagnostics", get(download_diagnostics))
        .route("/v1/admin/integrity", post(run_integrity_check))
        .route("/v1/admin/groups", get(list_groups).post(create_group))
        .route(
            "/v1/admin/groups/{group_id}/members/{user_id}",
            put(add_group_member).delete(remove_group_member),
        )
        .route("/v1/admin/users/{user_id}/quota", put(set_user_quota))
        .route("/v1/admin/spaces/{space_id}/quota", put(set_space_quota))
        .route(
            "/v1/admin/spaces/{space_id}/archive-protection",
            put(set_archive_protection),
        )
        .route(
            "/v1/spaces/{space_id}/members/{user_id}",
            put(set_member).delete(remove_member),
        )
        .route(
            "/v1/spaces/{space_id}/groups/{group_id}",
            put(set_group_space_role).delete(remove_group_space_role),
        )
        .route("/v1/spaces/{space_id}/files", get(list_files))
        .route("/v1/spaces/{space_id}/archive", get(download_archive))
        .route(
            "/v1/spaces/{space_id}/folder-restore",
            get(folder_restore_preview).post(restore_folder),
        )
        .route(
            "/v1/spaces/{space_id}/folders",
            get(list_directories).post(create_directory),
        )
        .route(
            "/v1/folders/{directory_id}",
            axum::routing::delete(delete_directory),
        )
        .route("/v1/folders/{directory_id}/move", post(move_directory))
        .route("/v1/spaces/{space_id}/usage", get(space_usage))
        .route(
            "/v1/spaces/{space_id}/webhooks",
            get(list_webhooks).post(create_webhook),
        )
        .route(
            "/v1/spaces/{space_id}/webhooks/{webhook_id}",
            axum::routing::delete(disable_webhook),
        )
        .route("/v1/spaces/{space_id}/uploads", post(start_upload))
        .route(
            "/v1/spaces/{space_id}/uploads/batch",
            post(start_upload_batch),
        )
        .route("/v1/uploads/{upload_id}/blocks/{hash}", put(put_block))
        .route(
            "/v1/uploads/batch/blocks",
            put(put_block_batch).layer(DefaultBodyLimit::max(34 * 1024 * 1024)),
        )
        .route("/v1/uploads/{upload_id}/commit", post(commit_upload))
        .route("/v1/uploads/batch/commit", post(commit_upload_batch))
        .route("/v1/files/{file_id}/content", get(download_file))
        .route("/v1/files/{file_id}/manifest", get(file_manifest))
        .route("/v1/files/{file_id}/versions", get(list_versions))
        .route(
            "/v1/files/{file_id}/versions/{version_id}/content",
            get(download_version),
        )
        .route(
            "/v1/files/{file_id}/versions/{version_id}/restore",
            post(restore_version),
        )
        .route(
            "/v1/files/{file_id}/versions/{version_id}/pin",
            put(pin_version),
        )
        .route("/v1/files/{file_id}/move", post(move_file))
        .route("/v1/files/{file_id}/copy", post(copy_file))
        .route("/v1/files/{file_id}", axum::routing::delete(delete_file))
        .route("/v1/files/{file_id}/restore", post(restore_file))
        .route("/v1/changes", get(list_changes))
        .route("/v1/events/wait", get(wait_for_change))
        .route("/v1/shares", get(list_shares).post(create_share))
        .route("/v1/shares/{share_id}", axum::routing::delete(revoke_share))
        .route("/v1/public/share", get(public_share))
        .route(
            "/v1/public/share/files/{file_id}/content",
            get(public_download_file),
        )
        .route("/v1/public/share/uploads", post(public_start_upload))
        .route(
            "/v1/public/share/uploads/{upload_id}/blocks/{hash}",
            put(public_put_block),
        )
        .route(
            "/v1/public/share/uploads/{upload_id}/commit",
            post(public_commit_upload),
        )
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(axum::middleware::from_fn(protocol_guard))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            notify_mutations,
        ))
        .layer(TraceLayer::new_for_http());
    let router = if let Some(web_dir) = web_dir {
        let index = web_dir.join("index.html");
        router.fallback_service(ServeDir::new(web_dir).fallback(ServeFile::new(index)))
    } else {
        router
    };
    router.with_state(state)
}

async fn notify_mutations(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mutation = matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    let response = next.run(request).await;
    if mutation && response.status().is_success() {
        let _ = state.change_notifier.send(());
    }
    response
}

async fn health() -> Json<Value> {
    Json(json!({"status":"ok","service":"hedwig","api_version":"v1"}))
}

async fn compatibility() -> Json<Value> {
    Json(json!({
        "api_version": "v1",
        "protocol": 1,
        "minimum_protocol": 1,
        "maximum_protocol": 1,
        "server_version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Debug, Deserialize)]
struct WaitForChangeQuery {
    timeout_seconds: Option<u64>,
}

async fn wait_for_change(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<WaitForChangeQuery>,
) -> Result<Json<Value>, ApiError> {
    authenticate(&state, &headers)?;
    let mut receiver = state.change_notifier.subscribe();
    let timeout = std::time::Duration::from_secs(query.timeout_seconds.unwrap_or(25).clamp(1, 30));
    let changed = match tokio::time::timeout(timeout, receiver.recv()).await {
        Ok(Ok(())) | Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => true,
        Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) | Err(_) => false,
    };
    Ok(Json(json!({"changed":changed})))
}

async fn protocol_guard(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    // The legacy header is accepted for one migration window so existing installed clients can
    // reach the renamed server and receive normal update guidance instead of being cut off.
    if let Some(value) = request
        .headers()
        .get("x-hedwig-protocol")
        .or_else(|| request.headers().get("x-cloud-sync-protocol"))
    {
        if value.as_bytes() != b"1" {
            return (
                StatusCode::UPGRADE_REQUIRED,
                Json(json!({"error":{
                    "code":"client_update_required",
                    "message":"Diese Client-Version ist mit dem Server nicht kompatibel. Bitte den Client aktualisieren."
                }})),
            )
                .into_response();
        }
    }
    next.run(request).await
}

async fn setup_status(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"setup_required":state.database.setup_required()?}),
    ))
}

async fn system_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(serde_json::to_value(
        state.database.system_status(&auth)?,
    )?))
}

async fn backup_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(json!({
        "latest": state.database.latest_backup_status(&auth)?,
        "configuration": state.backup,
    })))
}

async fn run_backup_now(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    if !auth.is_admin {
        return Err(DbError::Forbidden.into());
    }
    let configuration = state
        .backup
        .clone()
        .ok_or_else(|| ApiError::conflict("Es ist kein Backup-Ziel konfiguriert"))?;
    let _gate = state.backup_gate.lock().await;
    let data_dir = state.data_dir.clone();
    let destination = PathBuf::from(&configuration.destination);
    let keep = configuration.keep;
    let manifest =
        tokio::task::spawn_blocking(move || run_scheduled_backup(&data_dir, &destination, keep))
            .await
            .map_err(ApiError::internal)?
            .map_err(ApiError::internal)?;
    let bytes: u64 = manifest.entries.iter().map(|entry| entry.size).sum();
    Ok(Json(json!({
        "status":"completed",
        "entries":manifest.entries.len(),
        "bytes":bytes,
        "created_at":manifest.created_at,
    })))
}

async fn mirror_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    if !auth.is_admin {
        return Err(DbError::Forbidden.into());
    }
    let status = match &state.mirror {
        Some(configuration) => read_mirror_status(std::path::Path::new(&configuration.destination))
            .map_err(ApiError::internal)?,
        None => None,
    };
    Ok(Json(json!({"configuration":state.mirror,"latest":status})))
}

async fn run_mirror_now(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    if !auth.is_admin {
        return Err(DbError::Forbidden.into());
    }
    let configuration = state
        .mirror
        .clone()
        .ok_or_else(|| ApiError::conflict("Es ist kein Spiegelziel konfiguriert"))?;
    let _gate = state.mirror_gate.lock().await;
    let status = refresh_read_only_mirror(
        &state.data_dir,
        std::path::Path::new(&configuration.destination),
    )
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(serde_json::to_value(status)?))
}

#[derive(Debug, Deserialize)]
struct UploadPolicyRequest {
    patterns: String,
}

async fn get_upload_policy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(
        json!({"patterns":state.database.upload_policy(&auth)?}),
    ))
}

async fn set_upload_policy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<UploadPolicyRequest>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state.database.set_upload_policy(&auth, &request.patterns)?;
    Ok(Json(json!({"patterns":request.patterns})))
}

async fn download_diagnostics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let auth = authenticate(&state, &headers)?;
    if !auth.is_admin {
        return Err(DbError::Forbidden.into());
    }
    state.database.integrity_check()?;
    let body = serde_json::to_vec_pretty(&json!({
        "format": 1,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "server": {
            "version": env!("CARGO_PKG_VERSION"),
            "api_version": "v1",
            "operating_system": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
        },
        "database_integrity": "ok",
        "system": state.database.system_status(&auth)?,
        "latest_backup": state.database.latest_backup_status(&auth)?,
        "privacy": {
            "contains_tokens": false,
            "contains_passwords": false,
            "contains_file_contents": false,
            "contains_file_paths": false,
        }
    }))?;
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"hedwig-diagnostics.json\""),
    );
    Ok(response)
}

async fn run_integrity_check(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    if !auth.is_admin {
        return Err(DbError::Forbidden.into());
    }
    state.database.integrity_check()?;
    Ok(Json(
        json!({"status":"ok","checked_at":chrono::Utc::now().to_rfc3339()}),
    ))
}

#[derive(Debug, Deserialize)]
struct SetupRequest {
    username: String,
    password: String,
    device_name: Option<String>,
}

async fn setup(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SetupRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let session = state.database.setup_admin(
        &request.username,
        &request.password,
        request.device_name.as_deref().unwrap_or("Webbrowser"),
    )?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(session)?)))
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
    device_name: Option<String>,
    otp: Option<String>,
}

async fn login(
    State(state): State<Arc<AppState>>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<Value>, ApiError> {
    let session = state.database.login_with_otp(
        &request.username,
        &request.password,
        request.device_name.as_deref().unwrap_or("Webbrowser"),
        request.otp.as_deref(),
    )?;
    Ok(Json(serde_json::to_value(session)?))
}

async fn two_factor_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(
        json!({"enabled":state.database.two_factor_status(&auth)?}),
    ))
}

#[derive(Debug, Deserialize)]
struct TwoFactorPasswordRequest {
    password: String,
}

async fn begin_two_factor(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TwoFactorPasswordRequest>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(serde_json::to_value(
        state.database.begin_two_factor(&auth, &request.password)?,
    )?))
}

#[derive(Debug, Deserialize)]
struct TwoFactorCodeRequest {
    password: String,
    code: String,
}

async fn confirm_two_factor(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TwoFactorCodeRequest>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .confirm_two_factor(&auth, &request.password, &request.code)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn disable_two_factor(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TwoFactorCodeRequest>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .disable_two_factor(&auth, &request.password, &request.code)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    authenticate(&state, &headers)?;
    state.database.logout(bearer_token(&headers)?)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

async fn change_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state.database.change_password(
        &auth,
        &request.current_password,
        &request.new_password,
        bearer_token(&headers)?,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

async fn regenerate_recovery_codes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(
        json!({"codes": state.database.regenerate_recovery_codes(&auth)?}),
    ))
}

#[derive(Debug, Deserialize)]
struct RecoverRequest {
    username: String,
    recovery_code: String,
    device_name: Option<String>,
}

async fn recover(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RecoverRequest>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(
        state.database.recover(
            &request.username,
            &request.recovery_code,
            request
                .device_name
                .as_deref()
                .unwrap_or("Wiederherstellung"),
        )?,
    )?))
}

async fn list_devices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(
        json!({"devices": state.database.list_devices(&auth)?}),
    ))
}

async fn revoke_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state.database.revoke_device(&auth, &device_id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_spaces(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(
        json!({"spaces":state.database.list_spaces(&auth.user_id)?}),
    ))
}

#[derive(Debug, Deserialize)]
struct CreateSpaceRequest {
    name: String,
}

async fn create_space(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateSpaceRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let auth = authenticate(&state, &headers)?;
    let space = state.database.create_space(&auth.user_id, &request.name)?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(space)?)))
}

#[derive(Debug, Deserialize)]
struct DeleteSpaceRequest {
    name: String,
}

async fn delete_space(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Json(request): Json<DeleteSpaceRequest>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .delete_space(&auth, &space_id, &request.name)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct SpaceSettingsRequest {
    editor_sharing: bool,
}

async fn set_space_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Json(request): Json<SpaceSettingsRequest>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .set_editor_sharing(&auth, &space_id, request.editor_sharing)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn space_access(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(serde_json::to_value(
        state.database.space_access(&auth, &space_id)?,
    )?))
}

#[derive(Debug, Deserialize)]
struct CreateUserRequest {
    username: String,
    password: String,
    is_admin: Option<bool>,
}
async fn create_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let auth = authenticate(&state, &headers)?;
    let user = state.database.create_user(
        &auth,
        &request.username,
        &request.password,
        request.is_admin.unwrap_or(false),
    )?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(user)?)))
}
async fn list_users(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(json!({"users":state.database.list_users(&auth)?})))
}

async fn list_admin_spaces(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(
        json!({"spaces":state.database.list_admin_spaces(&auth)?}),
    ))
}

#[derive(Debug, Deserialize)]
struct CreateGroupRequest {
    name: String,
}

async fn create_group(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateGroupRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(
            state.database.create_group(&auth, &request.name)?,
        )?),
    ))
}

async fn list_groups(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(json!({"groups":state.database.list_groups(&auth)?})))
}

async fn add_group_member(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((group_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .set_group_member(&auth, &group_id, &user_id, true)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_group_member(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((group_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .set_group_member(&auth, &group_id, &user_id, false)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct QuotaRequest {
    quota_bytes: Option<i64>,
}

async fn set_user_quota(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(request): Json<QuotaRequest>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .set_user_quota(&auth, &user_id, request.quota_bytes)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_space_quota(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Json(request): Json<QuotaRequest>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .set_space_quota(&auth, &space_id, request.quota_bytes)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct ArchiveProtectionRequest {
    enabled: bool,
}

async fn set_archive_protection(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Json(request): Json<ArchiveProtectionRequest>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .set_archive_protection(&auth, &space_id, request.enabled)?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Debug, Deserialize)]
struct SetMemberRequest {
    role: String,
}
async fn set_member(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((space_id, user_id)): Path<(String, String)>,
    Json(request): Json<SetMemberRequest>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .set_member(&auth, &space_id, &user_id, &request.role)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn remove_member(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((space_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state.database.remove_member(&auth, &space_id, &user_id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_group_space_role(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((space_id, group_id)): Path<(String, String)>,
    Json(request): Json<SetMemberRequest>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .set_group_space_role(&auth, &space_id, &group_id, Some(&request.role))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_group_space_role(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((space_id, group_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .set_group_space_role(&auth, &space_id, &group_id, None)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_files(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let include_deleted = query
        .get("include_deleted")
        .is_some_and(|value| value == "true");
    Ok(Json(
        json!({"files":state.database.list_files(&space_id,&auth.user_id,include_deleted)?}),
    ))
}

async fn list_directories(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(json!({
        "folders": state.database.list_directories(&space_id, &auth.user_id)?
    })))
}

async fn download_archive(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state.database.require_permission(
        &space_id,
        &auth.user_id,
        crate::database::Permission::View,
    )?;
    let requested = query.get("path").map(String::as_str).unwrap_or("");
    let folder = if requested.is_empty() {
        String::new()
    } else {
        CloudPath::parse(requested.to_owned())
            .map_err(|error| ApiError::bad_request(error.to_string()))?
            .as_str()
            .to_owned()
    };
    let prefix = if folder.is_empty() {
        String::new()
    } else {
        format!("{folder}/")
    };
    let files = state
        .database
        .list_files(&space_id, &auth.user_id, false)?
        .into_iter()
        .filter(|file| folder.is_empty() || file.path.starts_with(&prefix))
        .collect::<Vec<_>>();
    let directories = state
        .database
        .list_directories(&space_id, &auth.user_id)?
        .into_iter()
        .filter(|directory| {
            folder.is_empty() || directory.path == folder || directory.path.starts_with(&prefix)
        })
        .collect::<Vec<_>>();
    if !folder.is_empty() && files.is_empty() && directories.is_empty() {
        return Err(DbError::NotFound.into());
    }

    let temporary = tempfile::tempfile().map_err(ApiError::internal)?;
    let mut archive = zip::ZipWriter::new(temporary);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let directory_options = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
    for directory in directories {
        let relative = if folder.is_empty() {
            directory.path
        } else if directory.path == folder {
            continue;
        } else {
            directory.path[prefix.len()..].to_owned()
        };
        if !relative.is_empty() {
            archive
                .add_directory(format!("{relative}/"), directory_options)
                .map_err(ApiError::internal)?;
        }
    }
    for file in files {
        let (_, _, blocks) = state.database.file_blocks(&file.id, &auth.user_id)?;
        let relative = if folder.is_empty() {
            file.path
        } else {
            file.path[prefix.len()..].to_owned()
        };
        archive
            .start_file(relative, options)
            .map_err(ApiError::internal)?;
        for block in blocks {
            let bytes = state
                .blocks
                .read(&space_id, &block.hash)
                .await
                .map_err(ApiError::internal)?;
            if bytes.len() != block.size as usize
                || blake3::hash(&bytes).to_hex().as_str() != block.hash
            {
                return Err(ApiError::internal(anyhow::anyhow!(
                    "Block {} ist beschädigt; das Archiv wurde nicht ausgeliefert",
                    block.hash
                )));
            }
            archive.write_all(&bytes).map_err(ApiError::internal)?;
        }
    }
    let mut std_file = archive.finish().map_err(ApiError::internal)?;
    std_file
        .seek(SeekFrom::Start(0))
        .map_err(ApiError::internal)?;
    let file = tokio::fs::File::from_std(std_file);
    let mut response = Response::new(Body::from_stream(ReaderStream::new(file)));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    let archive_name = folder
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("Cloud-Dateien")
        .replace(['"', '\r', '\n'], "_");
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{archive_name}.zip\""))
            .map_err(ApiError::internal)?,
    );
    Ok(response)
}

fn validated_folder_restore_input(path: String, at: String) -> Result<(String, String), ApiError> {
    let path = if path.is_empty() {
        path
    } else {
        CloudPath::parse(path)
            .map_err(|error| ApiError::bad_request(error.to_string()))?
            .as_str()
            .to_owned()
    };
    let at = chrono::DateTime::parse_from_rfc3339(&at)
        .map_err(|_| ApiError::bad_request("Zeitpunkt ist ungültig"))?
        .with_timezone(&chrono::Utc)
        .to_rfc3339();
    if at > chrono::Utc::now().to_rfc3339() {
        return Err(ApiError::bad_request("Zeitpunkt liegt in der Zukunft"));
    }
    Ok((path, at))
}

async fn folder_restore_preview(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let (path, at) = validated_folder_restore_input(
        query.get("path").cloned().unwrap_or_default(),
        query
            .get("at")
            .cloned()
            .ok_or_else(|| ApiError::bad_request("Zeitpunkt fehlt"))?,
    )?;
    Ok(Json(serde_json::to_value(
        state
            .database
            .folder_restore_preview(&auth, &space_id, &path, &at)?,
    )?))
}

#[derive(Debug, Deserialize)]
struct RestoreFolderRequest {
    path: String,
    at: String,
}

async fn restore_folder(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Json(request): Json<RestoreFolderRequest>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let (path, at) = validated_folder_restore_input(request.path, request.at)?;
    Ok(Json(serde_json::to_value(
        state
            .database
            .restore_folder_at(&auth, &space_id, &path, &at)?,
    )?))
}

#[derive(Debug, Deserialize)]
struct CreateDirectoryRequest {
    path: String,
}

async fn create_directory(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Json(request): Json<CreateDirectoryRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let auth = authenticate(&state, &headers)?;
    let path =
        CloudPath::parse(request.path).map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(state.database.create_directory(
            &auth,
            &space_id,
            path.as_str(),
        )?)?),
    ))
}

async fn delete_directory(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(directory_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(json!({
        "deleted_files": state.database.delete_directory(&auth, &directory_id)?
    })))
}

async fn move_directory(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(directory_id): Path<String>,
    Json(request): Json<MoveRequest>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let path =
        CloudPath::parse(request.path).map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(serde_json::to_value(state.database.move_directory(
        &auth,
        &directory_id,
        path.as_str(),
    )?)?))
}

async fn space_usage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(serde_json::to_value(
        state.database.space_usage(&auth, &space_id)?,
    )?))
}

#[derive(Debug, Deserialize)]
struct CreateWebhookRequest {
    url: String,
}

async fn create_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Json(request): Json<CreateWebhookRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let auth = authenticate(&state, &headers)?;
    let created = state
        .database
        .create_webhook(&auth, &space_id, &request.url)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "webhook":created.webhook,"signing_secret":created.signing_secret,
        })),
    ))
}

async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(
        json!({"webhooks":state.database.list_webhooks(&auth,&space_id)?}),
    ))
}

async fn disable_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((space_id, webhook_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .disable_webhook(&auth, &space_id, &webhook_id)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct StartUploadRequest {
    path: String,
    file_id: Option<String>,
    base_version_id: Option<String>,
    size: i64,
    blocks: Vec<StoredBlock>,
    idempotency_key: String,
}

#[derive(Debug, Serialize)]
struct StartUploadResponse {
    upload_id: String,
    missing_blocks: Vec<String>,
    expires_at: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct StartUploadBatchRequest {
    uploads: Vec<StartUploadRequest>,
}

#[derive(Debug, Serialize)]
struct StartUploadBatchResponse {
    uploads: Vec<StartUploadResponse>,
}

async fn start_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Json(request): Json<StartUploadRequest>,
) -> Result<(StatusCode, Json<StartUploadResponse>), ApiError> {
    let auth = authenticate(&state, &headers)?;
    if request.size < 0 {
        return Err(ApiError::bad_request("Dateigröße darf nicht negativ sein"));
    }
    let cloud_path =
        CloudPath::parse(request.path).map_err(|error| ApiError::bad_request(error.to_string()))?;
    state.database.ensure_upload_allowed(cloud_path.as_str())?;
    let upload = state.database.create_upload(
        &auth,
        &space_id,
        cloud_path.as_str(),
        request.file_id.as_deref(),
        request.base_version_id.as_deref(),
        request.size,
        &request.blocks,
        &request.idempotency_key,
    )?;
    let mut missing_blocks = Vec::new();
    for block in &upload.blocks {
        if !state
            .blocks
            .contains(&space_id, &block.hash, block.size)
            .await
            .map_err(ApiError::internal)?
        {
            missing_blocks.push(block.hash.clone());
        }
    }
    Ok((
        StatusCode::CREATED,
        Json(StartUploadResponse {
            upload_id: upload.id,
            missing_blocks,
            expires_at: upload.expires_at,
            status: upload.status,
        }),
    ))
}

async fn start_upload_batch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(space_id): Path<String>,
    Json(request): Json<StartUploadBatchRequest>,
) -> Result<(StatusCode, Json<StartUploadBatchResponse>), ApiError> {
    let auth = authenticate(&state, &headers)?;
    if request.uploads.is_empty() || request.uploads.len() > 1_024 {
        return Err(ApiError::bad_request(
            "Ein Upload-Batch muss 1 bis 1.024 Dateien enthalten",
        ));
    }
    if request
        .uploads
        .iter()
        .map(|upload| upload.blocks.len())
        .sum::<usize>()
        > 16_384
    {
        return Err(ApiError::bad_request(
            "Upload-Batch enthält zu viele Blöcke",
        ));
    }
    let mut inputs = Vec::with_capacity(request.uploads.len());
    for upload in request.uploads {
        if upload.size < 0 {
            return Err(ApiError::bad_request("Dateigröße darf nicht negativ sein"));
        }
        let path = CloudPath::parse(upload.path)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        state.database.ensure_upload_allowed(path.as_str())?;
        inputs.push(NewUpload {
            path: path.as_str().into(),
            file_id: upload.file_id,
            base_version_id: upload.base_version_id,
            size: upload.size,
            blocks: upload.blocks,
            idempotency_key: upload.idempotency_key,
        });
    }
    let uploads = state.database.create_uploads(&auth, &space_id, &inputs)?;
    let mut responses = Vec::with_capacity(uploads.len());
    for upload in uploads {
        let mut missing_blocks = Vec::new();
        for block in &upload.blocks {
            if !state
                .blocks
                .contains(&space_id, &block.hash, block.size)
                .await
                .map_err(ApiError::internal)?
            {
                missing_blocks.push(block.hash.clone());
            }
        }
        responses.push(StartUploadResponse {
            upload_id: upload.id,
            missing_blocks,
            expires_at: upload.expires_at,
            status: upload.status,
        });
    }
    Ok((
        StatusCode::CREATED,
        Json(StartUploadBatchResponse { uploads: responses }),
    ))
}

async fn put_block(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((upload_id, hash)): Path<(String, String)>,
    bytes: Bytes,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let upload = state.database.get_upload(&upload_id, &auth)?;
    if upload.status != "open" {
        return Err(ApiError::conflict("Upload ist bereits geschlossen"));
    }
    let expected = upload
        .blocks
        .iter()
        .find(|block| block.hash == hash)
        .ok_or_else(|| ApiError::bad_request("Block gehört nicht zu diesem Upload"))?;
    if bytes.len() != expected.size as usize {
        return Err(ApiError::bad_request("Blockgröße stimmt nicht"));
    }
    state
        .blocks
        .put_verified(&upload.space_id, &hash, &bytes)
        .await
        .map_err(ApiError::block_write)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_block_batch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let mut cursor = 0_usize;
    if bytes.get(..4) != Some(b"HDB1") {
        return Err(ApiError::bad_request("Ungültiges Block-Batch-Format"));
    }
    cursor += 4;
    let count = read_batch_u32(&bytes, &mut cursor)? as usize;
    if count == 0 || count > 512 {
        return Err(ApiError::bad_request(
            "Ein Block-Batch muss 1 bis 512 Blöcke enthalten",
        ));
    }
    let mut upload_cache = HashMap::new();
    let mut blocks = Vec::with_capacity(count);
    for _ in 0..count {
        let upload_length = read_batch_u16(&bytes, &mut cursor)? as usize;
        if upload_length == 0 || upload_length > 200 {
            return Err(ApiError::bad_request("Ungültige Upload-ID im Block-Batch"));
        }
        let upload_id = read_batch_text(&bytes, &mut cursor, upload_length)?;
        let hash = read_batch_text(&bytes, &mut cursor, 64)?;
        let data_length = read_batch_u32(&bytes, &mut cursor)? as usize;
        if data_length > hedwig_core::MAX_CHUNK_SIZE as usize {
            return Err(ApiError::bad_request("Block im Batch ist zu groß"));
        }
        let end = cursor
            .checked_add(data_length)
            .ok_or_else(|| ApiError::bad_request("Ungültige Blocklänge"))?;
        let data = bytes
            .get(cursor..end)
            .ok_or_else(|| ApiError::bad_request("Block-Batch ist unvollständig"))?
            .to_vec();
        cursor = end;
        let upload = match upload_cache.get(&upload_id) {
            Some(upload) => upload,
            None => {
                let upload = state.database.get_upload(&upload_id, &auth)?;
                upload_cache.insert(upload_id.clone(), upload);
                upload_cache.get(&upload_id).expect("just inserted")
            }
        };
        if upload.status != "open" {
            return Err(ApiError::conflict("Upload ist bereits geschlossen"));
        }
        let expected = upload
            .blocks
            .iter()
            .find(|block| block.hash == hash)
            .ok_or_else(|| ApiError::bad_request("Block gehört nicht zu diesem Upload"))?;
        if data.len() != expected.size as usize {
            return Err(ApiError::bad_request("Blockgröße stimmt nicht"));
        }
        blocks.push((upload.space_id.clone(), hash, data));
    }
    if cursor != bytes.len() {
        return Err(ApiError::bad_request(
            "Block-Batch enthält unerwartete Restdaten",
        ));
    }
    state
        .blocks
        .put_verified_batch(blocks)
        .await
        .map_err(ApiError::block_write)?;
    Ok(StatusCode::NO_CONTENT)
}

fn read_batch_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, ApiError> {
    let end = cursor
        .checked_add(2)
        .ok_or_else(|| ApiError::bad_request("Ungültige Block-Batch-Länge"))?;
    let value = bytes
        .get(*cursor..end)
        .ok_or_else(|| ApiError::bad_request("Block-Batch ist unvollständig"))?;
    *cursor = end;
    Ok(u16::from_be_bytes([value[0], value[1]]))
}

fn read_batch_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, ApiError> {
    let end = cursor
        .checked_add(4)
        .ok_or_else(|| ApiError::bad_request("Ungültige Block-Batch-Länge"))?;
    let value = bytes
        .get(*cursor..end)
        .ok_or_else(|| ApiError::bad_request("Block-Batch ist unvollständig"))?;
    *cursor = end;
    Ok(u32::from_be_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_batch_text(bytes: &[u8], cursor: &mut usize, length: usize) -> Result<String, ApiError> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| ApiError::bad_request("Ungültige Block-Batch-Länge"))?;
    let value = bytes
        .get(*cursor..end)
        .ok_or_else(|| ApiError::bad_request("Block-Batch ist unvollständig"))?;
    *cursor = end;
    std::str::from_utf8(value)
        .map(str::to_owned)
        .map_err(|_| ApiError::bad_request("Block-Batch enthält ungültigen Text"))
}

async fn commit_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(upload_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let upload = state.database.get_upload(&upload_id, &auth)?;
    for block in &upload.blocks {
        if !state
            .blocks
            .contains(&upload.space_id, &block.hash, block.size)
            .await
            .map_err(ApiError::internal)?
        {
            return Err(ApiError::conflict(format!("Block {} fehlt", block.hash)));
        }
    }
    Ok(Json(serde_json::to_value(
        state.database.commit_upload(&upload_id, &auth)?,
    )?))
}

#[derive(Debug, Deserialize)]
struct CommitUploadBatchRequest {
    upload_ids: Vec<String>,
}

async fn commit_upload_batch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CommitUploadBatchRequest>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    if request.upload_ids.is_empty() || request.upload_ids.len() > 1_024 {
        return Err(ApiError::bad_request(
            "Ein Commit-Batch muss 1 bis 1.024 Uploads enthalten",
        ));
    }
    let mut seen = std::collections::HashSet::with_capacity(request.upload_ids.len());
    for upload_id in &request.upload_ids {
        if !seen.insert(upload_id) {
            return Err(ApiError::bad_request("Upload-ID ist im Batch doppelt"));
        }
        let upload = state.database.get_upload(upload_id, &auth)?;
        for block in &upload.blocks {
            if !state
                .blocks
                .contains(&upload.space_id, &block.hash, block.size)
                .await
                .map_err(ApiError::internal)?
            {
                return Err(ApiError::conflict(format!("Block {} fehlt", block.hash)));
            }
        }
    }
    Ok(Json(serde_json::to_value(
        state.database.commit_uploads(&request.upload_ids, &auth)?,
    )?))
}

async fn download_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
) -> Result<Response, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let (space_id, path, blocks) = state.database.file_blocks(&file_id, &auth.user_id)?;
    download_blocks(&state, &space_id, &path, blocks).await
}

async fn file_manifest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let (space_id, path, blocks) = state.database.file_blocks(&file_id, &auth.user_id)?;
    let file = state
        .database
        .list_files(&space_id, &auth.user_id, false)?
        .into_iter()
        .find(|file| file.id == file_id)
        .ok_or(DbError::NotFound)?;
    Ok(Json(
        json!({"file_id":file.id,"version_id":file.current_version_id,"path":path,"size":file.size,"blocks":blocks}),
    ))
}

async fn list_versions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(
        json!({"versions":state.database.list_versions(&file_id,&auth.user_id)?}),
    ))
}

async fn download_version(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((file_id, version_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let (space_id, path, blocks) =
        state
            .database
            .file_version_blocks(&file_id, &version_id, &auth.user_id)?;
    download_blocks(&state, &space_id, &path, blocks).await
}

async fn restore_version(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((file_id, version_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(serde_json::to_value(
        state
            .database
            .restore_version(&file_id, &version_id, &auth)?,
    )?))
}

#[derive(Debug, Deserialize)]
struct PinVersionRequest {
    name: Option<String>,
}

async fn pin_version(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((file_id, version_id)): Path<(String, String)>,
    Json(request): Json<PinVersionRequest>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state
        .database
        .pin_version(&file_id, &version_id, request.name.as_deref(), &auth)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn download_blocks(
    state: &AppState,
    space_id: &str,
    path: &str,
    blocks: Vec<StoredBlock>,
) -> Result<Response, ApiError> {
    let content_length: u64 = blocks.iter().map(|block| u64::from(block.size)).sum();
    let store = state.blocks.clone();
    let stream_space = space_id.to_owned();
    // Stream verified blocks directly. The receiving client still writes to a temporary file and
    // verifies the immutable manifest before the final atomic rename, so a read or hash failure
    // can only leave a disposable partial file. This removes a complete server-side temp-file
    // write/read cycle and starts delivering bytes as soon as the first block is verified.
    let stream = async_stream::stream! {
        for block in blocks {
            let bytes = match store.read(&stream_space, &block.hash).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    yield Err::<Bytes, std::io::Error>(std::io::Error::other(format!(
                        "Block {} konnte nicht gelesen werden: {error}", block.hash
                    )));
                    break;
                }
            };
            if bytes.len() != block.size as usize
                || blake3::hash(&bytes).to_hex().as_str() != block.hash
            {
                yield Err::<Bytes, std::io::Error>(std::io::Error::other(format!(
                    "Block {} ist beschädigt; die Datei wurde nicht vollständig ausgeliefert",
                    block.hash
                )));
                break;
            }
            yield Ok::<Bytes, std::io::Error>(Bytes::from(bytes));
        }
    };
    let file_name = path
        .rsplit('/')
        .next()
        .unwrap_or("download")
        .replace(['"', '\r', '\n'], "_");
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{file_name}\""))
            .map_err(ApiError::internal)?,
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&content_length.to_string()).map_err(ApiError::internal)?,
    );
    Ok(response)
}

#[derive(Debug, Deserialize)]
struct MoveRequest {
    path: String,
}
#[derive(Debug, Deserialize)]
struct CopyRequest {
    path: String,
    space_id: Option<String>,
}
async fn move_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
    Json(request): Json<MoveRequest>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let path =
        CloudPath::parse(request.path).map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(serde_json::to_value(state.database.move_file(
        &file_id,
        path.as_str(),
        &auth,
    )?)?))
}
async fn copy_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
    Json(request): Json<CopyRequest>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let path =
        CloudPath::parse(request.path).map_err(|error| ApiError::bad_request(error.to_string()))?;
    let (source_space, _, blocks) = state.database.file_blocks(&file_id, &auth.user_id)?;
    let target_space = request.space_id.as_deref().unwrap_or(&source_space);
    state.database.ensure_upload_allowed(path.as_str())?;
    if target_space == source_space {
        return Ok(Json(serde_json::to_value(state.database.copy_file(
            &file_id,
            path.as_str(),
            &auth,
        )?)?));
    }
    let size: i64 = blocks.iter().map(|block| i64::from(block.size)).sum();
    let upload = state.database.create_upload(
        &auth,
        target_space,
        path.as_str(),
        None,
        None,
        size,
        &blocks,
        &format!("cross-space-copy-{}", uuid::Uuid::now_v7()),
    )?;
    for block in &blocks {
        let content = state
            .blocks
            .read(&source_space, &block.hash)
            .await
            .map_err(ApiError::internal)?;
        state
            .blocks
            .put_verified(target_space, &block.hash, &content)
            .await
            .map_err(ApiError::block_write)?;
    }
    Ok(Json(serde_json::to_value(
        state.database.commit_upload(&upload.id, &auth)?,
    )?))
}
async fn delete_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(serde_json::to_value(
        state.database.delete_file(&file_id, &auth)?,
    )?))
}
async fn restore_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(serde_json::to_value(
        state.database.restore_file(&file_id, &auth)?,
    )?))
}

async fn list_changes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    let after = query
        .get("after")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let limit = query
        .get("limit")
        .and_then(|value| value.parse().ok())
        .unwrap_or(250);
    let changes = state.database.list_changes(&auth.user_id, after, limit)?;
    let next_cursor = changes.last().map_or(after, |change| change.sequence);
    Ok(Json(json!({"changes":changes,"next_cursor":next_cursor})))
}

#[derive(Debug, Deserialize)]
struct CreateShareRequest {
    space_id: String,
    target_path: String,
    mode: String,
    password: Option<String>,
    expires_at: Option<String>,
    max_upload_size: Option<i64>,
}

async fn create_share(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateShareRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let auth = authenticate(&state, &headers)?;
    let target = CloudPath::parse(request.target_path)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let created = state.database.create_share(
        &auth,
        &request.space_id,
        target.as_str(),
        &request.mode,
        request.password.as_deref(),
        request.expires_at.as_deref(),
        request.max_upload_size,
    )?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "share": created.share,
            "token": created.token,
            "url_fragment": format!("#/share/{}", created.token),
        })),
    ))
}

async fn list_shares(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    Ok(Json(json!({"shares": state.database.list_shares(&auth)?})))
}

async fn revoke_share(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(share_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticate(&state, &headers)?;
    state.database.revoke_share(&auth, &share_id)?;
    Ok(StatusCode::NO_CONTENT)
}

fn authenticate_share(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<crate::database::ShareAccess, ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Share "))
        .ok_or(DbError::Unauthorized)?;
    let password = headers
        .get("x-share-password")
        .and_then(|value| value.to_str().ok());
    Ok(state.database.authenticate_share(token, password)?)
}

async fn public_share(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let access = authenticate_share(&state, &headers)?;
    let files = state.database.share_files(&access)?;
    Ok(Json(json!({
        "share": access.share,
        "files": files,
    })))
}

async fn public_download_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
) -> Result<Response, ApiError> {
    let access = authenticate_share(&state, &headers)?;
    let (space_id, path, blocks) = state.database.share_file_blocks(&access, &file_id)?;
    download_blocks(&state, &space_id, &path, blocks).await
}

#[derive(Debug, Deserialize)]
struct PublicStartUploadRequest {
    path: String,
    size: i64,
    blocks: Vec<StoredBlock>,
    idempotency_key: String,
}

async fn public_start_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<PublicStartUploadRequest>,
) -> Result<(StatusCode, Json<StartUploadResponse>), ApiError> {
    let access = authenticate_share(&state, &headers)?;
    if access.share.mode != "upload" {
        return Err(DbError::Forbidden.into());
    }
    if request.size < 0 {
        return Err(ApiError::bad_request("Dateigröße darf nicht negativ sein"));
    }
    if access
        .share
        .max_upload_size
        .is_some_and(|maximum| request.size > maximum)
    {
        return Err(ApiError::bad_request(
            "Datei überschreitet das Freigabelimit",
        ));
    }
    let relative =
        CloudPath::parse(request.path).map_err(|error| ApiError::bad_request(error.to_string()))?;
    let path = format!("{}/{}", access.share.target_path, relative.as_str());
    state.database.ensure_upload_allowed(&path)?;
    let upload = state.database.create_upload(
        &access.auth,
        &access.share.space_id,
        &path,
        None,
        None,
        request.size,
        &request.blocks,
        &request.idempotency_key,
    )?;
    let mut missing_blocks = Vec::new();
    for block in &upload.blocks {
        if !state
            .blocks
            .contains(&upload.space_id, &block.hash, block.size)
            .await
            .map_err(ApiError::internal)?
        {
            missing_blocks.push(block.hash.clone());
        }
    }
    Ok((
        StatusCode::CREATED,
        Json(StartUploadResponse {
            upload_id: upload.id,
            missing_blocks,
            expires_at: upload.expires_at,
            status: upload.status,
        }),
    ))
}

async fn public_put_block(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((upload_id, hash)): Path<(String, String)>,
    bytes: Bytes,
) -> Result<StatusCode, ApiError> {
    let access = authenticate_share(&state, &headers)?;
    if access.share.mode != "upload" {
        return Err(DbError::Forbidden.into());
    }
    let upload = state.database.get_upload(&upload_id, &access.auth)?;
    if upload.space_id != access.share.space_id
        || !upload
            .path
            .starts_with(&format!("{}/", access.share.target_path))
    {
        return Err(DbError::Forbidden.into());
    }
    let expected = upload
        .blocks
        .iter()
        .find(|block| block.hash == hash)
        .ok_or_else(|| ApiError::bad_request("Block gehört nicht zu diesem Upload"))?;
    if bytes.len() != expected.size as usize {
        return Err(ApiError::bad_request("Blockgröße stimmt nicht"));
    }
    state
        .blocks
        .put_verified(&upload.space_id, &hash, &bytes)
        .await
        .map_err(ApiError::block_write)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn public_commit_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(upload_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let access = authenticate_share(&state, &headers)?;
    if access.share.mode != "upload" {
        return Err(DbError::Forbidden.into());
    }
    let upload = state.database.get_upload(&upload_id, &access.auth)?;
    if upload.space_id != access.share.space_id
        || !upload
            .path
            .starts_with(&format!("{}/", access.share.target_path))
    {
        return Err(DbError::Forbidden.into());
    }
    for block in &upload.blocks {
        if !state
            .blocks
            .contains(&upload.space_id, &block.hash, block.size)
            .await
            .map_err(ApiError::internal)?
        {
            return Err(ApiError::conflict(format!("Block {} fehlt", block.hash)));
        }
    }
    Ok(Json(serde_json::to_value(
        state.database.commit_upload(&upload_id, &access.auth)?,
    )?))
}

fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthContext, ApiError> {
    Ok(state.database.authenticate(bearer_token(headers)?)?)
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| DbError::Unauthorized.into())
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request",
            message: message.into(),
        }
    }
    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "conflict",
            message: message.into(),
        }
    }
    fn block_write(error: anyhow::Error) -> Self {
        if Self::is_storage_full(&error) {
            tracing::error!(%error, "block write failed because storage is full");
            Self::storage_full()
        } else {
            Self::bad_request(error.to_string())
        }
    }
    fn is_storage_full(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io_error| {
                    io_error.kind() == std::io::ErrorKind::StorageFull
                        || io_error.raw_os_error() == Some(28)
                })
                || cause
                    .downcast_ref::<rusqlite::Error>()
                    .is_some_and(|database_error| {
                        matches!(
                            database_error,
                            rusqlite::Error::SqliteFailure(code, _)
                                if code.code == rusqlite::ErrorCode::DiskFull
                        )
                    })
        })
    }
    fn storage_full() -> Self {
        Self {
            status: StatusCode::INSUFFICIENT_STORAGE,
            code: "storage_full",
            message:
                "Der Serverdatenträger ist voll; die letzte bestätigte Version bleibt erhalten"
                    .into(),
        }
    }
    fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(error=%error,"request failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: "Interner Serverfehler".into(),
        }
    }
}

impl From<DbError> for ApiError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::Unauthorized | DbError::InvalidLogin => Self {
                status: StatusCode::UNAUTHORIZED,
                code: "unauthorized",
                message: error.to_string(),
            },
            DbError::Forbidden => Self {
                status: StatusCode::FORBIDDEN,
                code: "forbidden",
                message: error.to_string(),
            },
            DbError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message: error.to_string(),
            },
            DbError::AlreadySetup | DbError::Conflict(_) => Self {
                status: StatusCode::CONFLICT,
                code: "conflict",
                message: error.to_string(),
            },
            DbError::Invalid(_) => Self::bad_request(error.to_string()),
            DbError::Internal(inner) if Self::is_storage_full(&inner) => {
                tracing::error!(error = %inner, "database write failed because storage is full");
                Self::storage_full()
            }
            DbError::Internal(inner) => Self::internal(inner),
        }
    }
}
impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::internal(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"error":{"code":self.code,"message":self.message}})),
        )
            .into_response()
    }
}
