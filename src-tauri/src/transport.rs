use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use iroh::{Endpoint, EndpointAddr, endpoint::RelayMode};
use iroh::endpoint::Connection;
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{broadcast, Mutex, OnceCell};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

/// ALPN protocol identifier for Puck Duel
pub const ALPN: &[u8] = b"puck-duel/0";

/// mDNS service type for LAN discovery
const MDNS_SERVICE_TYPE: &str = "_puckduel._udp.local.";

/// Supabase credentials — set in src-tauri/.env (never committed)
/// See src-tauri/.env.example for the required keys.
const SUPABASE_URL: &str = env!("SUPABASE_URL", "Set SUPABASE_URL in src-tauri/.env");
const SUPABASE_ANON_KEY: &str = env!("SUPABASE_ANON_KEY", "Set SUPABASE_ANON_KEY in src-tauri/.env");

/// Managed state for the iroh transport layer.
pub struct TransportState {
    /// Lazily-initialized iroh Endpoint (created on first use)
    endpoint: OnceCell<Endpoint>,
    /// The active peer connection, shared with the game loop
    pub connection: Arc<Mutex<Option<Connection>>>,
    /// Broadcast channel — received datagrams are decoded to UTF-8 and sent here
    pub msg_tx: broadcast::Sender<String>,
    /// Background task handle (accept loop / join task) — abort to cancel
    pub bg_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// LAN addr server task — serves node_addr JSON over TCP on port 9876
    addr_server_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl TransportState {
    pub fn new() -> Self {
        let (msg_tx, _) = broadcast::channel(64);
        Self {
            endpoint: OnceCell::new(),
            connection: Arc::new(Mutex::new(None)),
            msg_tx,
            bg_task: Arc::new(Mutex::new(None)),
            addr_server_task: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the endpoint, initializing it lazily on first call.
    pub async fn endpoint(&self) -> Result<&Endpoint, String> {
        self.endpoint
            .get_or_try_init(|| async {
                Endpoint::builder()
                    .relay_mode(RelayMode::Default)
                    .alpns(vec![ALPN.to_vec()])
                    .bind()
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
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
fn spawn_recv_loop(conn: Connection, msg_tx: broadcast::Sender<String>) {
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
    });
}

/// Store the connection, spawn its recv loop, and emit `peer-connected`.
async fn activate_connection(
    state: &TransportState,
    conn: Connection,
    app: &AppHandle,
) {
    *state.connection.lock().await = Some(conn.clone());
    spawn_recv_loop(conn, state.msg_tx.clone());
    let _ = app.emit("peer-connected", ());
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn get_local_ip() -> Result<String, String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    socket.connect("8.8.8.8:80").map_err(|e| e.to_string())?;
    Ok(socket.local_addr().map_err(|e| e.to_string())?.ip().to_string())
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

    // Abort any previous background task
    if let Some(old) = transport.bg_task.lock().await.take() { old.abort(); }

    let handle = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
        loop {
            match tokio::time::timeout_at(deadline, ep.accept()).await {
                Ok(Some(incoming)) => {
                    match incoming.accept() {
                        Ok(accepting) => match accepting.await {
                            Ok(conn) => {
                                *transport_conn.lock().await = Some(conn.clone());
                                spawn_recv_loop(conn, msg_tx.clone());
                                let _ = app.emit("peer-connected", ());
                                break;
                            }
                            Err(e) => eprintln!("[transport] LAN handshake error: {e}"),
                        },
                        Err(e) => eprintln!("[transport] LAN accept error: {e}"),
                    }
                }
                Ok(None) | Err(_) => break,
            }
        }
    });
    *transport.bg_task.lock().await = Some(handle);

    Ok(())
}

/// Connect to a peer given their EndpointAddr JSON.
///
/// On success, stores the connection and emits `peer-connected`.
#[tauri::command]
pub async fn connect_to_peer(
    transport: State<'_, TransportState>,
    app: AppHandle,
    node_addr_json: String,
) -> Result<(), String> {
    let addr: EndpointAddr = addr_from_json(&node_addr_json)?;
    let ep = transport.endpoint().await?;
    let conn = ep.connect(addr, ALPN).await.map_err(|e| e.to_string())?;
    activate_connection(&transport, conn, &app).await;
    Ok(())
}

/// Host an online game: post our EndpointAddr to Supabase and wait for a peer.
///
/// Returns the 6-character room code to display to the user.
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

    // Post to Supabase
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "code": code,
        "node_addr": node_addr_json,
    });
    client
        .post(format!("{}/rest/v1/rooms", SUPABASE_URL))
        .header("apikey", SUPABASE_ANON_KEY)
        .header("Authorization", format!("Bearer {}", SUPABASE_ANON_KEY))
        .header("Content-Type", "application/json")
        .header("Prefer", "return=minimal")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Supabase POST failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Supabase POST error: {e}"))?;

    // Spawn a background task to accept the incoming peer connection
    let ep = ep.clone();
    let transport_conn = transport.connection.clone();
    let msg_tx = transport.msg_tx.clone();

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
                                    spawn_recv_loop(conn, msg_tx.clone());
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
/// Spawns a background task that polls Supabase then connects via QUIC.
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

    // Abort any previous background task
    if let Some(old) = transport.bg_task.lock().await.take() { old.abort(); }

    let handle = tokio::spawn(async move {
        let client = reqwest::Client::new();
        let url = format!("{}/rest/v1/rooms?code=eq.{}&select=node_addr", SUPABASE_URL, code);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut node_addr_json: Option<String> = None;
        loop {
            if tokio::time::Instant::now() >= deadline { break; }
            match client
                .get(&url)
                .header("apikey", SUPABASE_ANON_KEY)
                .header("Authorization", format!("Bearer {}", SUPABASE_ANON_KEY))
                .send()
                .await
            {
                Ok(resp) => {
                    #[derive(Deserialize)]
                    struct Row { node_addr: String }
                    if let Ok(rows) = resp.json::<Vec<Row>>().await {
                        if let Some(row) = rows.into_iter().next() {
                            node_addr_json = Some(row.node_addr);
                            break;
                        }
                    }
                }
                Err(_) => {}
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        let json = match node_addr_json {
            Some(j) => j,
            None => {
                let _ = app.emit("join-error", format!("Room '{}' not found", code));
                return;
            }
        };

        let addr = match addr_from_json(&json) {
            Ok(a) => a,
            Err(e) => { let _ = app.emit("join-error", e); return; }
        };

        match ep.connect(addr, ALPN).await {
            Ok(conn) => {
                *transport_conn.lock().await = Some(conn.clone());
                spawn_recv_loop(conn, msg_tx);
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
