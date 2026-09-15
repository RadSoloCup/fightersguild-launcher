mod installer;

#[cfg(debug_assertions)]
use tauri::Manager;

#[tauri::command]
async fn check_status() -> Result<installer::StatusReport, String> {
    installer::check_status().await
}

#[tauri::command]
async fn run_install(window: tauri::Window) -> Result<(), String> {
    installer::run(window).await
}

#[tauri::command]
fn launch_minecraft() -> Result<(), String> {
    installer::launch_official_launcher()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![check_status, run_install, launch_minecraft])
        .setup(|_app| {
            #[cfg(debug_assertions)]
            {
                let window = _app.get_webview_window("main").unwrap();
                window.open_devtools();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
