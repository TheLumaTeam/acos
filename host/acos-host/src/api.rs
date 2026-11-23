//! Versioned local HTTP API and durable computer registry.
#![allow(clippy::result_large_err)] // Axum handlers return concrete HTTP error responses.

use crate::{
    BackendError, CommandBackend, Computer, ComputerBackend, ComputerId, ComputerKind,
    CreateComputer, FileBackend, GuestCommandRequest, GuestCommandResult, ResourceLimits,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    backend: Arc<dyn ComputerBackend>,
    commands: Arc<dyn CommandBackend>,
    guest_info: Arc<crate::qemu::QemuBackend>,
    files: Arc<dyn FileBackend>,
    token: Arc<String>,
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
    request_id: String,
}
#[derive(Serialize)]
struct ErrorDetail {
    code: String,
    message: String,
}
#[derive(Deserialize)]
struct CreateRequest {
    name: String,
    kind: ComputerKind,
    image: Option<String>,
    resources: ResourceLimits,
}

fn error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: ErrorDetail {
                code: code.into(),
                message: message.into(),
            },
            request_id: uuid::Uuid::new_v4().to_string(),
        }),
    )
        .into_response()
}

async fn authorize(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let valid = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|v| v == state.token.as_str());
    if !valid {
        return error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "A valid bearer token is required",
        );
    }
    next.run(request).await
}

fn db(state: &AppState) -> Result<std::sync::MutexGuard<'_, Connection>, Response> {
    state.db.lock().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "database_unavailable",
            "database lock is poisoned",
        )
    })
}
fn persist(connection: &Connection, computer: &Computer) -> Result<(), Response> {
    let json = serde_json::to_string(computer).map_err(|e| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            e.to_string(),
        )
    })?;
    connection.execute("INSERT INTO computers(id, data) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data", params![computer.id.as_str(), json])
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, "database_error", e.to_string()))?;
    Ok(())
}
fn load(connection: &Connection, id: &ComputerId) -> Result<Computer, Response> {
    let json: String = connection
        .query_row(
            "SELECT data FROM computers WHERE id=?1",
            [id.as_str()],
            |row| row.get(0),
        )
        .map_err(|e| {
            if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                error(
                    StatusCode::NOT_FOUND,
                    "computer_not_found",
                    "computer does not exist",
                )
            } else {
                error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database_error",
                    e.to_string(),
                )
            }
        })?;
    serde_json::from_str(&json).map_err(|e| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "database_error",
            e.to_string(),
        )
    })
}
fn backend_error(e: BackendError) -> Response {
    let status = match e.code.as_str() {
        "computer_not_found" => StatusCode::NOT_FOUND,
        "invalid_image" | "invalid_guest_path" | "file_too_large" => StatusCode::BAD_REQUEST,
        _ => StatusCode::BAD_GATEWAY,
    };
    error(status, &e.code, e.message)
}

