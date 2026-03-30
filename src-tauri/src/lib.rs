mod config;
mod physics;
mod transport;
mod game;
mod udp_transport;

use transport::{host_online, join_online, reset_transport, get_room_id, cancel_online, WebRtcTransportState};

use crate::udp_transport::{start_discovery, stop_discovery};
use game::{start_game, stop_game, pause_game, resume_game, set_pointer, GameEngine};
use udp_transport::{start_udp_host, connect_udp_client, host_send_msg, client_send_msg, get_local_ips, UdpState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_admob::init())
        .manage(WebRtcTransportState::new())
        .manage(GameEngine::new())
        .manage(UdpState::new())
        .invoke_handler(tauri::generate_handler![
            host_online, join_online, reset_transport, get_room_id, cancel_online,
            start_game, stop_game, pause_game, resume_game, set_pointer,
            start_udp_host, connect_udp_client, host_send_msg, client_send_msg, get_local_ips,
            start_discovery, stop_discovery,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
