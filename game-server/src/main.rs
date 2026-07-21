use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::Duration;
use rand::Rng;
use puckduel_core::game::GameState;
use puckduel_core::config;

const MAX_ROOMS: usize = 256;
const ROOM_TIMEOUT_SECS: u64 = 120;
const GAME_TICK_HZ: f64 = 60.0;
const TABLE_W: f32 = config::TABLE_WIDTH;
const TABLE_H: f32 = config::TABLE_HEIGHT;
const PAD_R: f32 = config::PADDLE_RADIUS;

type ClientMap = Arc<Mutex<HashMap<String, Room>>>;

struct Room {
    creator: SocketAddr,
    joiner: Option<SocketAddr>,
    created_at: Instant,
    is_ai: bool,
    game: Option<GameLoop>,
}

struct GameLoop {
    gs: GameState,
    host_input: [f32; 2],
    client_input: [f32; 2],
}

fn generate_room_code() -> String {
    format!("{:04}", rand::thread_rng().gen_range(0..10000))
}

// ─── AI ──────────────────────────────────────────────────────────────────────

fn clamp_x(x: f32) -> f32 {
    x.clamp(PAD_R, TABLE_W - PAD_R)
}

fn ai_update(gs: &GameState, prev_input: &mut [f32; 2], dt: f32) -> [f32; 2] {
    // AI controls the client (top) paddle
    // Target: center of table
    let mut target_x = TABLE_W / 2.0;
    let mut target_y = 100.0; // home position

    // If puck is in AI's half or approaching, chase it
    let puck_in_ai_half = gs.puck.y < TABLE_H / 2.0;
    let puck_coming = gs.puck.vy < -30.0;

    if puck_in_ai_half || (puck_coming && gs.puck.y < TABLE_H * 0.6) {
        // Predict where puck will be
        let time_to_reach = if gs.puck.vy.abs() > 1.0 {
            ((gs.puck.y - 120.0) / gs.puck.vy).abs().min(0.5)
        } else {
            0.3
        };
        target_x = gs.puck.x + gs.puck.vx * time_to_reach;
        target_x = clamp_x(target_x);
        target_y = (gs.puck.y - 40.0).clamp(PAD_R, TABLE_H / 2.0 - PAD_R);
    }

    // Add slight error for realism
    let error = ((gs.puck.x * 1.7 + gs.puck.y * 3.1 + gs.score[0] as f32 * 13.7 + gs.score[1] as f32 * 7.3) * 100.0).sin() * 12.0;
    target_x += error;

    // Smooth move toward target
    let speed = if puck_in_ai_half { 12.0 } else { 5.0 };
    let max_step = speed * 60.0 * dt;
    let dx = (target_x - prev_input[0]).clamp(-max_step, max_step);
    let dy = (target_y - prev_input[1]).clamp(-max_step, max_step);
    prev_input[0] = clamp_x(prev_input[0] + dx);
    prev_input[1] = (prev_input[1] + dy).clamp(PAD_R, TABLE_H / 2.0 - PAD_R);
    *prev_input
}

// ─── Command handler ─────────────────────────────────────────────────────────

