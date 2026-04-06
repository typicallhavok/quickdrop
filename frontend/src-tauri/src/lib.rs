use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub local_name: String,
    pub download_dir: String,
    pub run_in_tray: bool,
    pub port: u16,
}

impl Settings {
    fn default_name() -> String {
        std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "My Device".to_string())
    }
}

impl Default for Settings {
    fn default() -> Self {
        let default_dir = dirs::download_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("./downloads"))
            .to_string_lossy()
            .to_string();

        Self {
            local_name: Self::default_name(),
            download_dir: default_dir,
            run_in_tray: false,
            port: 52341,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Transfer {
    pub id: String,
    pub file_name: String,
    pub file_size: u64,
    pub bytes_done: u64,
    pub direction: String, // "send" | "receive"
    pub peer_name: String,
    pub peer_ip: String,
    pub status: String, // "pending" | "active" | "done" | "error" | "rejected"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_bps: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub public_key: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: String,
}

// ── App state ─────────────────────────────────────────────────────────────────

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub settings_path: String,

    // Abstract the backend's state container here
    pub core: std::sync::Arc<quickdrop::state::QuickdropState>,
}

fn get_settings_path() -> String {
    if let Some(mut path) = dirs::config_dir() {
        path.push("quickdrop");
        let _ = std::fs::create_dir_all(&path);
        path.push("share_settings.json");
        return path.to_string_lossy().to_string();
    }
    "share_settings.json".to_string()
}

fn load_settings(path: &str) -> Settings {
    if let Ok(data) = std::fs::read_to_string(path) {
        if let Ok(mut s) = serde_json::from_str::<Settings>(&data) {
            if s.download_dir == "./downloads" {
                s.download_dir = dirs::download_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("./downloads"))
                    .to_string_lossy()
                    .to_string();
            }
            return s;
        }
    }
    Settings::default()
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(state: State<AppState>, settings: Settings) -> Result<(), String> {
    let mut s = state.settings.lock().unwrap();
    *s = settings.clone();
    serde_json::to_string_pretty(&settings)
        .map_err(|e| e.to_string())
        .and_then(|json| std::fs::write(&state.settings_path, json).map_err(|e| e.to_string()))
}

#[tauri::command]
fn get_local_ip() -> String {
    local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

// We make this `async` because we need to read a `tokio::sync::Mutex` under the hood
#[tauri::command]
async fn get_discovered_devices(
    state: State<'_, AppState>,
) -> Result<Vec<DiscoveredDevice>, String> {
    let devices = state.core.get_discovered_devices().await;

    Ok(devices
        .into_iter()
        .map(|d| DiscoveredDevice {
            id: d.id,
            name: d.name,
        })
        .collect())
}

#[tauri::command]
async fn send_file_cmd(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    target_id: String,
    file_path: String,
) -> Result<String, String> {
    let path = std::path::Path::new(&file_path);
    let file_meta = tokio::fs::metadata(path).await.map_err(|e| e.to_string())?;
    let file_name = path.file_name().unwrap().to_str().unwrap().to_string();
    let file_size = file_meta.len();

    // Emit initial active transfer
    let transfer_id_preview = format!("tx-{}", uuid::Uuid::new_v4());
    let initial = Transfer {
        id: transfer_id_preview.clone(),
        file_name: file_name.clone(),
        file_size,
        bytes_done: 0,
        direction: "send".to_string(),
        peer_name: target_id.clone(),
        peer_ip: target_id.clone(),
        status: "active".to_string(),
        speed_bps: None,
    };
    let _ = app.emit("transfer-progress", &initial);

    let progress_app = app.clone();
    let progress_id = transfer_id_preview.clone();
    let progress_fname = file_name.clone();
    let progress_peer = target_id.clone();
    let last_emit = std::sync::Mutex::new((std::time::Instant::now(), 0u64));

    let core_clone = state.core.clone();
    let result = state
        .core
        .send_file(&target_id, path, move |bytes_done| -> bool {
            let mut state = last_emit.lock().unwrap();
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(state.0).as_secs_f64();
            if elapsed >= 0.1 || bytes_done >= file_size {
                let speed = if elapsed > 0.0 {
                    Some(((bytes_done.saturating_sub(state.1)) as f64 / elapsed) as u64)
                } else { None };
                state.0 = now;
                state.1 = bytes_done;
                let t = Transfer {
                    id: progress_id.clone(),
                    file_name: progress_fname.clone(),
                    file_size,
                    bytes_done,
                    direction: "send".to_string(),
                    peer_name: progress_peer.clone(),
                    peer_ip: progress_peer.clone(),
                    status: "active".to_string(),
                    speed_bps: speed,
                };
                let _ = progress_app.emit("transfer-progress", &t);
            }
            !core_clone.is_cancelled(&progress_id)
        })
        .await;

    match result {
        Ok(id) => {
            let done = Transfer {
                id: id.clone(),
                file_name,
                file_size,
                bytes_done: file_size,
                direction: "send".to_string(),
                peer_name: target_id.clone(),
                peer_ip: target_id.clone(),
                status: "done".to_string(),
                speed_bps: None,
            };
            let _ = app.emit("transfer-complete", &done);
            Ok(id)
        }
        Err(e) => {
            let err_t = Transfer {
                id: transfer_id_preview,
                file_name: file_name.clone(),
                file_size,
                bytes_done: 0,
                direction: "send".to_string(),
                peer_name: target_id.clone(),
                peer_ip: target_id.clone(),
                status: "error".to_string(),
                speed_bps: None,
            };
            eprintln!("Transfer failed for file {}: {:?}", file_name, e);
            let _ = app.emit("transfer-error", &err_t);
            Err(e)
        }
    }
}

#[tauri::command]
async fn get_transfers(state: State<'_, AppState>) -> Result<Vec<Transfer>, String> {
    let transfers = state.core.get_transfers().await;
    Ok(transfers
        .into_iter()
        .map(|t| Transfer {
            id: t.id,
            file_name: t.file_name,
            file_size: t.file_size,
            bytes_done: t.bytes_done,
            direction: t.direction,
            peer_name: t.peer_name,
            peer_ip: t.peer_ip,
            status: t.status,
            speed_bps: t.speed_bps,
        })
        .collect())
}

#[tauri::command]
fn get_devices(state: State<AppState>) -> Vec<Device> {
    let trusted = state.core.get_trusted_devices();
    trusted
        .into_iter()
        .map(|(id, d)| Device {
            id,
            name: d.name,
            public_key: d.public_key,
        })
        .collect()
}

#[tauri::command]
fn remove_device(state: State<AppState>, device_id: String) {
    state.core.remove_trusted_device(&device_id);
}

#[tauri::command]
async fn accept_transfer(state: State<'_, AppState>, transfer_id: String) -> Result<(), String> {
    state
        .core
        .resolve_offer(&transfer_id, quickdrop::state::TransferAction::Accept)
        .await
}

#[tauri::command]
async fn reject_transfer(state: State<'_, AppState>, transfer_id: String) -> Result<(), String> {
    state
        .core
        .resolve_offer(&transfer_id, quickdrop::state::TransferAction::Reject)
        .await
}

#[tauri::command]
async fn trust_and_accept_transfer(
    state: State<'_, AppState>,
    transfer_id: String,
) -> Result<(), String> {
    state
        .core
        .resolve_offer(
            &transfer_id,
            quickdrop::state::TransferAction::TrustAndAccept,
        )
        .await
}

#[tauri::command]
fn cancel_transfer(state: State<AppState>, transfer_id: String) {
    state.core.cancel_transfer(&transfer_id);
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let settings_path = get_settings_path();
    let settings = load_settings(&settings_path);

    tauri::Builder::default()
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let sp_clone = settings_path.clone();

            let core = tauri::async_runtime::block_on(async {
                // Also broadcast our own IP
                if let Ok(ip) = local_ip_address::local_ip() {
                    if let std::net::IpAddr::V4(ipv4) = ip {
                        let bytes = ipv4.octets();
                        std::thread::spawn(move || {
                            let rt = tokio::runtime::Runtime::new().unwrap();
                            rt.block_on(async move {
                                let _ = quickdrop::ble::broadcast_ble(bytes).await;
                                // Keep the thread alive so the Windows COM apartment
                                // and GATT provider stay active for BLE advertising
                                loop {
                                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                                }
                            });
                        });
                    }
                }

                let ble_state = quickdrop::ble::init_ble_scan().await;

                let core = quickdrop::state::QuickdropState::new(
                    &sp_clone,
                    "identity.key",
                    std::path::PathBuf::from(&settings.download_dir),
                    settings.local_name.clone(),
                    ble_state,
                )
                .await;

                let core = std::sync::Arc::new(core);

                let core_clone = core.clone();
                let app_handle_progress = app_handle.clone();
                let core_clone2 = core.clone();
                core.start_receiving(
                    move |file_size, file_name, peer_name, peer_ip, is_trusted| {
                        let core = core_clone.clone();
                        let app_handle = app_handle.clone();
                        async move {
                            if is_trusted {
                                return quickdrop::state::TransferAction::Accept;
                            }
                            
                            let id = format!(
                                "offer-{}",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_nanos()
                            );
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            core.add_pending_offer(id.clone(), tx).await;

                            let offer = quickdrop::state::IncomingOffer {
                                id,
                                file_name,
                                file_size,
                                peer_name,
                                peer_ip,
                                is_trusted,
                            };

                            let _ = app_handle.emit("incoming-offer", offer);

                            rx.await.unwrap_or(quickdrop::state::TransferAction::Reject)
                        }
                    },
                    move |transfer: quickdrop::state::Transfer| -> bool {
                        let event_name = match transfer.status.as_str() {
                            "done" => "transfer-complete",
                            "error" => "transfer-error",
                            _ => "transfer-progress",
                        };
                        let id = transfer.id.clone();
                        let t = Transfer {
                            id: transfer.id,
                            file_name: transfer.file_name,
                            file_size: transfer.file_size,
                            bytes_done: transfer.bytes_done,
                            direction: transfer.direction,
                            peer_name: transfer.peer_name,
                            peer_ip: transfer.peer_ip,
                            status: transfer.status,
                            speed_bps: transfer.speed_bps,
                        };
                        let _ = app_handle_progress.emit(event_name, &t);
                        !core_clone2.is_cancelled(&id)
                    },
                );

                core
            });

            app.manage(AppState {
                settings: Mutex::new(settings),
                settings_path: settings_path.clone(),
                core,
            });

            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            get_local_ip,
            get_discovered_devices,
            send_file_cmd,
            get_transfers,
            get_devices,
            remove_device,
            accept_transfer,
            reject_transfer,
            trust_and_accept_transfer,
            cancel_transfer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
