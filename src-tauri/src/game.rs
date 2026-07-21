use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use std::net::SocketAddr;
use tauri::ipc::Channel;
use tauri::State;
use tokio::net::UdpSocket;
use tokio::sync::Mutex as TokioMutex;
use puckduel_core::game::{RenderState, GameState};
use puckduel_core::config::*;

/// Tauri-managed state for the game loop (pointer, running flag, etc.)
pub struct GameEngine {
    pub running: Arc<AtomicBool>,
    pub paused:  Arc<AtomicBool>,
    pub pointer: Arc<Mutex<[f32; 2]>>,
    pub task:    Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl GameEngine {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            paused:  Arc::new(AtomicBool::new(false)),
            pointer: Arc::new(Mutex::new([TABLE_WIDTH / 2.0, TABLE_HEIGHT - 120.0])),
            task:    Mutex::new(None),
        }
    }
}

/// Tauri-managed state for the server UDP connection
pub struct ServerState {
    pub socket: Arc<TokioMutex<Option<Arc<UdpSocket>>>>,
    pub room_code: Arc<TokioMutex<Option<String>>>,
    pub peer_addr: Arc<TokioMutex<Option<String>>>,
}
impl ServerState {
    pub fn new() -> Self {
        Self {
            socket: Arc::new(TokioMutex::new(None)),
            room_code: Arc::new(TokioMutex::new(None)),
            peer_addr: Arc::new(TokioMutex::new(None)),
        }
    }
}

// ─── Commands ────────────────────────────────────────────────────────────────

/// Create a room on the game server. Returns the 4-digit room code.
#[tauri::command]
pub async fn create_room(server: State<'_, ServerState>, server_addr: String) -> Result<String, String> {
    let sock = Arc::new(UdpSocket::bind("0.0.0.0:0").await.map_err(|e| format!("bind: {}", e))?);
    sock.connect(&server_addr).await.map_err(|e| format!("connect: {}", e))?;

    sock.send(b"CREATE").await.map_err(|e| format!("send: {}", e))?;

    let mut buf = [0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(5), sock.recv(&mut buf))
        .await
        .map_err(|_| "timeout waiting for server")?
        .map_err(|e| format!("recv: {}", e))?;
    let resp = String::from_utf8(buf[..n].to_vec()).map_err(|_| "invalid response")?;

    if let Some(code) = resp.strip_prefix("CREATED:") {
        let code = code.trim().to_string();
        *server.socket.lock().await = Some(sock);
        *server.room_code.lock().await = Some(code.clone());
        Ok(code)
    } else {
        Err(format!("unexpected response: {}", resp))
    }
}

/// Wait for a specific text response, skipping PEER messages. Stores peer addr.
async fn recv_until(sock: &UdpSocket, peer_addr: &TokioMutex<Option<String>>, target: &str, timeout_secs: u64) -> Result<(), String> {
    let mut buf = [0u8; 256];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(timeout_secs), sock.recv(&mut buf))
            .await
            .map_err(|_| format!("timeout waiting for {}", target))?
            .map_err(|e| format!("recv: {}", e))?;
        let resp = String::from_utf8(buf[..n].to_vec()).map_err(|_| "invalid response")?;
        let trimmed = resp.trim();
        if trimmed == target {
            return Ok(());
        }
        if let Some(peer) = trimmed.strip_prefix("PEER:") {
            *peer_addr.lock().await = Some(peer.to_string());
        }
    }
}

/// Join an existing room on the game server. Returns when the game starts.
#[tauri::command]
pub async fn join_room(server: State<'_, ServerState>, server_addr: String, room_code: String) -> Result<(), String> {
    let sock = Arc::new(UdpSocket::bind("0.0.0.0:0").await.map_err(|e| format!("bind: {}", e))?);
    sock.connect(&server_addr).await.map_err(|e| format!("connect: {}", e))?;

    let join_cmd = format!("JOIN:{}", room_code.trim());
    sock.send(join_cmd.as_bytes()).await.map_err(|e| format!("send: {}", e))?;

    recv_until(&sock, &server.peer_addr, "JOINED", 5).await?;
    recv_until(&sock, &server.peer_addr, "START", 10).await?;

    *server.socket.lock().await = Some(sock);
    *server.room_code.lock().await = Some(room_code);
    Ok(())
}

