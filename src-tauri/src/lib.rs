mod udp_server;
use std::sync::Mutex;
use std::collections::HashMap;
use udp_server::{start_udp_host, connect_udp_client, host_send_msg, client_send_msg, get_local_ip, UdpState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(UdpState {
            is_running: Mutex::new(false),
            host_socket: Mutex::new(None),
            client_socket: Mutex::new(None),
            client_remote_addr: Mutex::new(None),
            connected_clients: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            start_udp_host,
            host_send_msg,
            connect_udp_client,
            client_send_msg,
            get_local_ip
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
