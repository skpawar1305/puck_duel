fn main() {
    // Load src-tauri/.env into compile-time env vars so transport.rs can use env!()
    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    if env_path.exists() {
        let content = std::fs::read_to_string(&env_path)
            .expect("Failed to read src-tauri/.env");
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                println!("cargo:rustc-env={}={}", k.trim(), v.trim());
            }
        }
    }
    println!("cargo:rerun-if-changed=.env");

    tauri_build::build()
}
