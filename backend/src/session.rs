use crate::{
    identity::{Store, add_trusted, is_trusted},
    protocol::{
        CLIPBOARD, FILE_OFFER, FILE_UPLOAD, OFFER_ACCEPT, OFFER_REJECT, TCP_PORT, SecureChannel, secure_read, secure_write,
    },
    transfer::receive_file,
};
use btleplug::{api::Peripheral as _, platform::Peripheral};
use std::{
    io::{self, Error},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::net::TcpStream;
use uuid::Uuid;
use windows::Devices::WiFiDirect::WiFiDirectDevice;

static ACTIVE_WIFI_DEVICES: std::sync::OnceLock<Mutex<Vec<WiFiDirectDevice>>> =
    std::sync::OnceLock::new();

pub fn keep_wifi_device_alive(device: WiFiDirectDevice) {
    let mut devices = ACTIVE_WIFI_DEVICES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap();
    if devices.len() > 10 {
        devices.remove(0);
    }
    devices.push(device);
}

#[derive(PartialEq)]
enum IncomingState {
    Idle,
    AwaitingUpload {
        expected_size: u64,
        expected_name: String,
        resume: bool,
        /// Byte offset negotiated in the accept and sent to the peer. The receive
        /// uses exactly this (not a fresh on-disk recompute) so the body byte
        /// count stays in lock-step with what the sender streams.
        resume_offset: u64,
    },
}

pub async fn run_session<F, Fut, G, H>(
    stream: &mut TcpStream,
    channel: &mut SecureChannel,
    store: Arc<Mutex<Store>>,
    store_path: String,
    download_dir: &Path,
    peer_public_key: [u8; 32],
    peer_name: String,
    peer_ip: String,
    resume_enabled: Arc<AtomicBool>,
    mut on_offer: F,
    on_transfer_update: G,
    on_clipboard: H,
) -> io::Result<()>
where
    F: FnMut(u64, String, String, String, bool) -> Fut,
    Fut: std::future::Future<Output = crate::state::TransferAction>,
    G: Fn(crate::state::Transfer) -> bool + Send + Sync,
    H: Fn(String, String) + Send + Sync,
{
    let mut state: IncomingState = IncomingState::Idle;
    loop {
        let (msg_type, payload) = match secure_read(stream, channel).await {
            Ok(res) => res,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        if msg_type == FILE_OFFER && payload.len() >= 10 && state == IncomingState::Idle {
            let file_size =
                u64::from_be_bytes(payload[..8].try_into().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid file size")
                })?);
            let name_len =
                u16::from_be_bytes(payload[8..10].try_into().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid file length")
                })?);

            if payload.len() != 10 + name_len as usize {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid payload",
                ));
            }

            let file_name = match std::str::from_utf8(&payload[10..10 + name_len as usize]) {
                Ok(string) => string.to_string(),
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            };

            let trusted = is_trusted(Arc::clone(&store), &peer_public_key);

            let action = on_offer(
                file_size,
                file_name.clone(),
                peer_name.clone(),
                peer_ip.clone(),
                trusted,
            )
            .await;

            // Read the resume preference once per offer so the accept (which
            // tells the sender the resume offset) and the receive (which decides
            // the on-disk file) agree for this transfer.
            let resume = resume_enabled.load(Ordering::Relaxed);

            match action {
                crate::state::TransferAction::Accept => {
                    let resume_offset =
                        send_offer_accept(stream, channel, download_dir, &file_name, file_size, resume).await?;
                    state = IncomingState::AwaitingUpload {
                        expected_size: file_size,
                        expected_name: file_name.clone(),
                        resume,
                        resume_offset,
                    };
                }
                crate::state::TransferAction::TrustAndAccept => {
                    add_trusted(&store, &peer_public_key, &peer_name, &store_path);
                    let resume_offset =
                        send_offer_accept(stream, channel, download_dir, &file_name, file_size, resume).await?;
                    state = IncomingState::AwaitingUpload {
                        expected_size: file_size,
                        expected_name: file_name.clone(),
                        resume,
                        resume_offset,
                    };
                }
                crate::state::TransferAction::Reject => {
                    state = IncomingState::Idle;
                    send_offer_reject(stream, channel).await?;
                }
            }
        } else if msg_type == FILE_UPLOAD {
            if let IncomingState::AwaitingUpload {
                expected_size,
                expected_name,
                resume,
                resume_offset,
            } = &state
            {
                let transfer_id = format!(
                    "rx-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                );
                let file_total = *expected_size;
                let fname = expected_name.clone();
                let pname = peer_name.clone();
                let pip = peer_ip.clone();

                let _ = on_transfer_update(crate::state::Transfer {
                    id: transfer_id.clone(),
                    file_name: fname.clone(),
                    file_size: file_total,
                    bytes_done: 0,
                    direction: "receive".to_string(),
                    peer_name: pname.clone(),
                    peer_ip: pip.clone(),
                    status: "active".to_string(),
                    speed_bps: None,
                });

                // (last_emit_time, last_emit_bytes, last_speed). We call
                // on_transfer_update on EVERY chunk so cancellation (which it
                // reports via its return value) is seen promptly; the timestamp
                // only throttles how often the speed is recomputed/the UI redrawn.
                let last_emit = std::sync::Mutex::new((std::time::Instant::now(), 0u64, None::<u64>));

                let result = receive_file(
                    stream,
                    channel,
                    download_dir,
                    *expected_size,
                    expected_name,
                    *resume,
                    *resume_offset,
                    |bytes_done| {
                        let mut state = last_emit.lock().unwrap();
                        let now = std::time::Instant::now();
                        let elapsed = now.duration_since(state.0).as_secs_f64();
                        if elapsed >= 0.1 || bytes_done >= file_total {
                            let speed = if elapsed > 0.0 {
                                Some(((bytes_done.saturating_sub(state.1)) as f64 / elapsed) as u64)
                            } else { state.2 };
                            state.0 = now;
                            state.1 = bytes_done;
                            state.2 = speed;
                        }
                        let speed = state.2;
                        drop(state);
                        on_transfer_update(crate::state::Transfer {
                            id: transfer_id.clone(),
                            file_name: fname.clone(),
                            file_size: file_total,
                            bytes_done,
                            direction: "receive".to_string(),
                            peer_name: pname.clone(),
                            peer_ip: pip.clone(),
                            status: "active".to_string(),
                            speed_bps: speed,
                        })
                    },
                )
                .await;

                match result {
                    Ok(()) => {
                        let _ = on_transfer_update(crate::state::Transfer {
                            id: transfer_id.clone(),
                            file_name: expected_name.clone(),
                            file_size: file_total,
                            bytes_done: file_total,
                            direction: "receive".to_string(),
                            peer_name: peer_name.clone(),
                            peer_ip: peer_ip.clone(),
                            status: "done".to_string(),
                            speed_bps: None,
                        });
                        state = IncomingState::Idle;
                    }
                    Err(e) => {
                        let status = if e.to_string().contains("Cancelled by user") {
                            "cancelled"
                        } else {
                            "error"
                        };
                        let _ = on_transfer_update(crate::state::Transfer {
                            id: transfer_id.clone(),
                            file_name: expected_name.clone(),
                            file_size: file_total,
                            bytes_done: 0,
                            direction: "receive".to_string(),
                            peer_name: peer_name.clone(),
                            peer_ip: peer_ip.clone(),
                            status: status.to_string(),
                            speed_bps: None,
                        });
                        return Err(e);
                    }
                }
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected upload",
                ));
            }
        } else if msg_type == CLIPBOARD {
            // Peer pushed clipboard text. Hand it off; the UI copies it into the
            // system clipboard and notifies the user who it came from.
            if let Ok(text) = String::from_utf8(payload) {
                on_clipboard(text, peer_name.clone());
            }
        } else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "protocol error"));
        }
    }
}

