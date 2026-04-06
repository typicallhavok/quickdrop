use crate::{
    identity::{Store, add_trusted, is_trusted},
    protocol::{
        FILE_OFFER, FILE_UPLOAD, OFFER_ACCEPT, OFFER_REJECT, TCP_PORT, SecureChannel, WIFI_INFO_UUID,
        secure_read, secure_write,
    },
    transfer::receive_file,
};
use btleplug::{api::Peripheral as _, platform::Peripheral};
use std::{
    io::{self, Error},
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::net::TcpStream;
use uuid::Uuid;
use windows::{
    Devices::{
        Enumeration::DeviceInformation,
        WiFiDirect::{
            WiFiDirectConnectionListener, WiFiDirectConnectionRequestedEventArgs, WiFiDirectDevice,
        },
    },
    Foundation::TypedEventHandler,
};

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
    },
}

pub async fn run_session<F, Fut, G>(
    stream: &mut TcpStream,
    channel: &mut SecureChannel,
    store: Arc<Mutex<Store>>,
    store_path: String,
    download_dir: &Path,
    peer_public_key: [u8; 32],
    peer_name: String,
    peer_ip: String,
    mut on_offer: F,
    on_transfer_update: G,
) -> io::Result<()>
where
    F: FnMut(u64, String, String, String, bool) -> Fut,
    Fut: std::future::Future<Output = crate::state::TransferAction>,
    G: Fn(crate::state::Transfer) -> bool + Send + Sync,
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

            match action {
                crate::state::TransferAction::Accept => {
                    state = IncomingState::AwaitingUpload {
                        expected_size: file_size,
                        expected_name: file_name.clone(),
                    };
                    send_offer_accept(stream, channel, download_dir, &file_name).await?;
                }
                crate::state::TransferAction::TrustAndAccept => {
                    add_trusted(&store, &peer_public_key, &peer_name, &store_path);
                    state = IncomingState::AwaitingUpload {
                        expected_size: file_size,
                        expected_name: file_name.clone(),
                    };
                    send_offer_accept(stream, channel, download_dir, &file_name).await?;
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

                let last_emit = std::sync::Mutex::new((std::time::Instant::now(), 0u64));

                let result = receive_file(
                    stream,
                    channel,
                    download_dir,
                    *expected_size,
                    expected_name,
                    |bytes_done| {
                        let mut state = last_emit.lock().unwrap();
                        let now = std::time::Instant::now();
                        let elapsed = now.duration_since(state.0).as_secs_f64();
                        if elapsed >= 0.1 || bytes_done >= file_total {
                            let speed = if elapsed > 0.0 {
                                Some(((bytes_done.saturating_sub(state.1)) as f64 / elapsed) as u64)
                            } else { None };
                            state.0 = now;
                            state.1 = bytes_done;
                            return on_transfer_update(crate::state::Transfer {
                                id: transfer_id.clone(),
                                file_name: fname.clone(),
                                file_size: file_total,
                                bytes_done,
                                direction: "receive".to_string(),
                                peer_name: pname.clone(),
                                peer_ip: pip.clone(),
                                status: "active".to_string(),
                                speed_bps: speed,
                            });
                        }
                        true
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
                        let _ = on_transfer_update(crate::state::Transfer {
                            id: transfer_id.clone(),
                            file_name: expected_name.clone(),
                            file_size: file_total,
                            bytes_done: 0,
                            direction: "receive".to_string(),
                            peer_name: peer_name.clone(),
                            peer_ip: peer_ip.clone(),
                            status: "error".to_string(),
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
        } else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "protocol error"));
        }
    }
}

async fn send_offer_accept(stream: &mut TcpStream, channel: &mut SecureChannel, download_dir: &std::path::Path, file_name: &str) -> io::Result<()> {
    let unconfirmed_path = download_dir.join(format!("{}.unconfirmed", file_name));
    let mut payload = Vec::new();
    if unconfirmed_path.exists() {
        if let Ok(metadata) = tokio::fs::metadata(&unconfirmed_path).await {
            let offset = metadata.len();
            payload.extend_from_slice(&offset.to_be_bytes());
        }
    }
    secure_write(stream, channel, OFFER_ACCEPT, &payload).await?;
    Ok(())
}

async fn send_offer_reject(stream: &mut TcpStream, channel: &mut SecureChannel) -> io::Result<()> {
    secure_write(stream, channel, OFFER_REJECT, &[]).await?;
    Ok(())
}

