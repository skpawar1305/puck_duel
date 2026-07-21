mod config;
mod physics;
mod game;

use game::{GameEngine, ServerState, create_room, join_room, create_solo, wait_for_opponent, cancel_wait_for_opponent, start_game, stop_game, pause_game, resume_game, set_pointer};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_admob::init())
        .manage(ServerState::new())
        .manage(GameEngine::new())
        .invoke_handler(tauri::generate_handler![
            create_room, join_room, create_solo, wait_for_opponent, cancel_wait_for_opponent, start_game, stop_game, pause_game, resume_game, set_pointer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
