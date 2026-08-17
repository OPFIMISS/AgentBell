use crate::{
    discovery::local_ipv4s,
    model::{AgentEvent, Device, DiscoveredPeer, PendingPair, clean, new_id, now_ms},
    store::Store,
};
use axum::{
    Json, Router,
    extract::{
        ConnectInfo, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use qrcode::{QrCode, render::svg};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::sync::{Mutex, RwLock, broadcast};
use tracing::{info, warn};

pub struct AppState {
    pub store: Mutex<Store>,
    pub pending: RwLock<HashMap<String, PendingPair>>,
    pub pair_secret: String,
    pub tx: broadcast::Sender<AgentEvent>,
    pub discovered: RwLock<HashMap<String, DiscoveredPeer>>,
}

impl AppState {
    pub fn new(store: Store) -> Arc<Self> {
        let (tx, _) = broadcast::channel(128);
        Arc::new(Self {
            store: Mutex::new(store),
            pending: RwLock::new(HashMap::new()),
            pair_secret: new_id().replace('-', ""),
            tx,
            discovered: RwLock::new(HashMap::new()),
        })
    }
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
    after_id: Option<String>,
}

#[derive(Deserialize)]
struct PairRequest {
    name: String,
    pair_secret: Option<String>,
    device_id: Option<String>,
}

#[derive(Serialize)]
struct PairResponse {
    state: String,
    device_id: String,
    token: Option<String>,
    code: Option<String>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct ApproveRequest {
    device_id: String,
    allow: bool,
}

#[derive(Deserialize)]
struct InstallRequest {
    agent: String,
}

#[derive(Deserialize)]
struct DeleteEventsRequest {
    ids: Vec<String>,
}

#[derive(Serialize)]
struct StatusResponse {
    admin: bool,
    authorized: bool,
    device_name: String,
    urls: Vec<String>,
    pair_url: Option<String>,
    pair_svg: Option<String>,
    devices: Vec<Device>,
    pending: Vec<PendingPair>,
    events: Vec<AgentEvent>,
    emitter_token: Option<String>,
    adapters: Vec<AdapterStatus>,
    discovered: Vec<DiscoveredPeer>,
}

#[derive(Serialize)]
struct PollResponse {
    events: Vec<AgentEvent>,
    cursor: Option<String>,
}

#[derive(Serialize)]
struct DiagnosticsResponse {
    log_path: String,
    log_tail: String,
    tray_status: String,
}

fn latest_cursor(store: &Store) -> String {
    store
        .events
        .back()
        .map(|event| event.id.clone())
        .unwrap_or_else(|| "__start__".into())
}

#[derive(Serialize)]
struct AdapterStatus {
    id: &'static str,
    name: &'static str,
    mode: &'static str,
    state: &'static str,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(js))
        .route("/style.css", get(css))
        .route("/manifest.webmanifest", get(manifest))
        .route("/api/status", get(status))
        .route("/api/pair", post(pair))
        .route("/api/pair/approve", post(approve))
        .route("/api/device/revoke", post(revoke))
        .route("/api/events", post(emit_event))
        .route("/api/events/delete", post(delete_events))
        .route("/api/events/poll", get(poll_events))
        .route("/api/test", post(test_event))
        .route("/api/diagnostics", get(diagnostics))
        .route("/api/adapters/install", post(install_adapter))
        .route("/ws", get(ws_upgrade))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../web/index.html"))
}
async fn js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../web/app.js"),
    )
}
async fn css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../web/style.css"),
    )
}
async fn manifest() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        include_str!("../web/manifest.webmanifest"),
    )
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

