mod transport;
mod game;
use transport::{get_our_node_addr, start_accept_loop, connect_to_peer, host_online, join_online, cancel_online, TransportState};
use game::{start_game, stop_game, pause_game, resume_game, set_pointer, GameEngine};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(TransportState::new())
        .manage(GameEngine::new())
        .invoke_handler(tauri::generate_handler![
            get_our_node_addr, start_accept_loop, connect_to_peer, host_online, join_online, cancel_online,
            start_game, stop_game, pause_game, resume_game, set_pointer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
