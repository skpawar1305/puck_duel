use std::sync::Arc;
use matchbox_socket::{WebRtcSocket, WebRtcSocketBuilder, PeerId, Packet};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{broadcast, Mutex};
use tokio::time::{timeout, Duration};
use rand::Rng;

/// Managed state for the WebRTC transport layer.
pub struct WebRtcTransportState {
    /// WebRTC socket (optional, created when hosting or joining)
    socket: Arc<Mutex<Option<WebRtcSocket>>>,
    /// Background task handle for message loop
    bg_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Broadcast channel for received messages
    pub msg_tx: broadcast::Sender<String>,
    /// Room ID for current hosted/joined room
    room_id: Arc<Mutex<Option<String>>>,
    /// Peer ID of the connected peer (if any)
    peer_id: Arc<Mutex<Option<PeerId>>>,
}

impl WebRtcTransportState {
    pub fn new() -> Self {
        let (msg_tx, _) = broadcast::channel(64);
        Self {
            socket: Arc::new(Mutex::new(None)),
            bg_task: Arc::new(Mutex::new(None)),
            msg_tx,
            room_id: Arc::new(Mutex::new(None)),
            peer_id: Arc::new(Mutex::new(None)),
        }
    }

    /// Get signaling server URL from environment variable
    fn signaling_server_url() -> Result<String, String> {
        std::env::var("MATCHBOX_SIGNALING_URL")
            .map_err(|_| "MATCHBOX_SIGNALING_URL environment variable not set".to_string())
    }

    /// Generate a random 4-digit room code
    fn generate_room_code() -> String {
        format!("{:04}", rand::thread_rng().gen_range(0..10000))
    }

    /// Build room URL from signaling server and room code
    fn room_url(room_code: &str) -> Result<String, String> {
        let base = Self::signaling_server_url()?;
        Ok(format!("{}/room/{}", base.trim_end_matches('/'), room_code))
    }
}

/// Spawn combined tasks for WebRTC socket: driver future and event loop.
/// Returns a join handle that can be aborted to clean up both.
fn spawn_socket_tasks(
    socket: Arc<Mutex<Option<WebRtcSocket>>>,
    driver: impl std::future::Future<Output = ()> + Send + 'static,
    msg_tx: broadcast::Sender<String>,
    peer_id: Arc<Mutex<Option<PeerId>>>,
    app: AppHandle,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let driver_handle = tokio::spawn(async move { driver.await; });
        let event_handle = tokio::spawn({
            let socket = socket.clone();
            async move {
                loop {
                    let mut guard = socket.lock().await;
                    if let Some(socket) = guard.as_mut() {
                        match socket.next_event().await {
                            Some(event) => {
                                match event {
                                    matchbox_socket::Event::Packet(packet) => {
                                        if let Ok(msg) = String::from_utf8(packet) {
                                            let _ = msg_tx.send(msg);
                                        }
                                    }
                                    matchbox_socket::Event::PeerConnected(id) => {
                                        *peer_id.lock().await = Some(id);
                                        let _ = app.emit("peer-connected", ());
                                    }
                                    matchbox_socket::Event::PeerDisconnected(id) => {
                                        if peer_id.lock().await.as_ref() == Some(&id) {
                                            *peer_id.lock().await = None;
                                            let _ = app.emit("peer-disconnected", ());
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            None => break,
                        }
                    } else {
                        break;
                    }
                }
            }
        });
        // Wait for either task to finish, then abort the other.
        tokio::select! {
            _ = driver_handle => { event_handle.abort(); }
            _ = event_handle => { driver_handle.abort(); }
        }
    })
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

/// Host an online game: create a WebRTC socket and wait for a peer.
/// Returns the 4-digit room code to display to the user.
/// Emits `peer-connected` when the first peer joins.
#[tauri::command]
pub async fn host_online(
    transport: State<'_, WebRtcTransportState>,
    app: AppHandle,
) -> Result<String, String> {
    // Generate room code
    let code = WebRtcTransportState::generate_room_code();
    let room_url = WebRtcTransportState::room_url(&code)?;

    // Create WebRTC socket
    let (socket, message_loop) = WebRtcSocketBuilder::new(&room_url)
        .build()
        .await
        .map_err(|e| format!("Failed to create WebRTC socket: {e}"))?;

    // Store socket and room ID
    *transport.socket.lock().await = Some(socket);
    *transport.room_id.lock().await = Some(code.clone());

    // Spawn combined driver and event loop tasks
    let socket_arc = transport.socket.clone();
    let msg_tx = transport.msg_tx.clone();
    let peer_id = transport.peer_id.clone();
    let handle = spawn_socket_tasks(socket_arc, message_loop, msg_tx, peer_id, app);
    *transport.bg_task.lock().await = Some(handle);

    // Spawn additional task to wait for peer connection (optional)
    // The message loop will emit peer-connected event

    Ok(code)
}

/// Join an online game by entering the host's room code.
/// Emits `peer-connected` on success or `join-error` on failure.
#[tauri::command]
pub async fn join_online(
    transport: State<'_, WebRtcTransportState>,
    app: AppHandle,
    room_code: String,
) -> Result<(), String> {
    let code = room_code.trim().to_uppercase();
    let room_url = WebRtcTransportState::room_url(&code)?;

    // Create WebRTC socket
    let (socket, message_loop) = WebRtcSocketBuilder::new(&room_url)
        .build()
        .await
        .map_err(|e| format!("Failed to create WebRTC socket: {e}"))?;

    // Store socket and room ID
    *transport.socket.lock().await = Some(socket);
    *transport.room_id.lock().await = Some(code.clone());

    // Spawn combined driver and event loop tasks
    let socket_arc = transport.socket.clone();
    let msg_tx = transport.msg_tx.clone();
    let peer_id = transport.peer_id.clone();
    let handle = spawn_socket_tasks(socket_arc, message_loop, msg_tx, peer_id, app);
    *transport.bg_task.lock().await = Some(handle);

    Ok(())
}

/// Send a raw string message on the active WebRTC data channel.
/// Returns `true` if the send succeeded.
pub async fn send_msg(transport: &WebRtcTransportState, msg: String) -> bool {
    let socket_guard = transport.socket.lock().await;
    let peer_guard = transport.peer_id.lock().await;
    if let (Some(socket), Some(peer)) = (socket_guard.as_ref(), peer_guard.as_ref()) {
        // Use channel 0 (unreliable datagram channel)
        match socket.get_channel_mut(0) {
            Ok(channel) => {
                let packet = Packet::from(msg.into_bytes());
                channel.send(packet, *peer).is_ok()
            }
            Err(_) => false,
        }
    } else {
        false
    }
}

/// Reset the transport layer, cleaning up sockets and tasks.
#[tauri::command]
pub async fn reset_transport(transport: State<'_, WebRtcTransportState>) -> Result<(), String> {
    // Abort background task
    if let Some(old) = transport.bg_task.lock().await.take() {
        old.abort();
    }
    // Clear socket
    *transport.socket.lock().await = None;
    *transport.room_id.lock().await = None;
    *transport.peer_id.lock().await = None;
    Ok(())
}

/// Cancel any pending online connection (same as reset_transport).
#[tauri::command]
pub async fn cancel_online(transport: State<'_, WebRtcTransportState>) -> Result<(), String> {
    reset_transport(transport).await
}

/// Get the current room ID (if any)
#[tauri::command]
pub async fn get_room_id(transport: State<'_, WebRtcTransportState>) -> Option<String> {
    transport.room_id.lock().await.clone()
}