async fn status(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<TokenQuery>,
    headers: HeaderMap,
) -> Json<StatusResponse> {
    let admin = addr.ip().is_loopback();
    let token = q
        .token
        .as_deref()
        .or_else(|| bearer(&headers))
        .unwrap_or("");
    let store = state.store.lock().await;
    let authorized = admin || store.is_token_trusted(token);
    let urls = local_ipv4s()
        .into_iter()
        .map(|ip| format!("http://{}:{}", ip, store.config.port))
        .collect::<Vec<_>>();
    let pair_url = if admin {
        urls.first()
            .map(|u| format!("{}/?pair={}", u, state.pair_secret))
    } else {
        None
    };
    let pair_svg = pair_url
        .as_ref()
        .and_then(|url| QrCode::new(url).ok())
        .map(|code| {
            code.render::<svg::Color>()
                .min_dimensions(220, 220)
                .dark_color(svg::Color("#182124"))
                .light_color(svg::Color("#ffffff"))
                .build()
        });
    let pending = if admin {
        state.pending.read().await.values().cloned().collect()
    } else {
        vec![]
    };
    Json(StatusResponse {
        admin,
        authorized,
        device_name: store.config.device_name.clone(),
        urls,
        pair_url,
        pair_svg,
        devices: if admin {
            store.config.devices.clone()
        } else {
            vec![]
        },
        pending,
        events: if authorized {
            store.events.iter().rev().cloned().collect()
        } else {
            vec![]
        },
        emitter_token: admin.then(|| store.config.emitter_token.clone()),
        adapters: vec![
            AdapterStatus {
                id: "codex",
                name: "Codex",
                mode: "默认监听 Desktop rollout",
                state: "ready",
            },
            AdapterStatus {
                id: "deepseek-harness-eac",
                name: "Deepseek Harness EAC",
                mode: "默认监听 session.jsonl.zstd",
                state: "ready",
            },
            AdapterStatus {
                id: "claude-code-haha",
                name: "Claude Code Haha",
                mode: "默认监听 Haha 会话终态",
                state: "ready",
            },
            AdapterStatus {
                id: "opencode",
                name: "OpenCode",
                mode: "自动配置 session.idle / session.error 插件",
                state: "ready",
            },
            AdapterStatus {
                id: "openclaw",
                name: "OpenClaw",
                mode: "自动配置 agent_end 插件 Hook",
                state: "ready",
            },
            AdapterStatus {
                id: "hermes-agent",
                name: "Hermes Agent",
                mode: "自动配置 on_session_end 插件 Hook",
                state: "ready",
            },
        ],
        discovered: state.discovered.read().await.values().cloned().collect(),
    })
}

async fn pair(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<PairRequest>,
) -> Response {
    let name = clean(&body.name, 80);
    let client_id = clean(body.device_id.as_deref().unwrap_or(""), 160);
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "设备名不能为空").into_response();
    }
    let trusted = body.pair_secret.as_deref() == Some(state.pair_secret.as_str());
    if !client_id.is_empty() {
        let mut store = state.store.lock().await;
        if let Some(index) = store.config.devices.iter().position(|device| {
            device.client_id == client_id
                || (device.client_id.is_empty()
                    && device.name == name
                    && device.last_ip == addr.ip().to_string())
        }) {
            let cursor = latest_cursor(&store);
            let device = &mut store.config.devices[index];
            device.client_id = client_id.clone();
            device.name = name.clone();
            device.last_ip = addr.ip().to_string();
            device.last_seen_ms = now_ms();
            let response = PairResponse {
                state: "trusted".into(),
                device_id: device.id.clone(),
                token: Some(device.token.clone()),
                code: None,
                cursor: Some(cursor),
            };
            let _ = store.save_config();
            info!(device = %name, client_id = %client_id, "已授权设备复用现有配对");
            return Json(response).into_response();
        }
    }
    let id = new_id();
    let token = format!("phone_{}", new_id().replace('-', ""));
    if trusted {
        let mut store = state.store.lock().await;
        store.config.devices.push(Device {
            id: id.clone(),
            client_id: client_id.clone(),
            name,
            token: token.clone(),
            trusted: true,
            created_ms: now_ms(),
            last_seen_ms: now_ms(),
            last_ip: addr.ip().to_string(),
        });
        if let Err(err) = store.save_config() {
            return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
        }
        return Json(PairResponse {
            state: "trusted".into(),
            device_id: id,
            token: Some(token),
            code: None,
            cursor: Some(latest_cursor(&store)),
        })
        .into_response();
    }
    let existing_pending = {
        let mut pending = state.pending.write().await;
        pending.retain(|_, item| item.expires_ms > now_ms());
        pending
            .values()
            .find(|item| {
                (!client_id.is_empty() && item.client_id == client_id)
                    || (client_id.is_empty()
                        && item.name == name
                        && item.ip == addr.ip().to_string())
            })
            .cloned()
    };
    if let Some(existing) = existing_pending {
        info!(device = %name, client_id = %client_id, "复用待批准配对请求");
        return Json(PairResponse {
            state: "pending".into(),
            device_id: existing.id,
            token: Some(existing.request_token),
            code: Some(existing.code),
            cursor: Some(latest_cursor(&*state.store.lock().await)),
        })
        .into_response();
    }
    let code = format!("{:06}", rand::random::<u32>() % 1_000_000);
    let pending = PendingPair {
        id: id.clone(),
        client_id,
        name: name.clone(),
        code: code.clone(),
        request_token: token.clone(),
        ip: addr.ip().to_string(),
        expires_ms: now_ms() + 300_000,
    };
    state.pending.write().await.insert(id.clone(), pending);
    info!(device = %name, ip = %addr.ip(), "收到待批准的设备配对请求");
    Json(PairResponse {
        state: "pending".into(),
        device_id: id,
        token: Some(token),
        code: Some(code),
        cursor: Some(latest_cursor(&*state.store.lock().await)),
    })
    .into_response()
}

