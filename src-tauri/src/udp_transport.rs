use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::Duration;
use tauri::{AppHandle, State, Emitter};

/// Simple state for the raw-UDP transport used in LAN games.
///
/// We keep an optional socket and the most-recently seen peer address.  The
/// receive loop updates the peer address on every incoming packet and emits a
/// `udp-msg-received` event back to the frontend.
pub struct UdpState {
    /// optional socket wrapped in an Arc so it can be cloned cheaply
    pub socket: Arc<Mutex<Option<Arc<UdpSocket>>>>,
    pub peer: Arc<Mutex<Option<SocketAddr>>>,
    /// broadcast channel for received messages, same semantics as TransportState
    pub msg_tx: tokio::sync::broadcast::Sender<String>,
    /// background task handle for discovery; abort to restart
    pub discovery_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// background task handle for the main recv loop; abort before rebinding
    pub recv_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl UdpState {
    pub fn new() -> Self {
        let (msg_tx, _) = tokio::sync::broadcast::channel(64);
        Self {
            socket: Arc::new(Mutex::new(None)),
            peer: Arc::new(Mutex::new(None)),
            msg_tx,
            discovery_task: Arc::new(Mutex::new(None)),
            recv_task: Arc::new(Mutex::new(None)),
        }
    }
}

/// Spawn a background task that reads datagrams and forwards them to the GUI.
fn spawn_recv_loop(socket: Arc<UdpSocket>, peer: Arc<Mutex<Option<SocketAddr>>>, app: AppHandle, msg_tx: tokio::sync::broadcast::Sender<String>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, addr)) => {
                    // remember the last sender so we can reply later
                    {
                        let mut guard = peer.lock().await;
                        *guard = Some(addr);
                    }
                    if let Ok(msg) = String::from_utf8(buf[..len].to_vec()) {
                        let _ = app.emit("udp-msg-received", (addr.to_string(), msg.clone()));
                        let _ = msg_tx.send(msg); // also broadcast into channel
                    }
                }
                Err(e) => {
                    eprintln!("[udp] recv error: {}", e);
                    break;
                }
            }
        }
    })
}

/// Abort the recv loop and drop the existing socket so port 8080 is freed.
async fn close_existing_socket(state: &UdpState) {
    if let Some(task) = state.recv_task.lock().await.take() {
        task.abort();
    }
    *state.socket.lock().await = None;
    *state.peer.lock().await = None;
}

/// Bind a socket on port **8080** and start listening for a single peer.
#[tauri::command]
pub async fn start_udp_host(state: State<'_, UdpState>, app: AppHandle) -> Result<(), String> {
    close_existing_socket(&state).await;
    let sock = UdpSocket::bind("0.0.0.0:8080").await.map_err(|e| e.to_string())?;
    let sock_arc = Arc::new(sock);
    {
        let mut guard = state.socket.lock().await;
        *guard = Some(sock_arc.clone());
    }
    let handle = spawn_recv_loop(sock_arc.clone(), state.peer.clone(), app, state.msg_tx.clone());
    *state.recv_task.lock().await = Some(handle);
    Ok(())
}

/// Create a client socket bound to an ephemeral port and remember the host
/// address (port 8080 is assumed if not supplied).
#[tauri::command]
pub async fn connect_udp_client(
    state: State<'_, UdpState>,
    app: AppHandle,
    host_ip: String,
) -> Result<(), String> {
    close_existing_socket(&state).await;
    let sock = UdpSocket::bind("0.0.0.0:0").await.map_err(|e| e.to_string())?;
    let host_addr: SocketAddr = if host_ip.contains(':') {
        host_ip.parse().map_err(|e: std::net::AddrParseError| e.to_string())?
    } else {
        format!("{}:8080", host_ip.trim()).parse().map_err(|e: std::net::AddrParseError| e.to_string())?
    };
    let sock_arc = Arc::new(sock);
    {
        let mut peer_guard = state.peer.lock().await;
        *peer_guard = Some(host_addr);
        let mut sock_guard = state.socket.lock().await;
        *sock_guard = Some(sock_arc.clone());
    }
    let handle = spawn_recv_loop(sock_arc.clone(), state.peer.clone(), app, state.msg_tx.clone());
    *state.recv_task.lock().await = Some(handle);
    Ok(())
}