async fn capabilities() -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"capabilities":["computer.create","computer.read","computer.start","computer.stop","computer.pause","computer.resume","computer.delete","command.execute","files.read","files.write","guest.system_info","backend.qemu","guest_agent.qga"]}),
    )
}
async fn command(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    Json(request): Json<GuestCommandRequest>,
) -> Result<Json<GuestCommandResult>, Response> {
    let id = ComputerId::new(raw)
        .map_err(|e| error(StatusCode::BAD_REQUEST, "invalid_id", e.to_string()))?;
    let connection = db(&state)?;
    let _ = load(&connection, &id)?;
    let result = state
        .commands
        .execute(&id, &request)
        .map_err(backend_error)?;
    Ok(Json(result))
}
async fn agent_info(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<Json<serde_json::Value>, Response> {
    let id = ComputerId::new(raw)
        .map_err(|e| error(StatusCode::BAD_REQUEST, "invalid_id", e.to_string()))?;
    let connection = db(&state)?;
    let computer = load(&connection, &id)?;
    let info = state.guest_info.agent_info(&id).map_err(backend_error)?;
    Ok(Json(
        serde_json::json!({"computer":computer,"guest":info,"allocated_resources":computer.resources}),
    ))
}
async fn read_guest_file(
    State(state): State<AppState>,
    Path((raw, path)): Path<(String, String)>,
) -> Result<([(axum::http::HeaderName, &'static str); 1], Vec<u8>), Response> {
    let id = ComputerId::new(raw)
        .map_err(|e| error(StatusCode::BAD_REQUEST, "invalid_id", e.to_string()))?;
    let connection = db(&state)?;
    let _ = load(&connection, &id)?;
    let contents = state
        .files
        .read_file(&id, &format!("/workspace/{path}"))
        .map_err(backend_error)?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
        contents,
    ))
}
async fn write_guest_file(
    State(state): State<AppState>,
    Path((raw, path)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> Result<StatusCode, Response> {
    let id = ComputerId::new(raw)
        .map_err(|e| error(StatusCode::BAD_REQUEST, "invalid_id", e.to_string()))?;
    let connection = db(&state)?;
    let _ = load(&connection, &id)?;
    state
        .files
        .write_file(&id, &format!("/workspace/{path}"), &body)
        .map_err(backend_error)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn list(State(state): State<AppState>) -> Result<Json<serde_json::Value>, Response> {
    let connection = db(&state)?;
    let mut statement = connection
        .prepare("SELECT data FROM computers ORDER BY id")
        .map_err(|e| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "database_error",
                e.to_string(),
            )
        })?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "database_error",
                e.to_string(),
            )
        })?;
    let mut items = Vec::new();
    for row in rows {
        let json = row.map_err(|e| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "database_error",
                e.to_string(),
            )
        })?;
        let saved = serde_json::from_str::<Computer>(&json).map_err(|e| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "database_error",
                e.to_string(),
            )
        })?;
        let observed = match state.backend.inspect(&saved.id) {
            Ok(computer) => computer,
            Err(_) => {
                let mut uncertain = saved;
                uncertain.state = crate::ComputerState::Unknown;
                uncertain
            }
        };
        items.push(observed);
    }
    Ok(Json(serde_json::json!({"items":items})))
}
async fn create(
    State(state): State<AppState>,
    Json(request): Json<CreateRequest>,
) -> Result<(StatusCode, Json<Computer>), Response> {
    if request.name.trim().is_empty() || request.name.chars().count() > 128 {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_name",
            "name must contain 1-128 characters",
        ));
    }
    request
        .resources
        .validate()
        .map_err(|e| error(StatusCode::BAD_REQUEST, "invalid_resources", e.to_string()))?;
    if request.kind != ComputerKind::VirtualMachine {
        return Err(error(
            StatusCode::NOT_IMPLEMENTED,
            "capability_not_supported",
            "OCI containers are not implemented yet",
        ));
    }
    let image = request.image.ok_or_else(|| {
        error(
            StatusCode::BAD_REQUEST,
            "image_required",
            "a base QCOW2 image filename is required",
        )
    })?;
    let id = ComputerId::new(uuid::Uuid::new_v4().to_string()).map_err(|e| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "id_generation_failed",
            e.to_string(),
        )
    })?;
    let create = CreateComputer {
        id: id.clone(),
        name: request.name,
        kind: request.kind,
        resources: request.resources,
        image,
    };
    let computer = state.backend.create(&create).map_err(backend_error)?;
    let connection = db(&state)?;
    persist(&connection, &computer)?;
    Ok((StatusCode::CREATED, Json(computer)))
}
async fn get_computer(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<Json<Computer>, Response> {
    let id = ComputerId::new(raw)
        .map_err(|e| error(StatusCode::BAD_REQUEST, "invalid_id", e.to_string()))?;
    let connection = db(&state)?;
    let saved = load(&connection, &id)?;
    let computer = match state.backend.inspect(&id) {
        Ok(computer) => computer,
        Err(_) => {
            let mut uncertain = saved;
            uncertain.state = crate::ComputerState::Unknown;
            uncertain
        }
    };
    persist(&connection, &computer)?;
    Ok(Json(computer))
}
async fn lifecycle(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    operation: &'static str,
) -> Result<(StatusCode, Json<Computer>), Response> {
    let id = ComputerId::new(raw)
        .map_err(|e| error(StatusCode::BAD_REQUEST, "invalid_id", e.to_string()))?;
    let connection = db(&state)?;
    let _ = load(&connection, &id)?;
    let computer = match operation {
        "start" => state.backend.start(&id),
        "stop" => state.backend.stop(&id),
        "pause" => state.backend.pause(&id),
        _ => state.backend.resume(&id),
    }
    .map_err(backend_error)?;
    persist(&connection, &computer)?;
    Ok((StatusCode::ACCEPTED, Json(computer)))
}
async fn start(
    s: State<AppState>,
    p: Path<String>,
) -> Result<(StatusCode, Json<Computer>), Response> {
    lifecycle(s, p, "start").await
}
async fn stop(
    s: State<AppState>,
    p: Path<String>,
) -> Result<(StatusCode, Json<Computer>), Response> {
    lifecycle(s, p, "stop").await
}
async fn pause(
    s: State<AppState>,
    p: Path<String>,
) -> Result<(StatusCode, Json<Computer>), Response> {
    lifecycle(s, p, "pause").await
}
async fn resume(
    s: State<AppState>,
    p: Path<String>,
) -> Result<(StatusCode, Json<Computer>), Response> {
    lifecycle(s, p, "resume").await
}
async fn delete(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<StatusCode, Response> {
    let id = ComputerId::new(raw)
        .map_err(|e| error(StatusCode::BAD_REQUEST, "invalid_id", e.to_string()))?;
    let connection = db(&state)?;
    let _ = load(&connection, &id)?;
    state.backend.delete(&id).map_err(backend_error)?;
    connection
        .execute("DELETE FROM computers WHERE id=?1", [id.as_str()])
        .map_err(|e| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "database_error",
                e.to_string(),
            )
        })?;
    Ok(StatusCode::ACCEPTED)
}

