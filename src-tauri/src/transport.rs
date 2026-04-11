use std::sync::Arc;
use matchbox_socket::{WebRtcSocket, WebRtcSocketBuilder, ChannelConfig, RtcIceServerConfig, PeerId, Packet, PeerState};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{broadcast, Mutex};
use tokio::time::Duration;
use rand::Rng;
use log::{info, warn, error};
use crate::config::network;

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

    /// Get a clone of the socket Arc
    pub fn get_socket(&self) -> Arc<Mutex<Option<WebRtcSocket>>> {
        self.socket.clone()
    }

    /// Get a clone of the peer_id Arc
    pub fn get_peer_id(&self) -> Arc<Mutex<Option<PeerId>>> {
        self.peer_id.clone()
    }

    /// Get a reference to the msg_tx channel
    pub fn get_msg_tx(&self) -> &broadcast::Sender<String> {
        &self.msg_tx
    }

    /// Get signaling server URL from environment variable with default fallback
    fn signaling_server_url() -> Result<String, String> {
        // Use compile-time environment variable if set, otherwise use default public server
        // This is set via build.rs which loads src-tauri/.env at compile time
        Ok(option_env!("MATCHBOX_SIGNALING_URL")
            .unwrap_or("wss://puckduel.dano.win")
            .to_string())
    }

    /// Generate a random 4-digit room code
    fn generate_room_code() -> String {
        format!("{:04}", rand::thread_rng().gen_range(0..10000))
    }

    /// Build room URL from signaling server and room code
    fn room_url(room_code: &str) -> Result<String, String> {
        let base = Self::signaling_server_url()?;
        // Room code goes directly in path without /room/ prefix
        Ok(format!("{}/{}", base.trim_end_matches('/'), room_code))
    }
}