/// Send a message to the last-known peer address. Returns `false` if we don't
/// have either a socket or a peer yet, or if the send fails.
#[tauri::command]
pub async fn host_send_msg(state: State<'_, UdpState>, msg: String) -> Result<bool, String> {
    let peer_opt = { state.peer.lock().await.clone() };
    if let Some(peer) = peer_opt {
        if let Some(sock_arc) = &*state.socket.lock().await {
            // sock_arc is Arc<UdpSocket>
            return Ok(sock_arc.send_to(msg.as_bytes(), peer).await.is_ok());
        }
    }
    Ok(false)
}

/// Alias for `host_send_msg` so the frontend doesn't need to care about which
/// side it is running on.
#[tauri::command]
pub async fn client_send_msg(state: State<'_, UdpState>, msg: String) -> Result<bool, String> {
    host_send_msg(state, msg).await
}

/// Internal helper used by the game engine, not exposed as a Tauri command.
///
/// Returns `true` on success (message sent) and `false` otherwise.
pub async fn send_msg_internal(state: &UdpState, msg: String) -> bool {
    let peer_opt = { state.peer.lock().await.clone() };
    if let Some(peer) = peer_opt {
        if let Some(sock) = &*state.socket.lock().await {
            return sock.send_to(msg.as_bytes(), peer).await.is_ok();
        }
    }
    false
}

/// Return a list of local IPv4 addresses we can reach the outside world with.
/// For now this is a single address obtained by connecting to 8.8.8.8; it is
/// sufficient for generating QR codes.
#[tauri::command]
pub async fn get_local_ips() -> Result<Vec<String>, String> {
    let socket = UdpSocket::bind("0.0.0.0:0").await.map_err(|e| e.to_string())?;
    socket
        .connect("8.8.8.8:80")
        .await
        .map_err(|e| e.to_string())?;
    let ip = socket.local_addr().map_err(|e| e.to_string())?.ip().to_string();
    Ok(vec![ip])
}

/// Start simple UDP-based LAN discovery: periodically broadcast our local IP and
/// listen for other peers doing the same. Emits `peer-found` events with the
/// sender address as payload.
#[tauri::command]
pub async fn start_discovery(state: State<'_, UdpState>, app: AppHandle) -> Result<(), String> {
    // abort any existing discovery task
    if let Some(old) = state.discovery_task.lock().await.take() {
        old.abort();
    }

    // bind a socket on an arbitrary port for both send and recv
    let sock = UdpSocket::bind("0.0.0.0:9001").await.map_err(|e| e.to_string())?;
    sock.set_broadcast(true).map_err(|e| e.to_string())?;

    // determine our own IP so we can filter out self-sent packets
    let our_ips = get_local_ips().await?;
    let our_ip = our_ips.get(0).cloned().unwrap_or_default();
    let msg = our_ip.clone();

    let handle = tokio::spawn(async move {
        let mut buf = [0u8; 128];
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let _ = sock.send_to(msg.as_bytes(), "255.255.255.255:9001").await;
                }
                result = sock.recv_from(&mut buf) => {
                    if let Ok((len, src)) = result {
                        let ip_str = src.ip().to_string();
                        if ip_str == our_ip { continue; }
                        if let Ok(s) = String::from_utf8(buf[..len].to_vec()) {
                            let _ = app.emit("peer-found", ip_str);
                        }
                    }
                }
            }
        }
    });

    *state.discovery_task.lock().await = Some(handle);
    Ok(())
}

/// Stop ongoing LAN discovery (if any).
#[tauri::command]
pub async fn stop_discovery(state: State<'_, UdpState>) -> Result<(), String> {
    if let Some(handle) = state.discovery_task.lock().await.take() {
        handle.abort();
    }
    Ok(())
}