async fn handle_command(socket: &Arc<UdpSocket>, rooms: &ClientMap, cmd: String, src: SocketAddr) {
    let mut guard = rooms.lock().await;

    if cmd.starts_with("CREATE_SOLO") {
        if guard.len() >= MAX_ROOMS { return; }
        let code = format!("SOLO_{}", rand::thread_rng().gen_range(1000..9999));
        let gs = GameState::new();
        let host_input = [TABLE_W / 2.0, TABLE_H - 120.0];
        let client_input = [TABLE_W / 2.0, 120.0];
        guard.insert(code.clone(), Room {
            creator: src,
            joiner: None,
            created_at: Instant::now(),
            is_ai: true,
            game: Some(GameLoop { gs, host_input, client_input }),
        });
        let _ = socket.send_to(b"START", src).await;
        println!("Solo game for {}", src);

        drop(guard);

        let socket = socket.clone();
        let rooms = rooms.clone();
        tokio::spawn(async move {
            run_ai_game(socket, rooms, code, src).await;
        });
        return;
    }

    if cmd.starts_with("CREATE") {
        if guard.len() >= MAX_ROOMS {
            let _ = socket.send_to(b"BUSY", src).await;
            return;
        }
        if guard.values().any(|r| r.creator == src && r.joiner.is_none()) {
            let _ = socket.send_to(b"ALREADY_HOSTING", src).await;
            return;
        }
        let code = loop {
            let c = generate_room_code();
            if !guard.contains_key(&c) { break c; }
        };
        guard.insert(code.clone(), Room {
            creator: src,
            joiner: None,
            created_at: Instant::now(),
            is_ai: false,
            game: None,
        });
        let _ = socket.send_to(format!("CREATED:{}", code).as_bytes(), src).await;
        println!("Room {} created by {}", code, src);
        return;
    }

    if cmd.starts_with("JOIN:") {
        let code = cmd[5..].trim().to_string();
        if let Some(room) = guard.get_mut(&code) {
            if room.joiner.is_some() {
                let _ = socket.send_to(b"FULL", src).await;
                return;
            }
            if room.creator == src {
                let _ = socket.send_to(b"CANNOT_JOIN_OWN", src).await;
                return;
            }
            room.joiner = Some(src);

            let host = room.creator;
            let gs = GameState::new();
            let host_input = [TABLE_W / 2.0, TABLE_H - 120.0];
            let client_input = [TABLE_W / 2.0, 120.0];
            room.game = Some(GameLoop { gs, host_input, client_input });

            let _ = socket.send_to(b"JOINED", src).await;
            let _ = socket.send_to(b"START", host).await;
            let _ = socket.send_to(b"START", src).await;
            println!("Game started in room {}: {} vs {}", code, host, src);

            drop(guard);

            let socket = socket.clone();
            let rooms = rooms.clone();
            tokio::spawn(async move {
                run_game(socket, rooms, code, host, src).await;
            });
            return;
        }
        let _ = socket.send_to(b"NOT_FOUND", src).await;
    }
}

// ─── Input handler ───────────────────────────────────────────────────────────

async fn handle_input(rooms: &ClientMap, room_id: &str, src: SocketAddr, input: [f32; 2]) {
    let mut guard = rooms.lock().await;
    let room = if room_id.is_empty() {
        guard.values_mut().find(|r| r.creator == src || r.joiner == Some(src))
    } else {
        guard.get_mut(room_id)
    };
    if let Some(room) = room {
        if let Some(ref mut game) = room.game {
            let clamped = [input[0].clamp(PAD_R, TABLE_W - PAD_R), input[1]];
            if src == room.creator {
                game.host_input = clamped;
            } else if Some(src) == room.joiner {
                game.client_input = clamped;
            }
        }
    }
}

// ─── Multiplayer game loop ───────────────────────────────────────────────────

async fn run_game(socket: Arc<UdpSocket>, rooms: ClientMap, room_id: String, host: SocketAddr, joiner: SocketAddr) {
    let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / GAME_TICK_HZ));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let dt = 1.0 / GAME_TICK_HZ as f32;

    loop {
        tick.tick().await;

        let result = {
            let mut guard = rooms.lock().await;
            let room = match guard.get_mut(&room_id) {
                Some(r) => r,
                None => break,
            };
            let game = match room.game.as_mut() {
                Some(g) => g,
                None => break,
            };
            game.gs.server_update(dt, game.host_input, game.client_input);
            (game.gs.to_render(), game.gs.game_over)
        };

        let (state, game_over) = result;

        if let Ok(encoded) = bincode::serialize(&state) {
            let mut packet = vec![0u8; encoded.len() + 1];
            packet[0] = b'S';
            packet[1..].copy_from_slice(&encoded);
            let _ = socket.send_to(&packet, host).await;
            let _ = socket.send_to(&packet, joiner).await;
        }

        if game_over {
            let _ = socket.send_to(b"GAME_OVER", host).await;
            let _ = socket.send_to(b"GAME_OVER", joiner).await;
            println!("Game over in room {}", room_id);
            break;
        }
    }

    let mut guard = rooms.lock().await;
    guard.remove(&room_id);
    println!("Room {} closed", room_id);
}