/// Create a solo game (vs AI) on the game server. Returns when game starts.
#[tauri::command]
pub async fn create_solo(server: State<'_, ServerState>, server_addr: String) -> Result<(), String> {
    let sock = Arc::new(UdpSocket::bind("0.0.0.0:0").await.map_err(|e| format!("bind: {}", e))?);
    sock.connect(&server_addr).await.map_err(|e| format!("connect: {}", e))?;

    sock.send(b"CREATE_SOLO").await.map_err(|e| format!("send: {}", e))?;

    let mut buf = [0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(5), sock.recv(&mut buf))
        .await
        .map_err(|_| "timeout waiting for server")?
        .map_err(|e| format!("recv: {}", e))?;
    let resp = String::from_utf8(buf[..n].to_vec()).map_err(|_| "invalid response")?;

    if resp.trim() == "START" {
        *server.socket.lock().await = Some(sock);
        Ok(())
    } else {
        Err(format!("unexpected response: {}", resp))
    }
}

/// Wait for opponent to join. Blocks until START received from server (skips PEER).
#[tauri::command]
pub async fn wait_for_opponent(server: State<'_, ServerState>) -> Result<(), String> {
    let sock = server.socket.lock().await.take().ok_or("not connected")?;
    recv_until(&sock, &server.peer_addr, "START", 120).await?;
    *server.socket.lock().await = Some(sock);
    Ok(())
}

/// Try to establish a P2P connection to the peer.
/// Returns (p2p_socket, peer_actual_addr) on success.
async fn try_p2p(peer_addr_str: &str, timeout_secs: u64) -> Option<(Arc<UdpSocket>, SocketAddr)> {
    let parts: Vec<&str> = peer_addr_str.split(':').collect();
    if parts.len() != 2 { return None; }
    let peer_ip: std::net::IpAddr = parts[0].parse().ok()?;
    let peer_port: u16 = parts[1].parse().ok()?;
    let relay_addr: SocketAddr = (peer_ip, peer_port).into();

    let p2p = Arc::new(UdpSocket::bind("0.0.0.0:0").await.ok()?);
    // Send hole-punch packet to peer (address from relay server)
    let _ = p2p.send_to(b"P2P_HELLO", relay_addr).await.ok()?;

    // Wait for response — the peer's actual address (post-NAT) comes from recv_from
    let mut buf = [0u8; 32];
    match tokio::time::timeout(Duration::from_secs(timeout_secs), p2p.recv_from(&mut buf)).await {
        Ok(Ok((n, peer_actual))) if n >= 9 && &buf[..9] == b"P2P_HELLO" => {
            // Send a confirmation so peer knows we're alive
            let _ = p2p.send_to(b"P2P_HELLO", peer_actual).await;
            Some((p2p, peer_actual))
        }
        _ => None,
    }
}

/// Send data to opponent: via P2P send_to and relay send.
async fn send_to_opponent(data: &[u8], p2p: &Option<(Arc<UdpSocket>, SocketAddr)>, relay: &UdpSocket) {
    if let Some((ref p2p_sock, peer_addr)) = *p2p {
        let _ = p2p_sock.send_to(data, peer_addr).await;
    }
    let _ = relay.send(data).await;
}

