use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::State;
use tokio::net::UdpSocket;
use tokio::sync::Mutex as TokioMutex;
use puckduel_core::game::RenderState;
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
    pub cancel_wait: Arc<AtomicBool>,
}
impl ServerState {
    pub fn new() -> Self {
        Self {
            socket: Arc::new(TokioMutex::new(None)),
            room_code: Arc::new(TokioMutex::new(None)),
            cancel_wait: Arc::new(AtomicBool::new(false)),
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

/// Join an existing room on the game server. Returns when the game starts.
#[tauri::command]
pub async fn join_room(server: State<'_, ServerState>, server_addr: String, room_code: String) -> Result<(), String> {
    let sock = Arc::new(UdpSocket::bind("0.0.0.0:0").await.map_err(|e| format!("bind: {}", e))?);
    sock.connect(&server_addr).await.map_err(|e| format!("connect: {}", e))?;

    let join_cmd = format!("JOIN:{}", room_code.trim());
    sock.send(join_cmd.as_bytes()).await.map_err(|e| format!("send: {}", e))?;

    // Wait for JOINED response
    let mut buf = [0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(5), sock.recv(&mut buf))
        .await
        .map_err(|_| "timeout waiting for server")?
        .map_err(|e| format!("recv: {}", e))?;
    let resp = String::from_utf8(buf[..n].to_vec()).map_err(|_| "invalid response")?;

    if resp.trim() != "JOINED" {
        return Err(format!("join failed: {}", resp));
    }

    // Wait for START from server
    let n = tokio::time::timeout(Duration::from_secs(10), sock.recv(&mut buf))
        .await
        .map_err(|_| "timeout waiting for game start")?
        .map_err(|e| format!("recv: {}", e))?;
    let resp2 = String::from_utf8(buf[..n].to_vec()).map_err(|_| "invalid response")?;

    if resp2.trim() != "START" {
        return Err(format!("unexpected: {}", resp2));
    }

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

/// Wait for opponent to join. Blocks until START received from server, then stores socket back.
#[tauri::command]
pub async fn wait_for_opponent(server: State<'_, ServerState>) -> Result<(), String> {
    server.cancel_wait.store(false, Ordering::SeqCst);
    let sock = server.socket.lock().await.take().ok_or("not connected")?;
    let mut buf = [0u8; 64];
    let result = tokio::time::timeout(Duration::from_secs(120), sock.recv(&mut buf)).await;
    if server.cancel_wait.load(Ordering::SeqCst) {
        return Err("cancelled".into());
    }
    let n = result
        .map_err(|_| "timeout waiting for opponent")?
        .map_err(|e| format!("recv: {}", e))?;
    let resp = String::from_utf8_lossy(&buf[..n]);
    if resp.trim() != "START" {
        return Err(format!("unexpected: {}", resp));
    }
    *server.socket.lock().await = Some(sock);
    Ok(())
}

#[tauri::command]
pub fn cancel_wait_for_opponent(server: State<'_, ServerState>) {
    server.cancel_wait.store(true, Ordering::SeqCst);
}

/// Start the game loop: receives state from server, sends input, pushes to JS.
#[tauri::command]
pub async fn start_game(
    engine: State<'_, GameEngine>,
    server: State<'_, ServerState>,
    is_host: bool,
    is_single_player: bool,
    channel: Channel<RenderState>,
) -> Result<(), String> {
    // Abort any existing game
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

    // Take the socket from server state
    let sock = server.socket.lock().await.take().ok_or("not connected to server")?;
    let room_code = server.room_code.lock().await.take().unwrap_or_default();

    // Host needs to wait for START from server (joiner already got it)
    let handle = tokio::spawn(async move {
        if is_host {
            let mut buf = [0u8; 64];
            match tokio::time::timeout(Duration::from_secs(120), sock.recv(&mut buf)).await {
                Ok(Ok(n)) => {
                    let resp = String::from_utf8_lossy(&buf[..n]);
                    if resp.trim() != "START" {
                        return;
                    }
                }
                _ => return,
            }
        }

        // Encode room code prefix once
        let rc_bytes = room_code.as_bytes();
        let rc_len = rc_bytes.len() as u8;

        let mut interval = tokio::time::interval(Duration::from_secs_f64(1.0 / 60.0));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        while running.load(Ordering::Relaxed) {
            interval.tick().await;

            if paused.load(Ordering::Relaxed) {
                continue;
            }

            // Send paddle position to server
            let ptr = *pointer.lock().unwrap();
            let mut out = Vec::with_capacity(1 + rc_bytes.len() + 8);
            out.push(rc_len);
            out.extend_from_slice(rc_bytes);
            out.extend_from_slice(&ptr[0].to_le_bytes());
            out.extend_from_slice(&ptr[1].to_le_bytes());
            let _ = sock.send(&out).await;

            // Receive state from server (non-blocking, take latest)
            loop {
                let mut buf = [0u8; 1024];
                match sock.try_recv(&mut buf) {
                    Ok(n) if n > 0 && buf[0] == b'S' => {
                        if let Ok(state) = bincode::deserialize(&buf[1..n]) {
                            if channel.send(state).is_err() {
                                return;
                            }
                        }
                    }
                    Ok(n) if n > 0 => {
                        let txt = String::from_utf8_lossy(&buf[..n]);
                        if txt.trim() == "GAME_OVER" {
                            running.store(false, Ordering::Relaxed);
                            return;
                        }
                    }
                    _ => break,
                }
            }
        }
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