/// Decide where an accepted transfer should resume and bring the on-disk
/// `.unconfirmed` file into agreement with that decision. This is the single
/// source of truth for the resume offset — `receive_file` later recomputes the
/// offset from the file length, so the file state set here is what it sees.
///
/// Returns the byte offset the sender should start from (0 == fresh transfer).
async fn prepare_resume_offset(download_dir: &std::path::Path, file_name: &str, expected_size: u64) -> u64 {
    use crate::protocol::{RESUME_MIN_BYTES, RESUME_REWIND_BYTES};
    let unconfirmed_path = download_dir.join(format!("{}.unconfirmed", file_name));

    let existing = match tokio::fs::metadata(&unconfirmed_path).await {
        Ok(m) => m.len(),
        Err(_) => return 0,
    };

    // Partial is unusable (corrupt / already complete-but-unconfirmed) or too
    // small to be worth resuming: discard it and resend the whole file.
    if existing == 0 || existing >= expected_size || existing < RESUME_MIN_BYTES {
        let _ = tokio::fs::remove_file(&unconfirmed_path).await;
        return 0;
    }

    // Resume, but rewind a little and re-receive the tail so a torn write at the
    // end of the partial is overwritten instead of trusted as-is.
    let offset = existing.saturating_sub(RESUME_REWIND_BYTES);
    if let Ok(file) = tokio::fs::OpenOptions::new().write(true).open(&unconfirmed_path).await {
        if file.set_len(offset).await.is_err() {
            // Couldn't truncate — safest is to start over.
            let _ = tokio::fs::remove_file(&unconfirmed_path).await;
            return 0;
        }
    } else {
        return 0;
    }
    offset
}

