use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex};
use rand::Rng;

struct RoomSlot {
    host_write_tx: mpsc::UnboundedSender<String>,
    client_ready:  oneshot::Sender<mpsc::UnboundedSender<String>>,
}

type Rooms = Arc<Mutex<HashMap<String, RoomSlot>>>;

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT").unwrap_or_else(|_| "4000".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await.unwrap();
    let rooms: Rooms = Arc::new(Mutex::new(HashMap::new()));
    println!("Relay listening on {}", addr);

    loop {
        let Ok((stream, peer)) = listener.accept().await else { continue };
        let _ = stream.set_nodelay(true);
        println!("Connection from {}", peer);
        tokio::spawn(handle(stream, rooms.clone()));
    }
}

async fn writer_task(
    mut write: tokio::io::WriteHalf<tokio::net::TcpStream>,
    mut rx: mpsc::UnboundedReceiver<String>,
) {
    while let Some(msg) = rx.recv().await {
        let line = format!("{}\n", msg);
        if write.write_all(line.as_bytes()).await.is_err() { break; }
    }
}

async fn handle(stream: tokio::net::TcpStream, rooms: Rooms) {
    let (read, write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);

    let (my_tx, my_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(writer_task(write, my_rx));

    let mut line = String::new();
    if reader.read_line(&mut line).await.unwrap_or(0) == 0 { return; }
    let cmd = line.trim().to_string();

    if cmd == "HOST" {
        let code = format!("{:04}", rand::thread_rng().gen_range(1000u32..9999));
        let _ = my_tx.send(format!("ROOM:{}", code));

        let (client_ready_tx, client_ready_rx) = oneshot::channel();
        rooms.lock().await.insert(code.clone(), RoomSlot {
            host_write_tx: my_tx.clone(),
            client_ready: client_ready_tx,
        });

        let client_tx = match tokio::time::timeout(
            std::time::Duration::from_secs(300),
            client_ready_rx,
        ).await {
            Ok(Ok(tx)) => tx,
            _ => { rooms.lock().await.remove(&code); return; }
        };

        let _ = my_tx.send("START".to_string());
        let _ = client_tx.send("START".to_string());
        println!("Room {} paired", code);

        // Forward host messages to client
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => { let _ = client_tx.send(line.trim().to_string()); }
            }
        }
        println!("Room {} host disconnected", code);

    } else if let Some(code) = cmd.strip_prefix("JOIN:") {
        let slot = rooms.lock().await.remove(code);
        let Some(slot) = slot else {
            let _ = my_tx.send("ERR".to_string());
            return;
        };

        let host_tx = slot.host_write_tx;
        let _ = slot.client_ready.send(my_tx);

        // Forward client messages to host
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => { let _ = host_tx.send(line.trim().to_string()); }
            }
        }
        println!("Room {} client disconnected", code);
    }
}
