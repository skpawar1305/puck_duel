use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};
use tauri::{Emitter, State};

// Change this to your Fly.io app hostname after deploying
const RELAY_HOST: &str = "puck-duel-relay.fly.dev:4000";

pub struct RelayState {
    pub write_tx:  Mutex<Option<mpsc::UnboundedSender<String>>>,
    pub msg_tx:    broadcast::Sender<String>,
    pub connected: AtomicBool,
}

impl RelayState {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(32);
        Self {
            write_tx:  Mutex::new(None),
            msg_tx:    tx,
            connected: AtomicBool::new(false),
        }
    }
}

async fn connect_and_handshake(cmd: &str) -> Result<(BufReader<tokio::io::ReadHalf<TcpStream>>, tokio::io::WriteHalf<TcpStream>), String> {
    let stream = TcpStream::connect(RELAY_HOST).await.map_err(|e| e.to_string())?;
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    let (read, mut write) = tokio::io::split(stream);
    write.write_all(format!("{}\n", cmd).as_bytes()).await.map_err(|e| e.to_string())?;
    Ok((BufReader::new(read), write))
}

fn spawn_reader(
    mut reader: BufReader<tokio::io::ReadHalf<TcpStream>>,
    msg_tx: broadcast::Sender<String>,
) {
    tokio::spawn(async move {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => { let _ = msg_tx.send(line.trim().to_string()); }
            }
        }
    });
}

fn spawn_writer(
    mut write: tokio::io::WriteHalf<TcpStream>,
    mut rx: mpsc::UnboundedReceiver<String>,
) {
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if write.write_all(format!("{}\n", msg).as_bytes()).await.is_err() { break; }
        }
    });
}

#[tauri::command]
pub async fn connect_relay_host(
    app: tauri::AppHandle,
    relay: State<'_, RelayState>,
) -> Result<String, String> {
    let (mut reader, write) = connect_and_handshake("HOST").await?;

    // Read ROOM:XXXX response
    let mut line = String::new();
    reader.read_line(&mut line).await.map_err(|e| e.to_string())?;
    let room_code = line.trim().strip_prefix("ROOM:").ok_or("Bad relay response")?.to_string();

    // Set up write channel
    let (write_tx, write_rx) = mpsc::unbounded_channel::<String>();
    *relay.write_tx.lock().unwrap() = Some(write_tx);
    relay.connected.store(true, Ordering::Relaxed);
    spawn_writer(write, write_rx);

    // Wait for START in background, then emit event
    let msg_tx = relay.msg_tx.clone();
    let app2 = app.clone();
    tokio::spawn(async move {
        let mut line = String::new();
        reader.read_line(&mut line).await.ok();
        if line.trim() == "START" {
            let _ = app2.emit("relay-peer-connected", ());
            spawn_reader(reader, msg_tx);
        }
    });

    Ok(room_code)
}

#[tauri::command]
pub async fn connect_relay_join(
    app: tauri::AppHandle,
    relay: State<'_, RelayState>,
    room_code: String,
) -> Result<(), String> {
    let (mut reader, write) = connect_and_handshake(&format!("JOIN:{}", room_code)).await?;

    // Read START or ERR
    let mut line = String::new();
    reader.read_line(&mut line).await.map_err(|e| e.to_string())?;
    match line.trim() {
        "START" => {}
        "ERR" => return Err("Room not found".to_string()),
        other => return Err(format!("Unexpected: {}", other)),
    }

    let (write_tx, write_rx) = mpsc::unbounded_channel::<String>();
    *relay.write_tx.lock().unwrap() = Some(write_tx);
    relay.connected.store(true, Ordering::Relaxed);
    spawn_writer(write, write_rx);
    spawn_reader(reader, relay.msg_tx.clone());

    let _ = app.emit("relay-peer-connected", ());
    Ok(())
}

#[tauri::command]
pub fn disconnect_relay(relay: State<'_, RelayState>) {
    *relay.write_tx.lock().unwrap() = None;
    relay.connected.store(false, Ordering::Relaxed);
}
