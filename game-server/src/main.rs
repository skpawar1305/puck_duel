use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::Duration;
use rand::Rng;

const MAX_ROOMS: usize = 256;
const ROOM_TIMEOUT_SECS: u64 = 120;

type ClientMap = Arc<Mutex<HashMap<String, Room>>>;

struct Room {
    creator: SocketAddr,
    joiner: Option<SocketAddr>,
    created_at: Instant,
    is_ai: bool,
}

fn generate_room_code() -> String {
    format!("{:04}", rand::thread_rng().gen_range(0..10000))
}

// ─── Commands ────────────────────────────────────────────────────────────────

async fn handle_command(socket: &Arc<UdpSocket>, rooms: &ClientMap, cmd: String, src: SocketAddr) {
    let mut guard = rooms.lock().await;

    if cmd.starts_with("CREATE_SOLO") {
        if guard.len() >= MAX_ROOMS { return; }
        let code = format!("SOLO_{}", rand::thread_rng().gen_range(1000..9999));
        guard.insert(code.clone(), Room {
            creator: src,
            joiner: None,
            created_at: Instant::now(),
            is_ai: true,
        });
        let _ = socket.send_to(b"START", src).await;
        println!("Solo game for {}", src);
        return;
    }

    if cmd.starts_with("CREATE") {
        if guard.len() >= MAX_ROOMS {
            let _ = socket.send_to(b"BUSY", src).await;
            return;
        }
        if guard.values().any(|r| r.creator == src) {
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

            // Tell each player the other's public address (for P2P hole-punching)
            let host_peer = format!("PEER:{}:{}", src.ip(), src.port());
            let join_peer = format!("PEER:{}:{}", host.ip(), host.port());
            let _ = socket.send_to(host_peer.as_bytes(), host).await;
            let _ = socket.send_to(join_peer.as_bytes(), src).await;

            let _ = socket.send_to(b"JOINED", src).await;
            let _ = socket.send_to(b"START", host).await;
            let _ = socket.send_to(b"START", src).await;
            println!("Game started in room {}: {} vs {} (P2P capable)", code, host, src);
            return;
        }
        let _ = socket.send_to(b"NOT_FOUND", src).await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = std::env::var("PORT").unwrap_or_else(|_| "9876".into());
    let addr = format!("[::]:{}", port);
    let socket = Arc::new(UdpSocket::bind(&addr).await?);
    println!("Game server listening on {} (relay mode)", addr);

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

        // Text commands
        if data[0].is_ascii_alphabetic() && data[0] != b'I' && data[0] != b'S' {
            if let Ok(cmd) = String::from_utf8(data) {
                handle_command(&socket, &rooms, cmd, src).await;
            }
            continue;
        }

        // Binary — forward to the other player in the same room
        let guard = rooms.lock().await;
        for (_, room) in guard.iter() {
            let other = if room.creator == src {
                room.joiner
            } else if room.joiner == Some(src) {
                Some(room.creator)
            } else {
                None
            };
            if let Some(dst) = other {
                let _ = socket.send_to(&data, dst).await;
                // If GAME_OVER, also send to the other player
                if data.starts_with(b"GAME_OVER") {
                    let _ = socket.send_to(b"GAME_OVER", src).await;
                }
                break;
            }
        }
    }
}
