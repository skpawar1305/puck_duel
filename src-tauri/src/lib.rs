mod transport;
mod game;
mod udp_transport;

use transport::{get_our_node_addr, start_addr_server, fetch_peer_addr, start_accept_loop, connect_to_peer, accept_from_peer, host_online, join_online, cancel_online, reset_transport, set_hosted_room_id, TransportState};

use crate::udp_transport::{start_discovery, stop_discovery};
use game::{start_game, stop_game, pause_game, resume_game, set_pointer, GameEngine};
use udp_transport::{start_udp_host, connect_udp_client, host_send_msg, client_send_msg, get_local_ips, UdpState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_admob::init())
        .manage(TransportState::new())
        .manage(GameEngine::new())
        .manage(UdpState::new())
        .invoke_handler(tauri::generate_handler![
            get_our_node_addr, start_addr_server, fetch_peer_addr, start_accept_loop, connect_to_peer, accept_from_peer, host_online, join_online, cancel_online, reset_transport, set_hosted_room_id,
            start_game, stop_game, pause_game, resume_game, set_pointer,
            start_udp_host, connect_udp_client, host_send_msg, client_send_msg, get_local_ips,
            start_discovery, stop_discovery,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