/// Send OFFER_ACCEPT and return the resume offset that was negotiated with the
/// sender (0 == start from the beginning). The caller stores this and hands it to
/// `receive_file`, so the accept and the receive agree on exactly how many body
/// bytes will arrive.
async fn send_offer_accept(stream: &mut TcpStream, channel: &mut SecureChannel, download_dir: &std::path::Path, file_name: &str, expected_size: u64, resume: bool) -> io::Result<u64> {
    // When resume is disabled we never continue an existing partial: tell the
    // sender to start at 0 and leave any stale `.unconfirmed` untouched (the
    // receiver will write a fresh, uniquely-named file instead).
    let offset = if resume {
        prepare_resume_offset(download_dir, file_name, expected_size).await
    } else {
        0
    };
    let mut payload = Vec::new();
    if offset > 0 {
        payload.extend_from_slice(&offset.to_be_bytes());
    }
    secure_write(stream, channel, OFFER_ACCEPT, &payload).await?;
    Ok(offset)
}

async fn send_offer_reject(stream: &mut TcpStream, channel: &mut SecureChannel) -> io::Result<()> {
    secure_write(stream, channel, OFFER_REJECT, &[]).await?;
    Ok(())
}

pub async fn establish_tcp_connection(
    peripheral: Peripheral,
    _name: String,
) -> Result<TcpStream, Error> {
    let mut retry_count = 0;
    loop {
        if !peripheral
            .is_connected()
            .await
            .map_err(|e| Error::new(io::ErrorKind::Other, e))?
        {
            if let Err(e) = peripheral.connect().await {
                if retry_count >= 3 {
                    return Err(Error::new(io::ErrorKind::Other, e));
                }
                retry_count += 1;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                continue;
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        match peripheral.discover_services().await {
            Ok(_) => break,
            Err(e) if retry_count < 3 => {
                retry_count += 1;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                continue;
            }
            Err(e) => return Err(Error::new(io::ErrorKind::Other, e)),
        }
    }

    let chars = peripheral.characteristics();

    // First try standard TCP (if already on same network)
    if let Some(wifi_char) = chars.iter().find(|c| c.uuid == Uuid::from_u128(crate::protocol::WIFI_INFO_UUID)) {
        if let Ok(data) = peripheral.read(wifi_char).await {
            if data.len() >= 4 {
                let ip = std::net::Ipv4Addr::new(data[0], data[1], data[2], data[3]);
                let addr = format!("{}:{}", ip, TCP_PORT);
                if let Ok(Ok(stream)) = tokio::time::timeout(
                    std::time::Duration::from_millis(1500),
                    TcpStream::connect(&addr)
                ).await {
                    return Ok(stream);
                }
            }
        }
    }

    // Standard connection failed. Request Android to start Wi-Fi Direct Hotspot.
    use crate::protocol::{WIFIDIRECT_INFO_UUID, WIFIDIRECT_CONTROL_UUID};
    
    let control_char = chars.iter().find(|c| c.uuid == Uuid::from_u128(WIFIDIRECT_CONTROL_UUID))
        .ok_or_else(|| Error::new(io::ErrorKind::NotFound, "Missing Control Characteristic. Make sure Android app is updated."))?;
        
    peripheral.write(control_char, &[1], btleplug::api::WriteType::WithResponse).await
        .map_err(|e| Error::new(io::ErrorKind::Other, e))?;
        
    // Poll INFO characteristic until status == 1 (GROUP_CREATED)
    let info_char = chars.iter().find(|c| c.uuid == Uuid::from_u128(WIFIDIRECT_INFO_UUID))
        .ok_or_else(|| Error::new(io::ErrorKind::NotFound, "Missing Info Characteristic"))?;
        
    let mut ssid = String::new();
    let mut pass = String::new();
    let mut go_ip = "192.168.49.1".to_string();
    
    for _ in 0..30 { // Wait up to 15 seconds
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Ok(wd_data) = peripheral.read(info_char).await {
            if wd_data.len() >= 7 && wd_data[0] == 1 {
                let wd_ip = std::net::Ipv4Addr::new(wd_data[1], wd_data[2], wd_data[3], wd_data[4]);
                go_ip = wd_ip.to_string();
                
                let mut offset = 7;
                if wd_data.len() > offset {
                    let ssid_len = wd_data[offset] as usize;
                    offset += 1;
                    if wd_data.len() >= offset + ssid_len {
                        ssid = String::from_utf8_lossy(&wd_data[offset..offset+ssid_len]).to_string();
                        offset += ssid_len;
                    }
                    if wd_data.len() > offset {
                        let pass_len = wd_data[offset] as usize;
                        offset += 1;
                        if wd_data.len() >= offset + pass_len {
                            pass = String::from_utf8_lossy(&wd_data[offset..offset+pass_len]).to_string();
                        }
                    }
                }
                break;
            }
        }
    }
    
    if ssid.is_empty() {
        return Err(Error::new(io::ErrorKind::TimedOut, "Failed to get Wi-Fi Direct credentials from Android"));
    }
    
    eprintln!("[WifiDirect] Got credentials: SSID='{}', Password length={}, GO IP={}", ssid, pass.len(), go_ip);
    
    // Build hex SSID for the profile XML
    let ssid_hex: String = ssid.as_bytes().iter().map(|b| format!("{:02X}", b)).collect();
    
    // Connect to Wi-Fi Direct using netsh
    let profile_xml = format!(r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
    <name>{ssid}</name>
    <SSIDConfig>
        <SSID>
            <hex>{ssid_hex}</hex>
            <name>{ssid}</name>
        </SSID>
    </SSIDConfig>
    <connectionType>ESS</connectionType>
    <connectionMode>manual</connectionMode>
    <MSM>
        <security>
            <authEncryption>
                <authentication>WPA2PSK</authentication>
                <encryption>AES</encryption>
                <useOneX>false</useOneX>
            </authEncryption>
            <sharedKey>
                <keyType>passPhrase</keyType>
                <protected>false</protected>
                <keyMaterial>{pass}</keyMaterial>
            </sharedKey>
        </security>
    </MSM>
</WLANProfile>"#, ssid=ssid, ssid_hex=ssid_hex, pass=pass);

    let temp_dir = std::env::temp_dir();
    let profile_path = temp_dir.join(format!("{}.xml", ssid));
    std::fs::write(&profile_path, profile_xml).map_err(|e| Error::new(io::ErrorKind::Other, e))?;
    
    // Find the Wi-Fi interface name
    let iface_output = std::process::Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .output();
    let mut iface_name = String::from("Wi-Fi");
    if let Ok(output) = &iface_output {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Name") && trimmed.contains(":") {
                if let Some(name) = trimmed.split(':').nth(1) {
                    iface_name = name.trim().to_string();
                    break;
                }
            }
        }
    }
    eprintln!("[WifiDirect] Using Wi-Fi interface: '{}'", iface_name);
    
    let add_result = std::process::Command::new("netsh")
        .args(["wlan", "add", "profile", &format!("filename={}", profile_path.display())])
        .output();
    if let Ok(output) = &add_result {
        eprintln!("[WifiDirect] Profile add: {}", String::from_utf8_lossy(&output.stdout).trim());
        if !output.stderr.is_empty() {
            eprintln!("[WifiDirect] Profile add stderr: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
    }
        
    let connect_result = std::process::Command::new("netsh")
        .args(["wlan", "connect", &format!("name={}", ssid), &format!("interface={}", iface_name)])
        .output();
    if let Ok(output) = &connect_result {
        eprintln!("[WifiDirect] Connect: {}", String::from_utf8_lossy(&output.stdout).trim());
        if !output.stderr.is_empty() {
            eprintln!("[WifiDirect] Connect stderr: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
    }
    
    // Wait for DHCP lease before attempting TCP
    eprintln!("[WifiDirect] Waiting 5s for DHCP lease...");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    
    // Check actual Wi-Fi state and IP assignment
    if let Ok(output) = std::process::Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("SSID") || trimmed.starts_with("State") || trimmed.starts_with("Profile") {
                eprintln!("[WifiDirect] Interface: {}", trimmed);
            }
        }
    }
    
    if let Ok(output) = std::process::Command::new("ipconfig").output() {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut in_wifi_section = false;
        for line in text.lines() {
            if line.contains(&iface_name) || line.contains("Wireless") || line.contains("Wi-Fi") {
                in_wifi_section = true;
            }
            if in_wifi_section {
                let trimmed = line.trim();
                if trimmed.starts_with("IPv4") || trimmed.starts_with("Default Gateway") || trimmed.starts_with("Subnet") {
                    eprintln!("[WifiDirect] IP: {}", trimmed);
                }
                if trimmed.is_empty() && in_wifi_section {
                    in_wifi_section = false;
                }
            }
        }
    }
        
    // Wait for connection to establish and TCP to become available
    for i in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        if i % 5 == 0 {
            eprintln!("[WifiDirect] Waiting for TCP connection to {}:{} (attempt {}/30)...", go_ip, TCP_PORT, i+1);
        }
        if let Ok(Ok(stream)) = tokio::time::timeout(
            std::time::Duration::from_millis(1000),
            TcpStream::connect(format!("{}:{}", go_ip, TCP_PORT))
        ).await {
            eprintln!("[WifiDirect] TCP connection established!");
            return Ok(stream);
        }
    }
    
    Err(Error::new(io::ErrorKind::NotConnected, "Could not establish TCP connection to Wi-Fi Direct hotspot"))
}

pub fn init_wifi_direct_listener() -> Result<(), Error> {
    // We no longer need the Windows WiFi Direct Listener since we use netsh.
    Ok(())
}

/// Called from BLE write handler when Android requests the PC to connect to its Wi-Fi Direct hotspot.
/// Runs synchronously in a spawned thread — mirrors the same netsh logic from establish_tcp_connection.
pub fn connect_to_android_hotspot(ssid: &str, pass: &str, go_ip: &str) {
    eprintln!("[WifiDirect-Rev] Starting hotspot connection: SSID='{}', GO_IP={}", ssid, go_ip);
    
    let ssid_hex: String = ssid.as_bytes().iter().map(|b| format!("{:02X}", b)).collect();
    
    let profile_xml = format!(r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
    <name>{ssid}</name>
    <SSIDConfig>
        <SSID>
            <hex>{ssid_hex}</hex>
            <name>{ssid}</name>
        </SSID>
    </SSIDConfig>
    <connectionType>ESS</connectionType>
    <connectionMode>manual</connectionMode>
    <MSM>
        <security>
            <authEncryption>
                <authentication>WPA2PSK</authentication>
                <encryption>AES</encryption>
                <useOneX>false</useOneX>
            </authEncryption>
            <sharedKey>
                <keyType>passPhrase</keyType>
                <protected>false</protected>
                <keyMaterial>{pass}</keyMaterial>
            </sharedKey>
        </security>
    </MSM>
</WLANProfile>"#, ssid=ssid, ssid_hex=ssid_hex, pass=pass);

    let temp_dir = std::env::temp_dir();
    let profile_path = temp_dir.join(format!("{}.xml", ssid));
    if let Err(e) = std::fs::write(&profile_path, profile_xml) {
        eprintln!("[WifiDirect-Rev] Failed to write profile XML: {}", e);
        return;
    }
    
    // Find the Wi-Fi interface name (same as establish_tcp_connection)
    let mut iface_name = String::from("Wi-Fi");
    if let Ok(output) = std::process::Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Name") && trimmed.contains(":") {
                if let Some(name) = trimmed.split(':').nth(1) {
                    iface_name = name.trim().to_string();
                    break;
                }
            }
        }
    }
    eprintln!("[WifiDirect-Rev] Using Wi-Fi interface: '{}'", iface_name);
    
    // Add profile
    let add_result = std::process::Command::new("netsh")
        .args(["wlan", "add", "profile", &format!("filename={}", profile_path.display())])
        .output();
    if let Ok(output) = &add_result {
        eprintln!("[WifiDirect-Rev] Profile add: {}", String::from_utf8_lossy(&output.stdout).trim());
        if !output.stderr.is_empty() {
            eprintln!("[WifiDirect-Rev] Profile add stderr: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
    }
    
    // Connect with interface name (critical — without this netsh may pick wrong adapter)
    let connect_result = std::process::Command::new("netsh")
        .args(["wlan", "connect", &format!("name={}", ssid), &format!("interface={}", iface_name)])
        .output();
    if let Ok(output) = &connect_result {
        eprintln!("[WifiDirect-Rev] Connect: {}", String::from_utf8_lossy(&output.stdout).trim());
        if !output.stderr.is_empty() {
            eprintln!("[WifiDirect-Rev] Connect stderr: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
    }
    
    // Wait for DHCP lease
    eprintln!("[WifiDirect-Rev] Waiting 5s for DHCP lease...");
    std::thread::sleep(std::time::Duration::from_secs(5));
    
    // Verify connection state
    if let Ok(output) = std::process::Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("SSID") || trimmed.starts_with("State") || trimmed.starts_with("Profile") {
                eprintln!("[WifiDirect-Rev] Interface: {}", trimmed);
            }
        }
    }
    
    if let Ok(output) = std::process::Command::new("ipconfig").output() {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut in_wifi_section = false;
        for line in text.lines() {
            if line.contains(&iface_name) || line.contains("Wireless") || line.contains("Wi-Fi") {
                in_wifi_section = true;
            }
            if in_wifi_section {
                let trimmed = line.trim();
                if trimmed.starts_with("IPv4") || trimmed.starts_with("Default Gateway") || trimmed.starts_with("Subnet") {
                    eprintln!("[WifiDirect-Rev] IP: {}", trimmed);
                }
                if trimmed.is_empty() && in_wifi_section {
                    in_wifi_section = false;
                }
            }
        }
    }
    
    eprintln!("[WifiDirect-Rev] Hotspot connection complete. GO_IP={}", go_ip);
}
