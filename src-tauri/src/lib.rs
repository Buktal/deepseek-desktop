//! DeepSeek Desktop Tauri backend library.
//!
//! Empty shell for now — commands will land here.

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
