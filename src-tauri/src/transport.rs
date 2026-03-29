use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::time::Duration;
use bytes::Bytes;
use iroh::{Endpoint, EndpointAddr, endpoint::RelayMode};
use iroh::endpoint::Connection;
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{broadcast, Mutex};
use tokio::time::{timeout, timeout_at, Instant};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

/// ALPN protocol identifier for Puck Duel
pub const ALPN: &[u8] = b"puck-duel/0";

/// mDNS service type for LAN discovery
const MDNS_SERVICE_TYPE: &str = "_puckduel._udp.local.";

/// Room records older than this are treated as stale.
const ROOM_TTL_SECS: i64 = 5 * 60;

/// Join attempts poll the signaling backend for this long before timing out.
const JOIN_TIMEOUT_SECS: u64 = 30;

/// Poll interval for room discovery.
const JOIN_POLL_INTERVAL_SECS: u64 = 1;

/// Managed state for the iroh transport layer.
pub struct TransportState {
    /// Lazily-initialized iroh Endpoint (created on first use; replaced on reset)
    endpoint: Mutex<Option<Endpoint>>,
    /// The active peer connection, shared with the game loop
    pub connection: Arc<Mutex<Option<Connection>>>,
    /// Broadcast channel — received datagrams are decoded to UTF-8 and sent here
    pub msg_tx: broadcast::Sender<String>,
    /// Background task handle (accept loop / join task) — abort to cancel
    pub bg_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// LAN addr server task — serves node_addr JSON over TCP on port 9876
    addr_server_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// PocketBase room record id for the current hosted online room.
    hosted_room_id: Arc<Mutex<Option<String>>>,
}

