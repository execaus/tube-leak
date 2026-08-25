mod commands;
mod sidecar;
mod types;

use commands::check_sidecar;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![check_sidecar])
        .run(tauri::generate_context!())
        .unwrap_or_else(|err| {
            eprintln!("error while running tauri application: {err}");
            std::process::exit(1);
        });
}