/// Spawn combined tasks for WebRTC socket: driver future and event loop.
/// Returns a join handle that can be aborted to clean up both.
fn spawn_socket_tasks(
    socket: Arc<Mutex<Option<WebRtcSocket>>>,
    driver: impl std::future::Future<Output = Result<(), matchbox_socket::Error>> + Send + 'static,
    msg_tx: broadcast::Sender<String>,
    peer_id: Arc<Mutex<Option<PeerId>>>,
    app: AppHandle,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut driver_handle = tokio::spawn(async move { 
            if let Err(e) = driver.await {
                error!("WebRTC socket driver error: {:?}", e);
            }
        });
        let mut event_handle = tokio::spawn({
            let socket = socket.clone();
            async move {
                loop {
                    // Small sleep to avoid busy-waiting — keep this low to process
                    // network packets every frame for smooth multiplayer
                    tokio::time::sleep(Duration::from_millis(network::SOCKET_POLL_INTERVAL_MS)).await;

                    let mut guard = socket.lock().await;
                    let Some(socket) = guard.as_mut() else {
                        break; // socket cleaned up
                    };

                    // Update peer state changes
                    let peer_changes = socket.update_peers();
                    for (id, state) in peer_changes {
                        match state {
                            PeerState::Connected => {
                                *peer_id.lock().await = Some(id);
                                info!("Peer connected: {:?}", id);
                                let _ = app.emit("peer-connected", ());
                            }
                            PeerState::Disconnected => {
                                if peer_id.lock().await.as_ref() == Some(&id) {
                                    *peer_id.lock().await = None;
                                    warn!("Peer disconnected: {:?}", id);
                                    let _ = app.emit("peer-disconnected", ());
                                }
                            }
                        }
                    }

                    // Receive incoming messages
                    if let Ok(channel) = socket.get_channel_mut(0) {
                        for (_peer, packet) in channel.receive() {
                            if let Ok(msg) = String::from_utf8(packet.to_vec()) {
                                let _ = msg_tx.send(msg);
                            } else {
                                warn!("Received invalid UTF-8 network packet");
                            }
                        }
                    }
                }
            }
        });
        // Wait for either task to finish, then abort the other.
        tokio::select! {
            _ = &mut driver_handle => { event_handle.abort(); }
            _ = &mut event_handle => { driver_handle.abort(); }
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

    eprintln!("🌍 [host_online] Starting with room code: {}", code);
    eprintln!("🌍 [host_online] Room URL: {}", room_url);

    // Create WebRTC socket using builder with multiple STUN servers for better connectivity
    eprintln!("🌍 [host_online] Creating WebRTC socket...");
    let ice_config = RtcIceServerConfig {
        urls: vec![
            "stun:stun.l.google.com:19302".into(),
            "stun:stun1.l.google.com:19302".into(),
            "stun:stun2.l.google.com:19302".into(),
            "stun:stun3.l.google.com:19302".into(),
            "stun:stun4.l.google.com:19302".into(),
            "stun:stun.cloudflare.com:3478".into(),
        ],
        ..Default::default()
    };
    let (socket, message_loop) = WebRtcSocketBuilder::new(&room_url)
        .add_channel(ChannelConfig::unreliable())
        .ice_server(ice_config)
        .build();
    eprintln!("🌍 [host_online] WebRTC socket created successfully");

    // Prepare data for spawning tasks
    let msg_tx = transport.msg_tx.clone();
    let peer_id = transport.peer_id.clone();
    
    // Store socket and room ID (drop the guard immediately after)
    eprintln!("🌍 [host_online] Storing socket in state...");
    {
        let mut socket_guard = transport.socket.lock().await;
        *socket_guard = Some(socket);
        drop(socket_guard);
    }
    {
        let mut room_id_guard = transport.room_id.lock().await;
        *room_id_guard = Some(code.clone());
        drop(room_id_guard);
    }
    eprintln!("🌍 [host_online] Socket stored");

    // Spawn combined driver and event loop tasks
    let socket_arc = transport.socket.clone();
    eprintln!("🌍 [host_online] Spawning socket tasks...");
    let handle = spawn_socket_tasks(socket_arc, message_loop, msg_tx, peer_id, app);
    
    {
        let mut bg_task_guard = transport.bg_task.lock().await;
        *bg_task_guard = Some(handle);
        drop(bg_task_guard);
    }
    eprintln!("🌍 [host_online] Socket tasks spawned");

    eprintln!("🌍 [host_online] Returning room code to frontend");
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

    info!("Joining online game with room code: {}", code);

    // Create WebRTC socket using builder with multiple STUN servers for better connectivity
    let ice_config = RtcIceServerConfig {
        urls: vec![
            "stun:stun.l.google.com:19302".into(),
            "stun:stun1.l.google.com:19302".into(),
            "stun:stun2.l.google.com:19302".into(),
            "stun:stun3.l.google.com:19302".into(),
            "stun:stun4.l.google.com:19302".into(),
            "stun:stun.cloudflare.com:3478".into(),
        ],
        ..Default::default()
    };
    let (socket, message_loop) = WebRtcSocketBuilder::new(&room_url)
        .add_channel(ChannelConfig::unreliable())
        .ice_server(ice_config)
        .build();

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
    let mut socket_guard = transport.socket.lock().await;
    let peer_guard = transport.peer_id.lock().await;
    if let (Some(socket), Some(peer)) = (socket_guard.as_mut(), peer_guard.as_ref()) {
        // Use channel 0 (unreliable datagram channel)
        match socket.get_channel_mut(0) {
            Ok(channel) => {
                let packet = Packet::from(msg.into_bytes());
                if let Err(e) = channel.try_send(packet, *peer) {
                    warn!("Failed to send network message: {:?}", e);
                    false
                } else {
                    true
                }
            }
            Err(e) => {
                warn!("Failed to get network channel: {:?}", e);
                false
            }
        }
    } else {
        warn!("Cannot send: socket or peer not available");
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
pub async fn get_room_id(transport: State<'_, WebRtcTransportState>) -> Result<Option<String>, String> {
    Ok(transport.room_id.lock().await.clone())
}