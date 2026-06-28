use ed25519_dalek::{SigningKey, VerifyingKey};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::ble::BleState;
use crate::identity::{Store, load_store};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct IncomingOffer {
    pub id: String,
    pub file_name: String,
    pub file_size: u64,
    pub peer_name: String,
    pub peer_ip: String,
    pub is_trusted: bool,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Transfer {
    pub id: String,
    pub file_name: String,
    pub file_size: u64,
    pub bytes_done: u64,
    pub direction: String,
    pub peer_name: String,
    pub peer_ip: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_bps: Option<u64>,
}

pub enum TransferAction {
    Accept,
    Reject,
    TrustAndAccept,
}

pub struct QuickdropState {
    pub store: Arc<Mutex<Store>>,
    pub ble: BleState,
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
    pub store_path: String,
    pub download_dir: PathBuf,
    pub local_name: String,
    pub udp: crate::udp::UdpDiscovery,

    pub pending_offers: Arc<tokio::sync::Mutex<HashMap<String, oneshot::Sender<TransferAction>>>>,
    pub transfers: Arc<tokio::sync::Mutex<HashMap<String, Transfer>>>,
    pub cancelled_transfers: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,

    /// Abort handles for in-flight transfer tasks, keyed by transfer id. The
    /// per-chunk `cancelled_transfers` flag is best-effort (it's only observed
    /// between chunks, and a blocked socket write/read may never reach the
    /// check); aborting the task drops its socket, which stops the transfer
    /// immediately and unconditionally. We keep both so cancellation is robust.
    pub cancel_handles: Arc<std::sync::Mutex<HashMap<String, tokio::task::AbortHandle>>>,

    /// When true (default), an interrupted transfer is resumed from its partial
    /// `.unconfirmed` file; when false, a fresh file is received instead. Toggled
    /// at runtime from the settings UI.
    pub resume_transfers: Arc<AtomicBool>,
}

impl QuickdropState {
    pub async fn new(
        store_path: &str,
        identity_path: &str,
        download_dir: PathBuf,
        local_name: String,
        ble_state: BleState,
        resume_transfers: bool,
    ) -> Self {
        let store = load_store(store_path);

        let path = Path::new(identity_path);
        let signing_key = if path.exists() {
            let bytes = std::fs::read(path).unwrap_or_default();
            let key_bytes: [u8; 32] = bytes.try_into().expect("identity.key must be 32 bytes");
            SigningKey::from_bytes(&key_bytes)
        } else {
            use rand::rngs::OsRng;
            let signing_key = SigningKey::generate(&mut OsRng);
            let _ = std::fs::write(path, signing_key.to_bytes());
            signing_key
        };
        let verifying_key = signing_key.verifying_key();

        let _ = crate::session::init_wifi_direct_listener();

        let udp = crate::udp::UdpDiscovery::new();
        let mut my_id = String::new();
        my_id.push_str(&std::process::id().to_string());
        my_id.push_str("-");
        my_id.push_str(&std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos().to_string());
        let my_name = Arc::new(tokio::sync::Mutex::new(local_name.clone()));
        udp.start(my_id, my_name);

        Self {
            store,
            ble: ble_state,
            signing_key,
            verifying_key,
            store_path: store_path.to_string(),
            download_dir,
            local_name,
            udp,
            pending_offers: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            transfers: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            cancelled_transfers: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            cancel_handles: Arc::new(std::sync::Mutex::new(HashMap::new())),
            resume_transfers: Arc::new(AtomicBool::new(resume_transfers)),
        }
    }

    /// Update the resume preference at runtime (called when settings are saved).
    pub fn set_resume_transfers(&self, enabled: bool) {
        self.resume_transfers.store(enabled, Ordering::Relaxed);
    }

    pub async fn add_pending_offer(&self, id: String, tx: oneshot::Sender<TransferAction>) {
        self.pending_offers.lock().await.insert(id, tx);
    }

    pub async fn get_transfers(&self) -> Vec<Transfer> {
        let map = self.transfers.lock().await;
        map.values().cloned().collect()
    }

    pub async fn get_discovered_devices(&self) -> Vec<crate::ble::BleDevice> {
        let ble_devices = self.ble.get_devices().await;
        let udp_devices = self.udp.get_devices().await;

        let mut final_devices = Vec::new();

        for u in &udp_devices {
            final_devices.push(crate::ble::BleDevice {
                id: u.id.clone(),
                name: u.name.clone(),
            });
        }

        for b in ble_devices {
            if !udp_devices.iter().any(|u| u.name == b.name) {
                final_devices.push(b);
            }
        }

        final_devices
    }

    pub fn get_trusted_devices(&self) -> HashMap<String, crate::identity::TrustedDevice> {
        let store = self.store.lock().unwrap();
        store.devices.clone()
    }

    pub fn remove_trusted_device(&self, device_id: &str) {
        let mut store = self.store.lock().unwrap();
        store.devices.remove(device_id);
        if let Ok(json) = serde_json::to_string_pretty(&*store) {
            let _ = std::fs::write(&self.store_path, json);
        }
    }

    pub fn cancel_transfer(&self, id: &str) {
        self.cancelled_transfers.lock().unwrap().insert(id.to_string());
        // Also abort the running task (if any) so a transfer blocked in a socket
        // read/write — where the per-chunk flag check is never reached — stops
        // immediately instead of running to completion.
        if let Some(handle) = self.cancel_handles.lock().unwrap().get(id) {
            handle.abort();
        }
    }

    pub fn is_cancelled(&self, id: &str) -> bool {
        self.cancelled_transfers.lock().unwrap().contains(id)
    }

    pub fn clear_cancelled(&self, id: &str) {
        self.cancelled_transfers.lock().unwrap().remove(id);
    }

    /// Register the abort handle for an in-flight transfer task so a later
    /// `cancel_transfer(id)` can stop it immediately.
    pub fn register_cancel_handle(&self, id: &str, handle: tokio::task::AbortHandle) {
        self.cancel_handles.lock().unwrap().insert(id.to_string(), handle);
    }

    pub fn unregister_cancel_handle(&self, id: &str) {
        self.cancel_handles.lock().unwrap().remove(id);
    }

    pub async fn resolve_offer(&self, id: &str, action: TransferAction) -> Result<(), String> {
        if let Some(tx) = self.pending_offers.lock().await.remove(id) {
            let _ = tx.send(action);
            Ok(())
        } else {
            Err("Transfer not found or already resolved".to_string())
        }
    }

    pub fn start_receiving<F, Fut, G, H>(&self, on_offer: F, on_transfer_update: G, on_clipboard: H)
    where
        F: FnMut(u64, String, String, String, bool) -> Fut + Send + 'static + Clone,
        Fut: std::future::Future<Output = TransferAction> + Send,
        G: Fn(Transfer) -> bool + Send + Sync + 'static + Clone,
        H: Fn(String, String) + Send + Sync + 'static + Clone,
    {
        let store = Arc::clone(&self.store);
        let store_path = self.store_path.clone();
        let download_dir = self.download_dir.clone();
        let verifying_key = self.verifying_key;
        let local_name = self.local_name.clone();
        let signing_key = self.signing_key.clone();
        let resume_transfers = Arc::clone(&self.resume_transfers);

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let _ = crate::ble::WIFI_DIRECT_CONNECT_TX.set(tx);

        let store_clone2 = Arc::clone(&store);
        let store_path2 = store_path.clone();
        let download_dir2 = download_dir.clone();
        let verifying_key2 = verifying_key;
        let local_name2 = local_name.clone();
        let signing_key2 = signing_key.clone();
        let mut on_offer_clone2 = on_offer.clone();
        let on_transfer_update_clone2 = on_transfer_update.clone();
        let on_clipboard_clone2 = on_clipboard.clone();
        let resume_transfers2 = Arc::clone(&resume_transfers);

        tokio::spawn(async move {
            while let Some(go_ip) = rx.recv().await {
                // connect_to_android_hotspot already waited 5s for DHCP, 
                // but give a tiny extra moment for the network stack to settle
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                
                let port = crate::protocol::TCP_PORT;
                let addr = format!("{}:{}", go_ip, port);
                eprintln!("[WifiDirect-Rev] Attempting TCP connection to Android at {}...", addr);
                
                let mut connected = false;
                for attempt in 1..=15 {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        tokio::net::TcpStream::connect(&addr)
                    ).await {
                        Ok(Ok(mut stream)) => {
                            eprintln!("[WifiDirect-Rev] TCP connected to Android on attempt {}", attempt);
                            
                            let session_key_res = crate::handshake::run_client_handshake(
                                &mut stream,
                                &signing_key2,
                                "Android",
                                Arc::clone(&store_clone2),
                                &store_path2,
                                verifying_key2.as_bytes(),
                                &local_name2,
                            ).await;
                            
                            match session_key_res {
                                Ok((session_key, peer_public_key)) => {
                                    eprintln!("[WifiDirect-Rev] Handshake successful, entering session (receive mode)");
                                    let mut channel = crate::protocol::SecureChannel::new(session_key);
                                    let _ = crate::session::run_session(
                                        &mut stream,
                                        &mut channel,
                                        Arc::clone(&store_clone2),
                                        store_path2.clone(),
                                        &download_dir2,
                                        peer_public_key,
                                        "Android".to_string(),
                                        go_ip.clone(),
                                        Arc::clone(&resume_transfers2),
                                        &mut on_offer_clone2,
                                        &on_transfer_update_clone2,
                                        &on_clipboard_clone2,
                                    )
                                    .await;
                                    eprintln!("[WifiDirect-Rev] Session ended");
                                }
                                Err(e) => {
                                    eprintln!("[WifiDirect-Rev] Handshake failed: {}", e);
                                }
                            }
                            connected = true;
                            break;
                        }
                        Ok(Err(e)) => {
                            if attempt % 3 == 0 {
                                eprintln!("[WifiDirect-Rev] TCP connect attempt {}/15 failed: {}", attempt, e);
                            }
                        }
                        Err(_) => {
                            if attempt % 3 == 0 {
                                eprintln!("[WifiDirect-Rev] TCP connect attempt {}/15 timed out", attempt);
                            }
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                
                if !connected {
                    eprintln!("[WifiDirect-Rev] Failed to connect to Android TCP server at {} after 15 attempts", addr);
                }
            }
        });

        tokio::spawn(async move {
            let listener = match TcpListener::bind(format!("0.0.0.0:{}", crate::protocol::TCP_PORT)).await {
                Ok(l) => l,
                Err(_) => return,
            };
            loop {
                let (mut stream, addr) = match listener.accept().await {
                    Ok(res) => res,
                    Err(_) => continue,
                };

                let peer_ip = addr.ip().to_string();
                let store_clone = Arc::clone(&store);
                let store_path_clone = store_path.clone();
                let download_dir_clone = download_dir.clone();
                let local_name_clone = local_name.clone();
                let mut on_offer_clone = on_offer.clone();
                let on_transfer_update_clone = on_transfer_update.clone();
                let on_clipboard_clone = on_clipboard.clone();
                let resume_transfers_clone = Arc::clone(&resume_transfers);

                tokio::spawn(async move {
                    match crate::handshake::run_handshake(
                            &mut stream,
                            Arc::clone(&store_clone),
                            &store_path_clone,
                            verifying_key.as_bytes(),
                            &local_name_clone,
                        )
                        .await
                    {
                        Ok((session_key, peer_public_key, peer_name)) => {
                            let mut channel = crate::protocol::SecureChannel::new(session_key);
                            let _ = crate::session::run_session(
                                &mut stream,
                                &mut channel,
                                store_clone,
                                store_path_clone,
                                &download_dir_clone,
                                peer_public_key,
                                peer_name,
                                peer_ip,
                                resume_transfers_clone,
                                &mut on_offer_clone,
                                &on_transfer_update_clone,
                                &on_clipboard_clone,
                            )
                            .await;
                        }
                        Err(_) => {}
                    }
                });
            }
        });
    }

    /// Open a TCP connection to a peer, identified either by a literal IP or by a
    /// discovered device id (BLE, with a UDP fallback). Shared by file and
    /// clipboard sends.
    async fn connect_to_target(&self, target_id: &str) -> Result<tokio::net::TcpStream, String> {
        if let Ok(ip) = target_id.parse::<std::net::IpAddr>() {
            let addr = format!("{}:{}", ip, crate::protocol::TCP_PORT);
            tokio::net::TcpStream::connect(&addr)
                .await
                .map_err(|e| format!("Failed to connect to device {}: {}", addr, e))
        } else {
            // Try BLE first
            match self.ble.connect_to_device(target_id).await {
                Ok(s) => Ok(s),
                Err(ble_err) => {
                    eprintln!("[BLE] Connection failed: {}, trying UDP fallback...", ble_err);
                    // Fallback: look up the device name from BLE, find matching UDP device
                    let mut fallback_stream = None;
                    if let Some((_, device_name)) = self.ble.get_peripheral(target_id).await {
                        let udp_devices = self.udp.get_devices().await;
                        for ud in &udp_devices {
                            if ud.name == device_name || device_name.contains(&ud.name) || ud.name.contains(&device_name) {
                                let addr = format!("{}:{}", ud.ip, crate::protocol::TCP_PORT);
                                if let Ok(s) = tokio::net::TcpStream::connect(&addr).await {
                                    eprintln!("[UDP Fallback] Connected to {} via {}", device_name, addr);
                                    fallback_stream = Some(s);
                                    break;
                                }
                            }
                        }
                    }
                    fallback_stream.ok_or_else(|| format!("Failed to connect via BLE/WifiDirect: {}", ble_err))
                }
            }
        }
    }

    /// Run the client-side identity handshake and return the established secure
    /// channel for the connection.
    async fn open_channel(&self, stream: &mut tokio::net::TcpStream) -> Result<crate::protocol::SecureChannel, String> {
        let (session_key, _peer_public_key) = crate::handshake::run_client_handshake(
            stream,
            &self.signing_key,
            &self.local_name,
            Arc::clone(&self.store),
            &self.store_path,
            self.verifying_key.as_bytes(),
            &self.local_name,
        )
        .await
        .map_err(|e| format!("Handshake failed: {}", e))?;
        Ok(crate::protocol::SecureChannel::new(session_key))
    }

    pub async fn send_file<F>(&self, target_id: &str, file_path: &Path, on_progress: F) -> Result<String, String>
    where
        F: Fn(u64) -> bool + Send + Sync,
    {
        let mut stream = self.connect_to_target(target_id).await?;
        let mut channel = self.open_channel(&mut stream).await?;

        let id = format!("tx-{}", uuid::Uuid::new_v4());

        crate::transfer::offer_and_send_file(&mut stream, &mut channel, file_path, on_progress)
            .await
            .map_err(|e| e.to_string())?;

        Ok(id)
    }

    /// Push clipboard text to a peer: connect, handshake, send a single
    /// CLIPBOARD frame. The peer copies it into its system clipboard.
    pub async fn send_clipboard(&self, target_id: &str, text: &str) -> Result<(), String> {
        let mut stream = self.connect_to_target(target_id).await?;
        let mut channel = self.open_channel(&mut stream).await?;
        crate::protocol::secure_write(&mut stream, &mut channel, crate::protocol::CLIPBOARD, text.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