/// Receive from both P2P (first) and relay. Returns authoritative state if any.
async fn recv_from_both(
    p2p: &Option<(Arc<UdpSocket>, SocketAddr)>,
    relay: &UdpSocket,
    opp_ptr: &mut [f32; 2],
) -> (Option<RenderState>, bool) {
    let mut received_state: Option<RenderState> = None;
    let mut game_over = false;
    let mut buf = [0u8; 2048];

    if let Some((ref p2p_sock, _)) = *p2p {
        loop {
            match p2p_sock.try_recv(&mut buf) {
                Ok(n) if n > 0 && buf[0] == b'I' && n >= 9 => {
                    let px = f32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]);
                    let py = f32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
                    if px.is_finite() && py.is_finite() {
                        *opp_ptr = [px.clamp(PADDLE_RADIUS, TABLE_WIDTH - PADDLE_RADIUS), py];
                    }
                }
                Ok(n) if n > 0 && buf[0] == b'S' => {
                    if let Ok(state) = bincode::deserialize(&buf[1..n]) {
                        received_state = Some(state);
                    }
                }
                _ => break,
            }
        }
    }

    loop {
        match relay.try_recv(&mut buf) {
            Ok(n) if n > 0 && buf[0] == b'I' && n >= 9 => {
                let px = f32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]);
                let py = f32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
                if px.is_finite() && py.is_finite() {
                    *opp_ptr = [px.clamp(PADDLE_RADIUS, TABLE_WIDTH - PADDLE_RADIUS), py];
                }
            }
            Ok(n) if n > 0 && buf[0] == b'S' => {
                if let Ok(state) = bincode::deserialize(&buf[1..n]) {
                    received_state = Some(state);
                }
            }
            Ok(n) if n > 0 => {
                let txt = String::from_utf8_lossy(&buf[..n]);
                if txt.trim() == "GAME_OVER" { game_over = true; }
            }
            _ => break,
        }
    }

    (received_state, game_over)
}

/// Each player runs this loop. The player in whose half the puck resides
/// is authoritative for physics and sends the full RenderState. The other
/// player receives that state and renders it. Supports P2P + relay fallback.
async fn run_split_auth_game(
    relay_sock: Arc<UdpSocket>,
    p2p: Option<(Arc<UdpSocket>, SocketAddr)>,
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    pointer: Arc<Mutex<[f32; 2]>>,
    channel: Channel<RenderState>,
    is_host: bool,
    is_single_player: bool,
) {
    use puckduel_core::config::{TABLE_HEIGHT, WINNING_SCORE};

    let mut gs = GameState::new();
    let mut opp_ptr = [TABLE_WIDTH / 2.0, 120.0];
    let mut was_authoritative = is_host;
    let dt = 1.0 / 60.0;
    let mut ai_opponent = [TABLE_WIDTH / 2.0, 120.0];

    let mut interval = tokio::time::interval(Duration::from_secs_f64(1.0 / 60.0));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    while running.load(Ordering::Relaxed) {
        interval.tick().await;

        if paused.load(Ordering::Relaxed) {
            continue;
        }

        let my_ptr = *pointer.lock().unwrap();

        // Send my paddle via P2P + relay
        {
            let mut out = Vec::with_capacity(9);
            out.push(b'I');
            out.extend_from_slice(&my_ptr[0].to_le_bytes());
            out.extend_from_slice(&my_ptr[1].to_le_bytes());
            send_to_opponent(&out, &p2p, &relay_sock).await;
        }

        // Receive from both
        let (received_state, game_over) = recv_from_both(&p2p, &relay_sock, &mut opp_ptr).await;
        if game_over {
            running.store(false, Ordering::Relaxed);
            return;
        }

        // AI opponent
        if is_single_player {
            let p = &gs.puck;
            if p.y < TABLE_HEIGHT / 2.0 || (p.vy < -30.0 && p.y < TABLE_HEIGHT * 0.6) {
                let ttr = if p.vy.abs() > 1.0 { ((p.y - 120.0) / p.vy).abs().min(0.5) } else { 0.3 };
                let tx = (p.x + p.vx * ttr).clamp(PADDLE_RADIUS, TABLE_WIDTH - PADDLE_RADIUS);
                let ty = (p.y - 40.0).clamp(PADDLE_RADIUS, TABLE_HEIGHT / 2.0 - PADDLE_RADIUS);
                let err = ((p.x * 1.7 + p.y * 3.1) * 100.0).sin() * 12.0;
                ai_opponent[0] += (tx + err - ai_opponent[0]).clamp(-8.0, 8.0);
                ai_opponent[1] += (ty - ai_opponent[1]).clamp(-6.0, 6.0);
            }
            opp_ptr = ai_opponent;
        }

        // Authority check with hysteresis to prevent rapid flipping at midline
        let puck_in_my_half = if is_single_player {
            true
        } else if is_host {
            if gs.puck.y >= TABLE_HEIGHT / 2.0 + AUTH_HYSTERESIS {
                true
            } else if gs.puck.y <= TABLE_HEIGHT / 2.0 - AUTH_HYSTERESIS {
                false
            } else {
                was_authoritative
            }
        } else {
            if gs.puck.y <= TABLE_HEIGHT / 2.0 - AUTH_HYSTERESIS {
                true
            } else if gs.puck.y >= TABLE_HEIGHT / 2.0 + AUTH_HYSTERESIS {
                false
            } else {
                was_authoritative
            }
        };
        was_authoritative = puck_in_my_half;

        if puck_in_my_half {
            // server_update takes (host_ptr, client_ptr) — host is always host player
            let (h_ptr, c_ptr) = if is_host { (my_ptr, opp_ptr) } else { (opp_ptr, my_ptr) };
            gs.server_update(dt, h_ptr, c_ptr);
            let state = gs.to_render();

            if channel.send(state.clone()).is_err() { return; }

            if let Ok(encoded) = bincode::serialize(&state) {
                let mut out = Vec::with_capacity(1 + encoded.len());
                out.push(b'S');
                out.extend_from_slice(&encoded);
                send_to_opponent(&out, &p2p, &relay_sock).await;
            }

            if state.score[0] >= WINNING_SCORE || state.score[1] >= WINNING_SCORE {
                let _ = relay_sock.send(b"GAME_OVER").await;
                running.store(false, Ordering::Relaxed);
                return;
            }
        } else if let Some(state) = received_state {
            gs.puck.x = state.puck[0];
            gs.puck.y = state.puck[1];
            gs.puck.vx = state.puck_vx;
            gs.puck.vy = state.puck_vy;
            gs.score = state.score;
            if channel.send(state).is_err() { return; }
            if gs.score[0] >= WINNING_SCORE || gs.score[1] >= WINNING_SCORE {
                running.store(false, Ordering::Relaxed);
                return;
            }
        }
    }
}

