use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::UdpSocket;
use tokio::sync::broadcast;
use tauri::{State, Emitter};
use local_ip_address::local_ip;

pub struct UdpState {
    pub is_running: Mutex<bool>,
    pub host_socket: Mutex<Option<Arc<UdpSocket>>>,
    pub client_socket: Mutex<Option<Arc<UdpSocket>>>,
    pub client_remote_addr: Mutex<Option<SocketAddr>>,
    pub connected_clients: Arc<Mutex<HashMap<SocketAddr, ()>>>,
    /// Receive tasks publish here; game loop subscribes — no socket race
    pub msg_tx: broadcast::Sender<String>,
}

impl UdpState {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(16);
        Self {
            is_running: Mutex::new(false),
            host_socket: Mutex::new(None),
            client_socket: Mutex::new(None),
            client_remote_addr: Mutex::new(None),
            connected_clients: Arc::new(Mutex::new(HashMap::new())),
            msg_tx: tx,
        }
    }
}

#[tauri::command]
pub async fn start_udp_host(app: tauri::AppHandle, state: State<'_, UdpState>) -> Result<String, String> {
    {
        let mut is_running = state.is_running.lock().unwrap();
        if *is_running { return Ok("Host already running".to_string()); }
        *is_running = true;
    }

    let socket = Arc::new(UdpSocket::bind("0.0.0.0:8080").await.map_err(|e| e.to_string())?);
    *state.host_socket.lock().unwrap() = Some(socket.clone());

    let tx        = state.msg_tx.clone();
    let clients   = state.connected_clients.clone();

    tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        loop {
            if let Ok((len, addr)) = socket.recv_from(&mut buf).await {
                clients.lock().unwrap().entry(addr).or_insert(());
                let msg = String::from_utf8_lossy(&buf[..len]).to_string();
                // JS still needs this for connection detection on the host screen
                let _ = app.emit("udp-msg-received", (addr.to_string(), msg.clone()));
                // game loop subscribes here — zero IPC for game state
                let _ = tx.send(msg);
            }
        }
    });

    Ok("Host UDP started on port 8080".to_string())
}

#[tauri::command]
pub async fn host_send_msg(state: State<'_, UdpState>, msg: String) -> Result<(), String> {
    let socket  = state.host_socket.lock().unwrap().clone();
    let clients = state.connected_clients.lock().unwrap().clone();
    if let Some(sock) = socket {
        for addr in clients.keys() {
            let _ = sock.send_to(msg.as_bytes(), addr).await;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn connect_udp_client(app: tauri::AppHandle, state: State<'_, UdpState>, host_ip: String) -> Result<String, String> {
    let socket    = Arc::new(UdpSocket::bind("0.0.0.0:0").await.map_err(|e| e.to_string())?);
    let host_addr: SocketAddr = format!("{}:8080", host_ip).parse().map_err(|_| "Invalid IP")?;

    *state.client_socket.lock().unwrap()      = Some(socket.clone());
    *state.client_remote_addr.lock().unwrap() = Some(host_addr);

    socket.send_to(r#"{"type":"ping"}"#.as_bytes(), &host_addr).await.map_err(|e| e.to_string())?;

    let tx = state.msg_tx.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        loop {
            if let Ok((len, _)) = socket.recv_from(&mut buf).await {
                let msg = String::from_utf8_lossy(&buf[..len]).to_string();
                let _ = app.emit("udp-msg-received", ("host".to_string(), msg.clone()));
                let _ = tx.send(msg);
            }
        }
    });

    Ok("Connected to UDP Host".to_string())
}

#[tauri::command]
pub async fn client_send_msg(state: State<'_, UdpState>, msg: String) -> Result<(), String> {
    let socket = state.client_socket.lock().unwrap().clone();
    let addr   = state.client_remote_addr.lock().unwrap().clone();
    if let (Some(sock), Some(a)) = (socket, addr) {
        let _ = sock.send_to(msg.as_bytes(), &a).await;
    }
    Ok(())
}

#[tauri::command]
pub fn get_local_ips() -> Result<Vec<String>, String> {
    use local_ip_address::list_afinet_netifas;
    let mut ips = Vec::new();
    if let Ok(ifaces) = list_afinet_netifas() {
        for (name, ip) in &ifaces {
            if !ip.is_ipv4() || ip.is_loopback() { continue; }
            if name.starts_with("docker") || name.starts_with("br-")
            || name.starts_with("veth")   || name.starts_with("tun")
            || name.starts_with("tap")    || name.starts_with("vmnet") { continue; }
            let octets = match ip { std::net::IpAddr::V4(v4) => v4.octets(), _ => continue };
            let private = octets[0] == 10
                || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
                || (octets[0] == 192 && octets[1] == 168);
            if !private { continue; }
            ips.push(ip.to_string());
        }
    }
    if ips.is_empty() {
        ips.push(local_ip().map_err(|e| e.to_string())?.to_string());
    }
    Ok(ips)
}