// ─── AI game loop ────────────────────────────────────────────────────────────

async fn run_ai_game(socket: Arc<UdpSocket>, rooms: ClientMap, room_id: String, player: SocketAddr) {
    let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / GAME_TICK_HZ));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let dt = 1.0 / GAME_TICK_HZ as f32;

    loop {
        tick.tick().await;

        let result = {
            let mut guard = rooms.lock().await;
            let room = match guard.get_mut(&room_id) {
                Some(r) => r,
                None => break,
            };
            let game = match room.game.as_mut() {
                Some(g) => g,
                None => break,
            };

            // Compute AI paddle position
            game.client_input = ai_update(&game.gs, &mut game.client_input, dt);

            game.gs.server_update(dt, game.host_input, game.client_input);
            (game.gs.to_render(), game.gs.game_over)
        };

        let (state, game_over) = result;

        if let Ok(encoded) = bincode::serialize(&state) {
            let mut packet = vec![0u8; encoded.len() + 1];
            packet[0] = b'S';
            packet[1..].copy_from_slice(&encoded);
            let _ = socket.send_to(&packet, player).await;
        }

        if game_over {
            let _ = socket.send_to(b"GAME_OVER", player).await;
            println!("Game over in solo room {}", room_id);
            break;
        }
    }

    let mut guard = rooms.lock().await;
    guard.remove(&room_id);
    println!("Solo room {} closed", room_id);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = std::env::var("PORT").unwrap_or_else(|_| "9876".into());
    let addr = format!("[::]:{}", port);
    let socket = Arc::new(UdpSocket::bind(&addr).await?);
    println!("Game server listening on {}", addr);

    let rooms: ClientMap = Arc::new(Mutex::new(HashMap::new()));

    // Periodic cleanup of stale rooms
    let cleanup_rooms = rooms.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let mut guard = cleanup_rooms.lock().await;
            let before = guard.len();
            guard.retain(|_, r| r.joiner.is_some() || r.is_ai || r.created_at.elapsed().as_secs() < ROOM_TIMEOUT_SECS);
            let removed = before - guard.len();
            if removed > 0 {
                println!("Cleanup: removed {} stale room(s), {} remaining", removed, guard.len());
            }
        }
    });

    let mut buf = [0u8; 2048];

    loop {
        let (len, src) = socket.recv_from(&mut buf).await?;
        let data = buf[..len].to_vec();

        if data.is_empty() { continue; }

        if data[0].is_ascii_alphabetic() {
            if let Ok(cmd) = String::from_utf8(data) {
                handle_command(&socket, &rooms, cmd, src).await;
            }
            continue;
        }

        // Binary input: room_id_len (1 byte) + room_id (N bytes) + paddle_x (4) + paddle_y (4)
        if data.len() >= 9 {
            let id_len = data[0] as usize;
            if id_len <= 32 && data.len() >= 1 + id_len + 8 {
                if let Ok(room_id) = String::from_utf8(data[1..1+id_len].to_vec()) {
                    let px = f32::from_le_bytes([
                        data[1+id_len], data[2+id_len], data[3+id_len], data[4+id_len]
                    ]);
                    let py = f32::from_le_bytes([
                        data[5+id_len], data[6+id_len], data[7+id_len], data[8+id_len]
                    ]);
                    if px.is_finite() && py.is_finite() {
                        handle_input(&rooms, &room_id, src, [px, py]).await;
                    }
                }
            }
        }
    }
}