/// Start the game loop: split-authority + P2P (with relay fallback).
#[tauri::command]
pub async fn start_game(
    engine: State<'_, GameEngine>,
    server: State<'_, ServerState>,
    is_host: bool,
    is_single_player: bool,
    start_received: bool,
    channel: Channel<RenderState>,
) -> Result<(), String> {
    {
        let old = engine.task.lock().unwrap().take();
        if let Some(h) = old { h.abort(); }
    }
    engine.running.store(false, Ordering::SeqCst);
    engine.paused.store(false, Ordering::SeqCst);
    *engine.pointer.lock().unwrap() = if is_host { [TABLE_WIDTH / 2.0, TABLE_HEIGHT - 120.0] } else { [TABLE_WIDTH / 2.0, 120.0] };
    engine.running.store(true, Ordering::SeqCst);

    let running = engine.running.clone();
    let paused = engine.paused.clone();
    let pointer = engine.pointer.clone();

    let relay_sock = server.socket.lock().await.take().ok_or("not connected to server")?;
    let _room_code = server.room_code.lock().await.take().unwrap_or_default();
    let peer_addr_str = server.peer_addr.lock().await.take();

    // Try P2P hole-punching (2s timeout)
    let p2p = if let Some(ref addr) = peer_addr_str {
        try_p2p(addr, 2).await
    } else {
        None
    };

    let handle = tokio::spawn(async move {
        run_split_auth_game(relay_sock, p2p, running, paused, pointer, channel, is_host, is_single_player).await;
    });

    *engine.task.lock().unwrap() = Some(handle);

    Ok(())
}

#[tauri::command]
pub fn stop_game(engine: State<'_, GameEngine>) {
    engine.running.store(false, Ordering::SeqCst);
    engine.paused.store(false, Ordering::SeqCst);
    if let Some(h) = engine.task.lock().unwrap().take() {
        h.abort();
    }
}

#[tauri::command]
pub fn pause_game(engine: State<'_, GameEngine>) {
    engine.paused.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn resume_game(engine: State<'_, GameEngine>) {
    engine.paused.store(false, Ordering::SeqCst);
}

#[tauri::command]
pub fn set_pointer(engine: State<'_, GameEngine>, x: f32, y: f32) {
    *engine.pointer.lock().unwrap() = [x, y];
}