async fn poll_events(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TokenQuery>,
    headers: HeaderMap,
) -> Response {
    let token = q
        .token
        .as_deref()
        .or_else(|| bearer(&headers))
        .unwrap_or("");
    let mut store = state.store.lock().await;
    if !store.is_token_trusted(token) {
        warn!(token_suffix = %token.chars().rev().take(6).collect::<String>(), "手机事件轮询鉴权失败");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(device) = store.config.devices.iter_mut().find(|d| d.token == token) {
        device.last_seen_ms = now_ms();
    }
    let cursor = Some(latest_cursor(&store));
    let events = match q.after_id.as_deref().filter(|value| !value.is_empty()) {
        None => vec![],
        Some("__start__") => store.events.iter().cloned().collect(),
        Some(after) => store
            .events
            .iter()
            .position(|event| event.id == after)
            .map(|index| store.events.iter().skip(index + 1).cloned().collect())
            .unwrap_or_default(),
    };
    if !events.is_empty() {
        info!(count = events.len(), after = ?q.after_id, cursor = ?cursor, "手机轮询取得新事件");
    }
    Json(PollResponse { events, cursor }).into_response()
}

async fn diagnostics(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    if !addr.ip().is_loopback() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let dir = state.store.lock().await.dir.clone();
    let path = dir.join("agentbell.log");
    let log_tail = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .rev()
        .take(250)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    Json(DiagnosticsResponse {
        log_path: path.display().to_string(),
        log_tail,
        tray_status: crate::tray::status(),
    })
    .into_response()
}

async fn approve(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<ApproveRequest>,
) -> Response {
    if !addr.ip().is_loopback() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(pending) = state.pending.write().await.remove(&body.device_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !body.allow {
        return StatusCode::NO_CONTENT.into_response();
    }
    let mut store = state.store.lock().await;
    store.config.devices.retain(|device| {
        if !pending.client_id.is_empty() {
            device.client_id != pending.client_id
        } else {
            device.name != pending.name || device.last_ip != pending.ip
        }
    });
    store.config.devices.push(Device {
        id: pending.id,
        client_id: pending.client_id,
        name: pending.name,
        token: pending.request_token,
        trusted: true,
        created_ms: now_ms(),
        last_seen_ms: now_ms(),
        last_ip: pending.ip,
    });
    match store.save_config() {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn revoke(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<ApproveRequest>,
) -> Response {
    if !addr.ip().is_loopback() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut store = state.store.lock().await;
    store.config.devices.retain(|d| d.id != body.device_id);
    match store.save_config() {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn delete_events(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<DeleteEventsRequest>,
) -> Response {
    if !addr.ip().is_loopback() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let ids = body
        .ids
        .into_iter()
        .take(100)
        .map(|id| clean(&id, 160))
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return (StatusCode::BAD_REQUEST, "请选择要删除的任务记录").into_response();
    }
    let mut store = state.store.lock().await;
    match store.delete_events(&ids) {
        Ok(removed) => {
            info!(removed, "已删除任务历史");
            Json(serde_json::json!({ "removed": removed })).into_response()
        }
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn emit_event(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(event): Json<AgentEvent>,
) -> Response {
    let allowed = if addr.ip().is_loopback() {
        true
    } else {
        let store = state.store.lock().await;
        bearer(&headers) == Some(store.config.emitter_token.as_str())
    };
    if !allowed {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    publish(&state, event.normalize()).await
}

async fn test_event(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    if !addr.ip().is_loopback() {
        return StatusCode::FORBIDDEN.into_response();
    }
    publish(
        &state,
        AgentEvent {
            id: new_id(),
            agent: "AgentBell".into(),
            kind: crate::model::EventKind::Completed,
            conversation_id: "test".into(),
            title: "测试通知".into(),
            project: "AgentBell".into(),
            message: "电脑到手机的通知链路正常".into(),
            duration_ms: Some(2300),
            timestamp_ms: now_ms(),
        },
    )
    .await
}

async fn install_adapter(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<InstallRequest>,
) -> Response {
    if !addr.ip().is_loopback() {
        return StatusCode::FORBIDDEN.into_response();
    }
    if body.agent != "codex" {
        return (StatusCode::BAD_REQUEST, "该适配器无需安装").into_response();
    }
    match std::env::current_exe()
        .and_then(|exe| crate::adapters::install_codex(&exe).map_err(std::io::Error::other))
    {
        Ok(path) => Json(serde_json::json!({ "ok": true, "path": path })).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn publish(state: &Arc<AppState>, event: AgentEvent) -> Response {
    {
        let mut store = state.store.lock().await;
        if let Err(err) = store.add_event(event.clone()) {
            return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
        }
        if !store.config.ntfy_topic.is_empty() {
            let topic = store.config.ntfy_topic.clone();
            let event_clone = event.clone();
            tokio::spawn(async move {
                let url = format!("https://ntfy.sh/{}", topic);
                let body = format!("{}\n{}", event_clone.title, event_clone.message);
                if let Err(err) = reqwest::Client::new()
                    .post(url)
                    .header("Title", format!("{} · AgentBell", event_clone.agent))
                    .body(body)
                    .send()
                    .await
                {
                    warn!(%err, "ntfy 推送失败");
                }
            });
        }
    }
    let receivers = state.tx.send(event.clone()).unwrap_or(0);
    info!(agent = %event.agent, event_id = %event.id, receivers, "事件已保存并广播");
    StatusCode::ACCEPTED.into_response()
}

async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<TokenQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let token = q.token.unwrap_or_default();
    let allowed = addr.ip().is_loopback() || state.store.lock().await.is_token_trusted(&token);
    if !allowed {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let admin = addr.ip().is_loopback();
    ws.on_upgrade(move |socket| ws_session(socket, state, token, admin))
        .into_response()
}

async fn ws_session(mut socket: WebSocket, state: Arc<AppState>, token: String, admin: bool) {
    let mut rx = state.tx.subscribe();
    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Ok(event) => {
                    if !admin && !state.store.lock().await.is_token_trusted(&token) { break; }
                    if socket.send(Message::Text(serde_json::to_string(&event).unwrap_or_default().into())).await.is_err() { break; }
                },
                Err(_) => break,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Ping(v))) => { let _ = socket.send(Message::Pong(v)).await; },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            }
        }
    }
}