impl TransportState {
    pub fn new() -> Self {
        let (msg_tx, _) = broadcast::channel(64);
        Self {
            endpoint: Mutex::new(None),
            connection: Arc::new(Mutex::new(None)),
            msg_tx,
            bg_task: Arc::new(Mutex::new(None)),
            addr_server_task: Arc::new(Mutex::new(None)),
            hosted_room_id: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the endpoint, initializing it lazily on first call.
    /// Returns a clone (Endpoint is Arc-backed, so this is cheap).
    pub async fn endpoint(&self) -> Result<Endpoint, String> {
        let mut guard = self.endpoint.lock().await;
        if guard.is_none() {
            let ep = Endpoint::builder()
                .relay_mode(RelayMode::Disabled)
                .alpns(vec![ALPN.to_vec()])
                .bind()
                .await
                .map_err(|e| e.to_string())?;
            *guard = Some(ep);
        }
        Ok(guard.as_ref().unwrap().clone())
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Serialize an EndpointAddr to a JSON string.
fn addr_to_json(addr: &EndpointAddr) -> Result<String, String> {
    serde_json::to_string(addr).map_err(|e| e.to_string())
}

/// Deserialize an EndpointAddr from a JSON string.
fn addr_from_json(s: &str) -> Result<EndpointAddr, String> {
    serde_json::from_str(s).map_err(|e| e.to_string())
}

/// Spawn a task that reads datagrams from `conn` and broadcasts them on `msg_tx`.
/// Emits `peer-disconnected` and clears the connection when the loop exits.
fn spawn_recv_loop(
    conn: Connection,
    msg_tx: broadcast::Sender<String>,
    connection: Arc<Mutex<Option<Connection>>>,
    app: AppHandle,
) {
    tokio::spawn(async move {
        loop {
            match conn.read_datagram().await {
                Ok(data) => {
                    if let Ok(msg) = String::from_utf8(data.to_vec()) {
                        let _ = msg_tx.send(msg);
                    }
                }
                Err(_) => break, // connection closed
            }
        }
        *connection.lock().await = None;
        let _ = app.emit("peer-disconnected", ());
    });
}

/// Store the connection, spawn its recv loop, and emit `peer-connected`.
async fn activate_connection(
    state: &TransportState,
    conn: Connection,
    app: &AppHandle,
) {
    *state.connection.lock().await = Some(conn.clone());
    spawn_recv_loop(conn, state.msg_tx.clone(), state.connection.clone(), app.clone());
    let _ = app.emit("peer-connected", ());
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn get_local_ip() -> Result<String, String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    socket.connect("8.8.8.8:80").map_err(|e| e.to_string())?;
    Ok(socket.local_addr().map_err(|e| e.to_string())?.ip().to_string())
}

fn unix_now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn pocketbase_url() -> Result<String, String> {
    let raw = std::env::var("POCKETBASE_URL")
        .ok()
        .or_else(|| option_env!("POCKETBASE_URL").map(str::to_string))
        .ok_or_else(|| "Set POCKETBASE_URL in src-tauri/.env".to_string())?;
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        return Err("POCKETBASE_URL cannot be empty".to_string());
    }
    Ok(trimmed)
}

fn pocketbase_api_base() -> Result<String, String> {
    let base = pocketbase_url()?;
    if base.ends_with("/api") {
        Ok(base)
    } else {
        Ok(format!("{base}/api"))
    }
}

fn pocketbase_token() -> Option<String> {
    std::env::var("POCKETBASE_TOKEN")
        .ok()
        .or_else(|| option_env!("POCKETBASE_TOKEN").map(str::to_string))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn with_pocketbase_auth(
    request: reqwest::RequestBuilder,
    token: Option<&str>,
) -> reqwest::RequestBuilder {
    if let Some(t) = token {
        request.bearer_auth(t)
    } else {
        request
    }
}

#[derive(Deserialize)]
struct PbList<T> {
    items: Vec<T>,
}

#[derive(Deserialize)]
struct PbRoomCreateResp {
    id: String,
}

#[derive(Deserialize)]
struct PbRoomRow {
    id: String,
    node_addr: String,
}

#[derive(Deserialize)]
struct PbRoomIdRow {
    id: String,
}

async fn cleanup_expired_rooms(
    client: &reqwest::Client,
    api_base: &str,
    token: Option<&str>,
) -> Result<(), String> {
    let mut url = reqwest::Url::parse(&format!("{api_base}/collections/rooms/records"))
        .map_err(|e| format!("Invalid PocketBase URL: {e}"))?;
    let now = unix_now_secs();
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("filter", &format!("expires_at <= {now}"));
        qp.append_pair("fields", "id");
        qp.append_pair("perPage", "200");
    }

    let req = with_pocketbase_auth(client.get(url), token);
    let resp = req
        .send()
        .await
        .map_err(|e| format!("PocketBase cleanup list failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("PocketBase cleanup list error: {e}"))?;

    let rows = resp
        .json::<PbList<PbRoomIdRow>>()
        .await
        .map_err(|e| format!("PocketBase cleanup parse failed: {e}"))?;

    for row in rows.items {
        let req = with_pocketbase_auth(
            client.delete(format!("{api_base}/collections/rooms/records/{}", row.id)),
            token,
        );
        if let Err(e) = req.send().await {
            eprintln!("[transport] PocketBase cleanup delete failed: {e}");
        }
    }

    Ok(())
}

async fn delete_room_record(
    client: &reqwest::Client,
    api_base: &str,
    token: Option<&str>,
    room_id: &str,
) -> Result<(), String> {
    with_pocketbase_auth(
        client.delete(format!("{api_base}/collections/rooms/records/{room_id}")),
        token,
    )
    .send()
    .await
    .map_err(|e| format!("PocketBase delete failed: {e}"))?
    .error_for_status()
    .map_err(|e| format!("PocketBase delete error: {e}"))?;
    Ok(())
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

/// Return our current EndpointAddr as a JSON string.
#[tauri::command]
pub async fn get_our_node_addr(transport: State<'_, TransportState>) -> Result<String, String> {
    let ep = transport.endpoint().await?;
    addr_to_json(&ep.addr())
}

/// Start a tiny TCP server on port 9876 that serves our node_addr JSON to LAN peers.
/// Returns our local IP address to use as QR code content.
#[tauri::command]
pub async fn start_addr_server(transport: State<'_, TransportState>) -> Result<String, String> {
    use tokio::net::TcpListener;
    use tokio::io::AsyncWriteExt;

    let ep = transport.endpoint().await?;
    let node_addr_json = addr_to_json(&ep.addr())?;
    let local_ip = get_local_ip()?;

    if let Some(old) = transport.addr_server_task.lock().await.take() { old.abort(); }

    let handle = tokio::spawn(async move {
        let Ok(listener) = TcpListener::bind("0.0.0.0:9876").await else { return };
        loop {
            if let Ok((mut socket, _)) = listener.accept().await {
                let json = node_addr_json.clone();
                tokio::spawn(async move { let _ = socket.write_all(json.as_bytes()).await; });
            }
        }
    });
    *transport.addr_server_task.lock().await = Some(handle);

    Ok(local_ip)
}

/// Fetch a LAN host's node_addr JSON by connecting to their addr server.
#[tauri::command]
pub async fn fetch_peer_addr(peer_ip: String) -> Result<String, String> {
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpStream;

    let addr = format!("{}:9876", peer_ip.trim());
    let mut stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&addr))
        .await
        .map_err(|_| "Could not reach host".to_string())?
        .map_err(|e| e.to_string())?;

    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf))
        .await
        .map_err(|_| "Host did not respond".to_string())?
        .map_err(|e| e.to_string())?;

    String::from_utf8(buf).map_err(|e| e.to_string())
}

/// Start LAN peer discovery via mDNS-SD.
///
/// This command:
/// 1. Registers our own service so others can find us
/// 2. Browses for other peers and emits `peer-discovered` events with their node_addr_json
#[tauri::command]
pub async fn discover_lan(
    transport: State<'_, TransportState>,
    app: AppHandle,
) -> Result<(), String> {
    use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

    let ep = transport.endpoint().await?;
    let our_addr = ep.addr();
    let our_id_hex = hex::encode(our_addr.id.as_bytes());
    let node_addr_json = addr_to_json(&our_addr)?;
    let addr_b64 = B64.encode(&node_addr_json);

    // Our node id (first 8 hex chars) as the service instance name
    let service_name = format!("pd-{}", &our_id_hex[..8]);

    // Port: take first bound socket port (required by mDNS-SD, but iroh uses its own QUIC port)
    let bound = ep.bound_sockets();
    let port = if let Some(sa) = bound.first() { sa.port() } else { 11000 };

    tokio::task::spawn_blocking(move || {
        let mdns = match ServiceDaemon::new() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[transport] mDNS daemon error: {e}");
                return;
            }
        };

        // Register ourselves
        let mut props = std::collections::HashMap::new();
        props.insert("addr".to_string(), addr_b64);

        let my_addr_str = "";  // empty = auto-detect IP
        let hostname = format!("{}.local.", service_name);
        let svc = match ServiceInfo::new(
            MDNS_SERVICE_TYPE,
            &service_name,
            &hostname,
            my_addr_str,
            port,
            Some(props),
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[transport] mDNS ServiceInfo error: {e}");
                return;
            }
        };

        if let Err(e) = mdns.register(svc) {
            eprintln!("[transport] mDNS register error: {e}");
        }

        // Browse for peers
        let receiver = match mdns.browse(MDNS_SERVICE_TYPE) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[transport] mDNS browse error: {e}");
                return;
            }
        };

        let my_service_name = service_name.clone();

        // Read events for 30 seconds
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let timeout = deadline.saturating_duration_since(std::time::Instant::now());
            if timeout.is_zero() { break; }
            match receiver.recv_timeout(timeout) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    let name = info.get_fullname();
                    // Skip our own service
                    if name.starts_with(&my_service_name) { continue; }

                    if let Some(addr_b64) = info.get_properties().get_property_val_str("addr") {
                        if let Ok(json_bytes) = B64.decode(addr_b64.as_bytes()) {
                            if let Ok(json) = String::from_utf8(json_bytes) {
                                let _ = app.emit("peer-discovered", json);
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    Ok(())
}

// ─── Bidirectional-connect helpers ────────────────────────────────────────────

/// Fetch the joiner's EndpointAddr JSON from the room record (returns None if not set yet).
async fn fetch_joiner_addr(
    client: &reqwest::Client,
    api_base: &str,
    token: Option<&str>,
    room_id: &str,
) -> Option<String> {
    #[derive(Deserialize)]
    struct Resp {
        #[serde(default)]
        joiner_addr: Option<String>,
    }
    let url = format!("{}/collections/rooms/records/{}?fields=joiner_addr", api_base, room_id);
    let resp = with_pocketbase_auth(client.get(&url), token)
        .send().await.ok()?
        .error_for_status().ok()?
        .json::<Resp>().await.ok()?;
    resp.joiner_addr.filter(|s| !s.is_empty())
}

/// Store the room ID so the host's accept loop knows which room to poll for joiner_addr.
#[tauri::command]
pub async fn set_hosted_room_id(
    transport: State<'_, TransportState>,
    room_id: String,
) -> Result<(), String> {
    *transport.hosted_room_id.lock().await = Some(room_id);
    Ok(())
}

/// Start listening for an incoming peer connection (LAN host mode).
///
/// Spawns a background task that accepts exactly one peer and emits `peer-connected`.
#[tauri::command]
pub async fn start_accept_loop(
    transport: State<'_, TransportState>,
    app: AppHandle,
) -> Result<(), String> {
    let ep = transport.endpoint().await?.clone();
    let transport_conn = transport.connection.clone();
    let msg_tx = transport.msg_tx.clone();
    let hosted_room_id = transport.hosted_room_id.clone();

    // Clear any stale room id from a previous session.
    *hosted_room_id.lock().await = None;

    // Abort any previous background task
    if let Some(old) = transport.bg_task.lock().await.take() { old.abort(); }

    let api_base = pocketbase_api_base().ok();
    let token = pocketbase_token();

    let handle = tokio::spawn(async move {
        let deadline = Instant::now() + Duration::from_secs(300);

        // Path 1: wait for the joiner to connect to us (classic accept).
        let accept_fut = {
            let ep = ep.clone();
            async move {
                loop {
                    match timeout_at(deadline, ep.accept()).await {
                        Ok(Some(incoming)) => match incoming.accept() {
                            Ok(accepting) => match accepting.await {
                                Ok(conn) => return Some(conn),
                                Err(e) => eprintln!("[transport] accept handshake: {e}"),
                            },
                            Err(e) => eprintln!("[transport] accept: {e}"),
                        },
                        _ => return None,
                    }
                }
            }
        };

        // Path 2: poll PocketBase for joiner's addr, then connect to them.
        // This lets us reach joiners that have a public IPv6 but whose host (us) only
        // has a private IPv4 — the roles are effectively reversed for the TCP handshake.
        let connect_fut = {
            let ep = ep.clone();
            async move {
                let api_base = match api_base {
                    Some(s) => s,
                    None => return None,
                };
                let client = reqwest::Client::new();
                let mut cached_joiner_addr: Option<EndpointAddr> = None;
                loop {
                    if Instant::now() >= deadline { return None; }

                    // Once we have a joiner addr, keep retrying connect rather than re-polling.
                    if cached_joiner_addr.is_none() {
                        let room_id = hosted_room_id.lock().await.clone();
                        if let Some(ref rid) = room_id {
                            if let Some(joiner_json) = fetch_joiner_addr(&client, &api_base, token.as_deref(), rid).await {
                                if let Ok(joiner_addr) = addr_from_json(&joiner_json) {
                                    cached_joiner_addr = Some(joiner_addr);
                                }
                            }
                        }
                    }

                    if let Some(ref joiner_addr) = cached_joiner_addr {
                        match timeout(Duration::from_secs(8), ep.connect(joiner_addr.clone(), ALPN)).await {
                            Ok(Ok(conn)) => return Some(conn),
                            Ok(Err(e)) => eprintln!("[transport] connect-to-joiner failed: {e}"),
                            Err(_) => eprintln!("[transport] connect-to-joiner timed out"),
                        }
                        // Retry after a short delay instead of giving up.
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    } else {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        };

        // Race accept vs connect-to-joiner; if one finishes with None, wait for the other.
        let mut accept_fut = Box::pin(accept_fut);
        let mut connect_fut = Box::pin(connect_fut);
        let conn = loop {
            tokio::select! {
                conn = &mut accept_fut => {
                    if let Some(c) = conn { break Some(c); }
                    break connect_fut.await;
                }
                conn = &mut connect_fut => {
                    if let Some(c) = conn { break Some(c); }
                    break accept_fut.await;
                }
            }
        };

        if let Some(conn) = conn {
            *transport_conn.lock().await = Some(conn.clone());
            spawn_recv_loop(conn, msg_tx, transport_conn, app.clone());
            let _ = app.emit("peer-connected", ());
        }
    });
    *transport.bg_task.lock().await = Some(handle);

    Ok(())
}

/// Connect outbound to a peer given their EndpointAddr JSON (first attempt, joiner → host).
///
/// Only tries direct outbound connections — no accept path.  If this fails, the caller
/// should post their own address to the room and call `accept_from_peer` so the host
/// can connect in the other direction (second attempt, host → joiner).
#[tauri::command]
pub async fn connect_to_peer(
    transport: State<'_, TransportState>,
    app: AppHandle,
    node_addr_json: String,
) -> Result<(), String> {
    let addr: EndpointAddr = addr_from_json(&node_addr_json)?;
    let ep = transport.endpoint().await?;
    let deadline = Instant::now() + Duration::from_secs(15);

    for attempt in 0..3u32 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        if Instant::now() >= deadline { break; }
        match timeout(Duration::from_secs(4), ep.connect(addr.clone(), ALPN)).await {
            Ok(Ok(conn)) => {
                activate_connection(&transport, conn, &app).await;
                return Ok(());
            }
            Ok(Err(e)) => eprintln!("[transport] connect attempt {attempt}: {e}"),
            Err(_)     => eprintln!("[transport] connect attempt {attempt} timed out"),
        }
    }
    Err("direct connection failed".into())
}

/// Wait for the host to connect to us (second attempt, host → joiner).
///
/// Called after the joiner has posted their address to the room.  The host's accept
/// loop will discover that address and connect outbound; we simply accept here.
#[tauri::command]
pub async fn accept_from_peer(
    transport: State<'_, TransportState>,
    app: AppHandle,
) -> Result<(), String> {
    let ep = transport.endpoint().await?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match timeout_at(deadline, ep.accept()).await {
            Ok(Some(incoming)) => match incoming.accept() {
                Ok(accepting) => match accepting.await {
                    Ok(conn) => {
                        activate_connection(&transport, conn, &app).await;
                        return Ok(());
                    }
                    Err(e) => eprintln!("[transport] accept_from_peer handshake: {e}"),
                },
                Err(e) => eprintln!("[transport] accept_from_peer: {e}"),
            },
            _ => return Err("timed out waiting for host to connect back".into()),
        }
    }
}

/// Reset the transport layer: abort tasks, drop the connection and endpoint.
/// Must be called after a network change before re-hosting or re-joining.
#[tauri::command]
pub async fn reset_transport(transport: State<'_, TransportState>) -> Result<(), String> {
    if let Some(old) = transport.bg_task.lock().await.take() { old.abort(); }
    if let Some(old) = transport.addr_server_task.lock().await.take() { old.abort(); }
    *transport.connection.lock().await = None;
    *transport.hosted_room_id.lock().await = None;
    // Drop the old endpoint so the next call to endpoint() creates a fresh one.
    let _old_ep = transport.endpoint.lock().await.take();
    // _old_ep is dropped here, which closes the QUIC endpoint.
    Ok(())
}

/// Host an online game: post our EndpointAddr to PocketBase and wait for a peer.
///
/// Returns the 4-digit room code to display to the user.
/// Emits `peer-connected` when the first peer joins.
#[tauri::command]
pub async fn host_online(
    transport: State<'_, TransportState>,
    app: AppHandle,
) -> Result<String, String> {
    let ep = transport.endpoint().await?;
    let our_addr = ep.addr();
    let node_addr_json = addr_to_json(&our_addr)?;

    // Generate a random 4-digit numeric room code
    let code: String = format!("{:04}", rand::random::<u32>() % 10000);

    let api_base = pocketbase_api_base()?;
    let token = pocketbase_token();
    let client = reqwest::Client::new();
    let expires_at = unix_now_secs() + ROOM_TTL_SECS;

    // Best-effort cleanup to keep room collection bounded.
    if let Err(e) = cleanup_expired_rooms(&client, &api_base, token.as_deref()).await {
        eprintln!("[transport] cleanup_expired_rooms failed: {e}");
    }

    // Create room record in PocketBase.
    let body = serde_json::json!({
        "code": code,
        "node_addr": node_addr_json,
        "expires_at": expires_at,
    });
    let create_req = with_pocketbase_auth(
        client.post(format!("{api_base}/collections/rooms/records")),
        token.as_deref(),
    )
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("PocketBase create failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("PocketBase create error: {e}"))?;
    let create_resp = create_req
        .json::<PbRoomCreateResp>()
        .await
        .map_err(|e| format!("PocketBase create parse failed: {e}"))?;
    *transport.hosted_room_id.lock().await = Some(create_resp.id);

    // Spawn a background task to accept the incoming peer connection
    let ep = ep.clone();
    let transport_conn = transport.connection.clone();
    let hosted_room_id = transport.hosted_room_id.clone();
    let msg_tx = transport.msg_tx.clone();
    let api_base_for_task = api_base.clone();
    let token_for_task = token.clone();

    // Abort any previous background task
    if let Some(old) = transport.bg_task.lock().await.take() { old.abort(); }

    let handle = tokio::spawn(async move {
        // Wait up to 5 minutes for a peer to connect
        let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
        loop {
            let accept_fut = ep.accept();
            match tokio::time::timeout_at(deadline, accept_fut).await {
                Ok(Some(incoming)) => {
                    match incoming.accept() {
                        Ok(accepting) => {
                            match accepting.await {
                                Ok(conn) => {
                                    *transport_conn.lock().await = Some(conn.clone());
                                    spawn_recv_loop(conn, msg_tx.clone(), transport_conn.clone(), app.clone());
                                    if let Some(room_id) = hosted_room_id.lock().await.take() {
                                        let client = reqwest::Client::new();
                                        if let Err(e) = delete_room_record(
                                            &client,
                                            &api_base_for_task,
                                            token_for_task.as_deref(),
                                            &room_id,
                                        ).await {
                                            eprintln!("[transport] host room delete failed: {e}");
                                        }
                                    }
                                    let _ = app.emit("peer-connected", ());
                                    break; // Accept only one peer
                                }
                                Err(e) => eprintln!("[transport] handshake error: {e}"),
                            }
                        }
                        Err(e) => eprintln!("[transport] accept error: {e}"),
                    }
                }
                Ok(None) => break, // Endpoint closed
                Err(_) => break,   // Timed out
            }
        }
    });
    *transport.bg_task.lock().await = Some(handle);

    Ok(code)
}

/// Join an online game by entering the host's room code.
///
/// Spawns a background task that polls PocketBase then connects via QUIC.
/// Emits `peer-connected` on success or `join-error` on failure.
#[tauri::command]
pub async fn join_online(
    transport: State<'_, TransportState>,
    app: AppHandle,
    room_code: String,
) -> Result<(), String> {
    let code = room_code.trim().to_uppercase();
    let ep = transport.endpoint().await?.clone();
    let transport_conn = transport.connection.clone();
    let msg_tx = transport.msg_tx.clone();
    let api_base = pocketbase_api_base()?;
    let token = pocketbase_token();

    // Abort any previous background task
    if let Some(old) = transport.bg_task.lock().await.take() { old.abort(); }

    let handle = tokio::spawn(async move {
        let client = reqwest::Client::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(JOIN_TIMEOUT_SECS);
        let mut room: Option<PbRoomRow> = None;
        loop {
            if tokio::time::Instant::now() >= deadline { break; }
            let now = unix_now_secs();
            let filter = format!("code = \"{}\" && expires_at > {}", code, now);
            let mut url = match reqwest::Url::parse(&format!("{api_base}/collections/rooms/records")) {
                Ok(u) => u,
                Err(e) => {
                    let _ = app.emit("join-error", format!("Invalid PocketBase URL: {e}"));
                    return;
                }
            };
            {
                let mut qp = url.query_pairs_mut();
                qp.append_pair("filter", &filter);
                qp.append_pair("perPage", "1");
                qp.append_pair("sort", "-expires_at");
                qp.append_pair("fields", "id,node_addr");
            }

            let req = with_pocketbase_auth(client.get(url), token.as_deref());
            match req.send().await {
                Ok(resp) => {
                    if let Ok(resp) = resp.error_for_status() {
                        if let Ok(rows) = resp.json::<PbList<PbRoomRow>>().await {
                            if let Some(found) = rows.items.into_iter().next() {
                                room = Some(found);
                                break;
                            }
                        }
                    }
                }
                Err(_) => {}
            }
            tokio::time::sleep(Duration::from_secs(JOIN_POLL_INTERVAL_SECS)).await;
        }

        let found_room = match room {
            Some(r) => r,
            None => {
                let _ = app.emit("join-error", format!("Room '{}' not found", code));
                return;
            }
        };

        let addr = match addr_from_json(&found_room.node_addr) {
            Ok(a) => a,
            Err(e) => { let _ = app.emit("join-error", e); return; }
        };

        match ep.connect(addr, ALPN).await {
            Ok(conn) => {
                *transport_conn.lock().await = Some(conn.clone());
                spawn_recv_loop(conn, msg_tx, transport_conn.clone(), app.clone());
                if let Err(e) = delete_room_record(
                    &client,
                    &api_base,
                    token.as_deref(),
                    &found_room.id,
                ).await {
                    eprintln!("[transport] join room delete failed: {e}");
                }
                let _ = app.emit("peer-connected", ());
            }
            Err(e) => { let _ = app.emit("join-error", e.to_string()); }
        }
    });
    *transport.bg_task.lock().await = Some(handle);
    Ok(())
}

/// Abort any pending background connection task (join or accept loop).
#[tauri::command]
pub async fn cancel_online(transport: State<'_, TransportState>) -> Result<(), String> {
    if let Some(handle) = transport.bg_task.lock().await.take() { handle.abort(); }
    if let Some(handle) = transport.addr_server_task.lock().await.take() { handle.abort(); }

    if let Ok(api_base) = pocketbase_api_base() {
        let token = pocketbase_token();
        let client = reqwest::Client::new();

        if let Some(room_id) = transport.hosted_room_id.lock().await.take() {
            if let Err(e) = delete_room_record(&client, &api_base, token.as_deref(), &room_id).await {
                eprintln!("[transport] cancel room delete failed: {e}");
            }
        }

        // Best-effort sweep to remove any already-expired rooms.
        if let Err(e) = cleanup_expired_rooms(&client, &api_base, token.as_deref()).await {
            eprintln!("[transport] cancel cleanup sweep failed: {e}");
        }
    }

    Ok(())
}

/// Send a raw string datagram on the active connection.
///
/// Called by the game loop — not a Tauri command.
/// Returns `true` if the send succeeded.
pub async fn send_msg(transport: &TransportState, msg: String) -> bool {
    let guard = transport.connection.lock().await;
    if let Some(conn) = guard.as_ref() {
        conn.send_datagram(Bytes::from(msg.into_bytes())).is_ok()
    } else {
        false
    }
}
