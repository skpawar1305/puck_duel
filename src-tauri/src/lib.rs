mod udp_server;
mod game;
use udp_server::{start_udp_host, connect_udp_client, host_send_msg, client_send_msg, get_local_ips, UdpState};
use game::{start_game, stop_game, set_pointer, GameEngine};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(UdpState::new())
        .manage(GameEngine::new())
        .invoke_handler(tauri::generate_handler![
            start_udp_host, connect_udp_client, host_send_msg, client_send_msg, get_local_ips,
            start_game, stop_game, set_pointer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
