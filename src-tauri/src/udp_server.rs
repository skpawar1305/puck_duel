use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::UdpSocket;
use tauri::{State, Emitter};
use local_ip_address::local_ip;

pub struct UdpState {
    pub is_running: Mutex<bool>,
    pub host_socket: Mutex<Option<Arc<UdpSocket>>>,
    pub client_socket: Mutex<Option<Arc<UdpSocket>>>,
    pub client_remote_addr: Mutex<Option<SocketAddr>>, // For client to know where to send to Host
    pub connected_clients: Mutex<HashMap<SocketAddr, ()>>, // Host tracks clients
}

// ----------------------------------------------------
// HOST LOGIC (Server)
// ----------------------------------------------------
#[tauri::command]
pub async fn start_udp_host(app: tauri::AppHandle, state: State<'_, UdpState>) -> Result<String, String> {
    {
        let mut is_running = state.is_running.lock().unwrap();
        if *is_running {
            return Ok("Host already running".to_string());
        }
        *is_running = true;
    }

    // Bind on all interfaces
    let socket = UdpSocket::bind("0.0.0.0:8080").await.map_err(|e| e.to_string())?;
    let socket = Arc::new(socket);
    
    // Store in state so we can send from UI
    *state.host_socket.lock().unwrap() = Some(socket.clone());

    let state_clients = Arc::new(Mutex::new(HashMap::new()));
    
    // Wait for incoming UDP packets
    tokio::spawn(async move {
        let mut buf = [0; 1024];
        loop {
            if let Ok((len, addr)) = socket.recv_from(&mut buf).await {
                // Register client if not seen
                let mut clients = state_clients.lock().unwrap();
                if !clients.contains_key(&addr) {
                    clients.insert(addr, ());
                }

                let msg = String::from_utf8_lossy(&buf[..len]).to_string();
                
                // Emit event to Svelte layer
                let _ = app.emit("udp-msg-received", (addr.to_string(), msg));
            }
        }
    });

    Ok("Host UDP started on port 8080".to_string())
}

#[tauri::command]
pub async fn host_send_msg(state: State<'_, UdpState>, msg: String) -> Result<(), String> {
    let socket_opt = state.host_socket.lock().unwrap().clone();
    let clients = state.connected_clients.lock().unwrap().clone();
    
    if let Some(socket) = socket_opt {
        for (addr, _) in clients.iter() {
            let _ = socket.send_to(msg.as_bytes(), addr).await;
        }
    }
    Ok(())
}

// ----------------------------------------------------
// CLIENT LOGIC
// ----------------------------------------------------
#[tauri::command]
pub async fn connect_udp_client(app: tauri::AppHandle, state: State<'_, UdpState>, host_ip: String) -> Result<String, String> {
    // Bind to any ephemeral port
    let socket = UdpSocket::bind("0.0.0.0:0").await.map_err(|e| e.to_string())?;
    let socket = Arc::new(socket);
    
    let host_addr: SocketAddr = format!("{}:8080", host_ip).parse().map_err(|_| "Invalid IP")?;
    
    *state.client_socket.lock().unwrap() = Some(socket.clone());
    *state.client_remote_addr.lock().unwrap() = Some(host_addr);

    // Initial ping to punch hole / register with host
    let ping_msg = r#"{"type":"ping"}"#;
    socket.send_to(ping_msg.as_bytes(), &host_addr).await.map_err(|e| e.to_string())?;

    // Wait for incoming from Host
    tokio::spawn(async move {
        let mut buf = [0; 1024];
        loop {
            if let Ok((len, _addr)) = socket.recv_from(&mut buf).await {
                let msg = String::from_utf8_lossy(&buf[..len]).to_string();
                let _ = app.emit("udp-msg-received", ("host".to_string(), msg));
            }
        }
    });

    Ok("Connected to UDP Host".to_string())
}

#[tauri::command]
pub async fn client_send_msg(state: State<'_, UdpState>, msg: String) -> Result<(), String> {
    let socket_opt = state.client_socket.lock().unwrap().clone();
    let host_addr = state.client_remote_addr.lock().unwrap().clone();
    
    if let (Some(socket), Some(addr)) = (socket_opt, host_addr) {
        let _ = socket.send_to(msg.as_bytes(), &addr).await;
    }
    Ok(())
}

// ----------------------------------------------------
// UTILS
// ----------------------------------------------------
#[tauri::command]
pub fn get_local_ip() -> Result<String, String> {
    match local_ip() {
        Ok(ip) => Ok(ip.to_string()),
        Err(e) => Err(format!("Failed to get local IP: {}", e)),
    }
}
