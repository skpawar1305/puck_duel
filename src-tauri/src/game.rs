use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
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
}
impl ServerState {
    pub fn new() -> Self {
        Self {
            socket: Arc::new(TokioMutex::new(None)),
            room_code: Arc::new(TokioMutex::new(None)),
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

/// Wait for opponent to join. Blocks until START received from server.
#[tauri::command]
pub async fn wait_for_opponent(server: State<'_, ServerState>) -> Result<(), String> {
    let sock = server.socket.lock().await.take().ok_or("not connected")?;
    let mut buf = [0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(120), sock.recv(&mut buf))
        .await
        .map_err(|_| "timeout waiting for opponent")?
        .map_err(|e| format!("recv: {}", e))?;
    let resp = String::from_utf8_lossy(&buf[..n]);
    if resp.trim() != "START" {
        return Err(format!("unexpected: {}", resp));
    }
    *server.socket.lock().await = Some(sock);
    Ok(())
}

/// Each player runs this loop. The player in whose half the puck resides
/// is authoritative for physics and sends the full RenderState. The other
/// player receives that state and renders it.
async fn run_split_auth_game(
    sock: Arc<UdpSocket>,
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

        // Send my paddle to opponent (via relay server)
        {
            let mut out = Vec::with_capacity(9);
            out.push(b'I');
            out.extend_from_slice(&my_ptr[0].to_le_bytes());
            out.extend_from_slice(&my_ptr[1].to_le_bytes());
            let _ = sock.send(&out).await;
        }

        // Drain socket: opponent paddle + authoritative state
        let mut received_state: Option<RenderState> = None;
        loop {
            let mut buf = [0u8; 2048];
            match sock.try_recv(&mut buf) {
                Ok(n) if n > 0 && buf[0] == b'I' && n >= 9 => {
                    let px = f32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]);
                    let py = f32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
                    if px.is_finite() && py.is_finite() {
                        opp_ptr = [px.clamp(PADDLE_RADIUS, TABLE_WIDTH - PADDLE_RADIUS), py];
                    }
                }
                Ok(n) if n > 0 && buf[0] == b'S' => {
                    if let Ok(state) = bincode::deserialize(&buf[1..n]) {
                        received_state = Some(state);
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

        // AI opponent for single player
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

        // Am I authoritative? (puck in my half)
        // Host owns bottom half (y > TH/2), joiner owns top half (y < TH/2)
        let puck_in_my_half = if is_single_player {
            true
        } else if is_host {
            gs.puck.y >= TABLE_HEIGHT / 2.0
        } else {
            gs.puck.y <= TABLE_HEIGHT / 2.0
        };

        if puck_in_my_half {
            // I'm authoritative
            gs.server_update(dt, my_ptr, opp_ptr);
            let state = gs.to_render();

            if channel.send(state.clone()).is_err() {
                return;
            }

            // Send authoritative state to opponent
            if let Ok(encoded) = bincode::serialize(&state) {
                let mut out = Vec::with_capacity(1 + encoded.len());
                out.push(b'S');
                out.extend_from_slice(&encoded);
                let _ = sock.send(&out).await;
            }

            if state.score[0] >= WINNING_SCORE || state.score[1] >= WINNING_SCORE {
                let _ = sock.send(b"GAME_OVER").await;
                running.store(false, Ordering::Relaxed);
                return;
            }
        } else if let Some(state) = received_state {
            // Opponent is authoritative: use their state
            gs.puck.x = state.puck[0];
            gs.puck.y = state.puck[1];
            gs.puck.vx = state.puck_speed * 0.5 * gs.puck.vx.signum().max(0.01);
            gs.score = state.score;

            if channel.send(state).is_err() {
                return;
            }

            if gs.score[0] >= WINNING_SCORE || gs.score[1] >= WINNING_SCORE {
                running.store(false, Ordering::Relaxed);
                return;
            }
        }
    }
}

/// Start the game loop with split-authority: the player on whose side the puck
/// resides runs physics and sends the authoritative state to the opponent.
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

    let sock = server.socket.lock().await.take().ok_or("not connected to server")?;
    let _room_code = server.room_code.lock().await.take().unwrap_or_default();

    let handle = tokio::spawn(async move {
        run_split_auth_game(sock, running, paused, pointer, channel, is_host, is_single_player).await;
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