pub fn serve(
    database: PathBuf,
    backend: Arc<crate::qemu::QemuBackend>,
    token: String,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(
        database
            .parent()
            .unwrap_or_else(|| std::path::Path::new(".")),
    )?;
    let connection = Connection::open(database)?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS computers(id TEXT PRIMARY KEY, data TEXT NOT NULL);")?;
    let commands: Arc<dyn CommandBackend> = backend.clone();
    let files: Arc<dyn FileBackend> = backend.clone();
    let guest_info = backend.clone();
    let backend: Arc<dyn ComputerBackend> = backend;
    let state = AppState {
        db: Arc::new(Mutex::new(connection)),
        backend,
        commands,
        guest_info,
        files,
        token: Arc::new(token),
    };
    let protected = Router::new()
        .route("/computers", get(list).post(create))
        .route("/computers/{id}", get(get_computer).delete(delete))
        .route("/computers/{id}/start", post(start))
        .route("/computers/{id}/stop", post(stop))
        .route("/computers/{id}/pause", post(pause))
        .route("/computers/{id}/resume", post(resume))
        .route("/computers/{id}/commands", post(command))
        .route("/computers/{id}/agent", get(agent_info))
        .route(
            "/computers/{id}/files/{*path}",
            get(read_guest_file).put(write_guest_file),
        )
        .layer(DefaultBodyLimit::max(10_485_760))
        .route_layer(middleware::from_fn_with_state(state.clone(), authorize));
    let app = Router::new()
        .route("/v1/capabilities", get(capabilities))
        .nest("/v1", protected)
        .with_state(state);
    let address = std::env::var("ACOS_LISTEN").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let listener = std::net::TcpListener::bind(&address)?;
    listener.set_nonblocking(true)?;
    eprintln!("ACOS API listening on http://{address}/v1");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        axum::serve(tokio::net::TcpListener::from_std(listener)?, app).await
    })?;
    Ok(())
}
