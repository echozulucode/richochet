//! Richochet's Tauri shell.
//!
//! Deliberately thin: it owns the clipboard, the window and the command surface, and delegates
//! every conversion decision to [`mdcore`].

pub mod clipboard;
pub mod commands;

/// Build and run the application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::convert,
            commands::outline,
            commands::read_clipboard,
            commands::write_clipboard,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Richochet");
}
