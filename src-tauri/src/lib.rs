mod udp_server;
mod game;
mod relay;
use udp_server::{start_udp_host, connect_udp_client, host_send_msg, client_send_msg, get_local_ips, UdpState};
use game::{start_game, stop_game, pause_game, resume_game, set_pointer, GameEngine};
use relay::{connect_relay_host, connect_relay_join, disconnect_relay, RelayState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(UdpState::new())
        .manage(GameEngine::new())
        .manage(RelayState::new())
        .invoke_handler(tauri::generate_handler![
            start_udp_host, connect_udp_client, host_send_msg, client_send_msg, get_local_ips,
            start_game, stop_game, pause_game, resume_game, set_pointer,
            connect_relay_host, connect_relay_join, disconnect_relay,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