pub async fn establish_tcp_connection(
    peripheral: Peripheral,
    name: String,
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

    let wifi_char = chars
        .iter()
        .find(|c| c.uuid == Uuid::from_u128(WIFI_INFO_UUID))
        .ok_or_else(|| Error::new(io::ErrorKind::NotFound, "MissingCharacteristic"))?;

    let data = peripheral
        .read(wifi_char)
        .await
        .map_err(|e| Error::new(io::ErrorKind::Other, e))?;

    let ip = std::net::Ipv4Addr::new(data[0], data[1], data[2], data[3]);
    let addr = format!("{}:{}", ip, TCP_PORT);

    if let Ok(Ok(stream)) = tokio::time::timeout(
        std::time::Duration::from_millis(1500),
        TcpStream::connect(&addr)
    ).await {
        return Ok(stream);
    }

    // Try WiFi Direct fallback if direct connection failed
    let mut go_ip = "192.168.49.1".to_string(); // Default Android GO IP
    use crate::protocol::WIFIDIRECT_INFO_UUID;
    if let Some(wd_char) = chars.iter().find(|c| c.uuid == Uuid::from_u128(WIFIDIRECT_INFO_UUID)) {
        if let Ok(wd_data) = peripheral.read(wd_char).await {
            if wd_data.len() >= 7 && wd_data[0] == 1 {
                let wd_ip = std::net::Ipv4Addr::new(wd_data[1], wd_data[2], wd_data[3], wd_data[4]);
                go_ip = wd_ip.to_string();
            }
        }
    }

    // Fast path: try connecting to GO IP directly in case we are already connected to its P2P group
    if let Ok(Ok(stream)) = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        TcpStream::connect(format!("{}:{}", go_ip, TCP_PORT))
    ).await {
        return Ok(stream);
    }

    // Enumerate WiFi Direct devices with active scanning
    let aqs = WiFiDirectDevice::GetDeviceSelector().unwrap();
    let target_ids = Arc::new(Mutex::new(Vec::new()));

    let target_ids_clone = target_ids.clone();
    let name_clone = name.clone();
    let aqs_string = aqs.to_string();

    let _ = tokio::task::spawn_blocking(move || {
        let handler = TypedEventHandler::<
            windows::Devices::Enumeration::DeviceWatcher,
            DeviceInformation,
        >::new({
            let t_ids = target_ids_clone.clone();
            let n_clone = name_clone.clone();
            move |_sender, args| {
                if let Some(device) = args.as_ref() {
                    if let Ok(dev_name) = device.Name() {
                        let dev_name_str = dev_name.to_string();
                        if dev_name_str.contains(&n_clone) || n_clone.contains(&dev_name_str) {
                            if let Ok(id) = device.Id() {
                                t_ids.lock().unwrap().push(id);
                            }
                        }
                    }
                }
                Ok(())
            }
        });

        let h_aqs = windows::core::HSTRING::from(&aqs_string);
        if let Ok(watcher) = windows::Devices::Enumeration::DeviceInformation::CreateWatcherAqsFilter(&h_aqs) {
            if let Ok(token) = watcher.Added(&handler) {
                let _ = watcher.Start();
                for _ in 0..30 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if !target_ids_clone.lock().unwrap().is_empty() {
                        break;
                    }
                }
                let _ = watcher.Stop();
                let _ = watcher.RemoveAdded(token);
            }
        }
    }).await;

    // Also check cached devices
    if let Ok(devices) = DeviceInformation::FindAllAsyncAqsFilter(&aqs).unwrap().await {
        for device in &devices {
            if let Ok(dev_name) = device.Name() {
                let dev_name_str = dev_name.to_string();
                if dev_name_str.contains(&name) || name.contains(&dev_name_str) {
                    target_ids.lock().unwrap().push(device.Id().unwrap());
                }
            }
        }
    }

    let ids = target_ids.lock().unwrap().clone();
    for id in ids {
            let mut ips_to_try = Vec::new();
            {
                if let Ok(op) = WiFiDirectDevice::FromIdAsync(&id) {
                    if let Ok(wifi_device) = op.await {
                        if let Ok(endpoints) = wifi_device.GetConnectionEndpointPairs() {
                            if endpoints.Size().unwrap_or(0) > 0 {
                                if let Ok(pair) = endpoints.GetAt(0) {
                                    if let Ok(host_name) = pair.RemoteHostName() {
                                        if let Ok(new_ip) = host_name.ToString() {
                                            ips_to_try.push(new_ip.to_string());
                                        }
                                    }
                                }
                            }
                        }
                        keep_wifi_device_alive(wifi_device);
                    }
                }
            }

            for new_ip in ips_to_try {
                let addr = format!("{}:{}", new_ip, TCP_PORT);
                if let Ok(stream) = TcpStream::connect(&addr).await {
                    return Ok(stream);
                }
            }
        }

    return Err(Error::new(
        io::ErrorKind::NotConnected,
        "could not establish connection",
    ));
}

pub fn init_wifi_direct_listener() -> Result<WiFiDirectConnectionListener, Error> {
    let listener = WiFiDirectConnectionListener::new()
        .map_err(|e| Error::new(io::ErrorKind::Other, e))?;
    listener
        .ConnectionRequested(&TypedEventHandler::<
            WiFiDirectConnectionListener,
            WiFiDirectConnectionRequestedEventArgs,
        >::new(|_sender, args| {
            if let Some(args) = args.as_ref() {
                if let Ok(request) = args.GetConnectionRequest() {
                    let _ = request
                        .DeviceInformation()
                        .and_then(|device_info| {
                            device_info.Id().and_then(|incoming_id| {
                                std::thread::spawn(move || {
                                    if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                                        rt.block_on(async move {
                                            if let Ok(op) = WiFiDirectDevice::FromIdAsync(&incoming_id) {
                                                if let Ok(device) = op.await {
                                                    keep_wifi_device_alive(device);
                                                }
                                            }
                                        });
                                    }
                                });
                                Ok(())
                            })
                        });
                }
            }
            Ok(())
        }))
        .map_err(|e| Error::new(io::ErrorKind::Other, e))?;
    Ok(listener)
}
