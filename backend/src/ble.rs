use btleplug::{
    api::{Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter},
    platform::{Manager, Peripheral, PeripheralId},
};
use futures::stream::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use uuid::Uuid;
use windows::Devices::Bluetooth::Advertisement::BluetoothLEAdvertisementPublisher;
use windows::Devices::Bluetooth::GenericAttributeProfile::GattServiceProvider;

pub static WIFI_DIRECT_CONNECT_TX: std::sync::OnceLock<tokio::sync::mpsc::Sender<String>> = std::sync::OnceLock::new();

pub struct DeviceInfo {
    pub name: String,
    pub timestamp: Instant,
    pub peripheral: Peripheral,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct BleDevice {
    pub id: String,
    pub name: String,
}

#[derive(Clone)]
pub struct BleState {
    devices: Arc<Mutex<HashMap<PeripheralId, DeviceInfo>>>,
}

impl BleState {
    pub async fn get_devices(&self) -> Vec<BleDevice> {
        let map = self.devices.lock().await;
        map.iter()
            .map(|(id, info)| BleDevice {
                id: format!("{:?}", id),
                name: info.name.clone(),
            })
            .collect()
    }

    pub async fn get_peripheral(&self, target_id: &str) -> Option<(Peripheral, String)> {
        let map = self.devices.lock().await;
        for (id, info) in map.iter() {
            if format!("{:?}", id) == target_id {
                return Some((info.peripheral.clone(), info.name.clone()));
            }
        }
        None
    }

    pub async fn connect_to_device(
        &self,
        target_id: &str,
    ) -> Result<tokio::net::TcpStream, std::io::Error> {
        let (peripheral, name) = self.get_peripheral(target_id).await.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Peripheral not found")
        })?;
        crate::session::establish_tcp_connection(peripheral, name).await
    }
}

impl DeviceInfo {
    pub fn new(name: String, peripheral: Peripheral) -> Self {
        Self {
            name,
            timestamp: Instant::now(),
            peripheral,
        }
    }
}

static BLE_PROVIDER: std::sync::OnceLock<std::sync::Mutex<Option<GattServiceProvider>>> =
    std::sync::OnceLock::new();
static BLE_PUBLISHER: std::sync::OnceLock<
    std::sync::Mutex<Option<BluetoothLEAdvertisementPublisher>>,
> = std::sync::OnceLock::new();

pub async fn broadcast_ble(ip_bytes: [u8; 4]) -> Result<(), std::io::Error> {
    use windows::core::GUID;
    use windows::Devices::Bluetooth::Advertisement::BluetoothLEAdvertisementPublisher;
    use windows::Devices::Bluetooth::GenericAttributeProfile::{
        GattCharacteristicProperties, GattLocalCharacteristicParameters,
        GattServiceProviderAdvertisingParameters,
    };
    use windows::Security::Cryptography::CryptographicBuffer;

    // Explicitly define GUIDs to bypass u128 endianness mismatch with Java's UUID
    let uuid = GUID {
        data1: 0x00001d09,
        data2: 0x0000,
        data3: 0x1000,
        data4: [0x80, 0x00, 0x00, 0x80, 0x5f, 0x9b, 0x34, 0xfc],
    };

    // 1. Create GATT Service Provider
    let provider_result = GattServiceProvider::CreateAsync(uuid);
    match provider_result {
        Ok(provider_op) => {
            match provider_op.await {
                Ok(result) => {
                    match result.ServiceProvider() {
                        Ok(provider) => {
                            let char_uuid = GUID {
                                data1: 0x00001d0a,
                                data2: 0x0000,
                                data3: 0x1000,
                                data4: [0x80, 0x00, 0x00, 0x80, 0x5f, 0x9b, 0x34, 0xfc],
                            };
                            let char_params = GattLocalCharacteristicParameters::new().unwrap();
                            char_params
                                .SetCharacteristicProperties(GattCharacteristicProperties::Read)
                                .unwrap();

                            if let Ok(buffer) = CryptographicBuffer::CreateFromByteArray(&ip_bytes)
                            {
                                char_params.SetStaticValue(&buffer).unwrap();
                            }

                            if let Ok(service) = provider.Service() {
                                let _ = service
                                    .CreateCharacteristicAsync(char_uuid, &char_params)
                                    .unwrap()
                                    .await;

                                // 2b. Create Wi-Fi Direct Info Characteristic
                                let wd_char_uuid = GUID {
                                    data1: 0x00001d0b,
                                    data2: 0x0000,
                                    data3: 0x1000,
                                    data4: [0x80, 0x00, 0x00, 0x80, 0x5f, 0x9b, 0x34, 0xfc],
                                };
                                let wd_char_params =
                                    GattLocalCharacteristicParameters::new().unwrap();
                                wd_char_params
                                    .SetCharacteristicProperties(
                                        GattCharacteristicProperties::Read,
                                    )
                                    .unwrap();
                                let wd_info = vec![0u8, 0, 0, 0, 0, 0, 0];
                                if let Ok(buffer) =
                                    CryptographicBuffer::CreateFromByteArray(&wd_info)
                                {
                                    wd_char_params.SetStaticValue(&buffer).unwrap();
                                }
                                let _ = service
                                    .CreateCharacteristicAsync(wd_char_uuid, &wd_char_params)
                                    .unwrap()
                                    .await;
                                    
                                // 2c. Create Wi-Fi Direct Connect Characteristic (writable by Android)
                                let connect_char_uuid = GUID {
                                    data1: 0x00001d0c,
                                    data2: 0x0000,
                                    data3: 0x1000,
                                    data4: [0x80, 0x00, 0x00, 0x80, 0x5f, 0x9b, 0x34, 0xfc],
                                };
                                let connect_params = GattLocalCharacteristicParameters::new().unwrap();
                                connect_params
                                    .SetCharacteristicProperties(GattCharacteristicProperties::Write | GattCharacteristicProperties::WriteWithoutResponse)
                                    .unwrap();
                                let connect_char_result = service
                                    .CreateCharacteristicAsync(connect_char_uuid, &connect_params)
                                    .unwrap()
                                    .await;
                                if let Ok(result) = connect_char_result {
                                    if let Ok(characteristic) = result.Characteristic() {
                                        use windows::Devices::Bluetooth::GenericAttributeProfile::{
                                            GattLocalCharacteristic, GattWriteRequestedEventArgs,
                                        };
                                        let _ = characteristic.WriteRequested(
                                            &windows::Foundation::TypedEventHandler::<
                                                GattLocalCharacteristic,
                                                GattWriteRequestedEventArgs,
                                            >::new(move |_char, args| {
                                                eprintln!("[BLE] WriteRequested handler fired!");
                                                if let Some(args) = &*args {
                                                    let deferral = args.GetDeferral()?;
                                                    if let Ok(op) = args.GetRequestAsync() {
                                                        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                                                        let request_res = rt.block_on(async { op.await });
                                                        if let Ok(request) = request_res {
                                                            if let Ok(value) = request.Value() {
                                                                if let Ok(len) = value.Length() {
                                                                    let mut buf = vec![0u8; len as usize];
                                                                    if let Ok(reader) = windows::Storage::Streams::DataReader::FromBuffer(&value) {
                                                                        if reader.ReadBytes(&mut buf).is_ok() {
                                                                            eprintln!("[BLE] Got write data: {} bytes", buf.len());
                                                                            if buf.len() >= 6 {
                                                                                let go_ip = format!("{}.{}.{}.{}", buf[0], buf[1], buf[2], buf[3]);
                                                                                let mut offset = 4;
                                                                                let ssid_len = buf[offset] as usize; offset += 1;
                                                                                let ssid = String::from_utf8_lossy(&buf[offset..offset+ssid_len]).to_string(); offset += ssid_len;
                                                                                let pass_len = buf[offset] as usize; offset += 1;
                                                                                let pass = String::from_utf8_lossy(&buf[offset..offset+pass_len]).to_string();
                                                                                
                                                                                eprintln!("[BLE] Received Wi-Fi Direct creds: SSID={}, GO_IP={}", ssid, go_ip);
                                                                                
                                                                                // Spawn thread ONLY for netsh work (no COM objects)
                                                                                let go_ip_clone = go_ip.clone();
                                                                                std::thread::spawn(move || {
                                                                                    crate::session::connect_to_android_hotspot(&ssid, &pass, &go_ip);
                                                                                    
                                                                                    if let Some(tx) = crate::ble::WIFI_DIRECT_CONNECT_TX.get() {
                                                                                        let _ = tx.blocking_send(go_ip_clone);
                                                                                    }
                                                                                });
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            let _ = request.Respond();
                                                        }
                                                    }
                                                    deferral.Complete()?;
                                                }
                                                Ok(())
                                            }),
                                        );
                                    }
                                }
                            }

                            let adv_params =
                                GattServiceProviderAdvertisingParameters::new().unwrap();
                            adv_params.SetIsDiscoverable(true).unwrap();
                            adv_params.SetIsConnectable(true).unwrap();
                            provider
                                .StartAdvertisingWithParameters(&adv_params)
                                .unwrap();

                            eprintln!(
                                "[BLE] GATT advertising started successfully for custom UUID"
                            );

                            // Keep the provider alive for the lifetime of the app
                            let storage =
                                BLE_PROVIDER.get_or_init(|| std::sync::Mutex::new(None));
                            *storage.lock().unwrap() = Some(provider);
                        }
                        Err(e) => eprintln!("[BLE] Failed to get ServiceProvider: {:?}", e),
                    }
                }
                Err(e) => eprintln!("[BLE] Failed to create GattServiceProvider: {:?}", e),
            }
        }
        Err(e) => eprintln!("[BLE] Failed to call CreateAsync: {:?}", e),
    }

    // 2. Explicitly start a publisher to ensure Service UUID is in the advertisement packet
    if let Ok(publisher) = BluetoothLEAdvertisementPublisher::new() {
        if let Ok(adv) = publisher.Advertisement() {
            if let Ok(uuids) = adv.ServiceUuids() {
                let _ = uuids.Append(uuid);
            }
        }
        if publisher.Start().is_ok() {
            eprintln!("[BLE] Extra advertisement publisher started");
            let storage = BLE_PUBLISHER.get_or_init(|| std::sync::Mutex::new(None));
            *storage.lock().unwrap() = Some(publisher);
        }
    }

    Ok(())
}

pub async fn init_ble_scan() -> BleState {
    BleState {
        devices: scan_ble().await,
    }
}

pub async fn scan_ble() -> Arc<Mutex<HashMap<PeripheralId, DeviceInfo>>> {
    let devices: Arc<Mutex<HashMap<PeripheralId, DeviceInfo>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let update_devices_clone = Arc::clone(&devices);
    let remove_devices_clone = Arc::clone(&devices);

    tokio::spawn(async move { update_devices(update_devices_clone).await });
    tokio::spawn(async move { check_devices(remove_devices_clone).await });

    devices
}

async fn update_devices(devices: Arc<Mutex<HashMap<PeripheralId, DeviceInfo>>>) {
    let manager = Manager::new().await.unwrap();
    let adapters = manager.adapters().await.unwrap();
    let central = adapters
        .into_iter()
        .next()
        .expect("No Bluetooth adapter found");

    let mut events = central.events().await.unwrap();
    central.start_scan(ScanFilter::default()).await.unwrap();

    let target_uuid = Uuid::from_u128(crate::protocol::BLE_UUID);

    while let Some(event) = events.next().await {
        let id = match event {
            CentralEvent::DeviceDiscovered(id) => id,
            CentralEvent::ServicesAdvertisement { id, .. } => id,
            CentralEvent::DeviceUpdated(id) => id,
            _ => continue,
        };

        {
            let mut map = devices.lock().await;
            if let Some(info) = map.get_mut(&id) {
                info.timestamp = Instant::now();
                continue;
            }
        }

        let peripheral = central.peripheral(&id).await.unwrap();
        if let Some(properties) = peripheral.properties().await.unwrap() {
            let has_service = properties.services.contains(&target_uuid);

            if has_service {
                let name = properties
                    .local_name
                    .unwrap_or_else(|| "Nearby Device".to_string());

                devices
                    .lock()
                    .await
                    .insert(id, DeviceInfo::new(name, peripheral));
            }
        }
    }
}

async fn check_devices(devices: Arc<Mutex<HashMap<PeripheralId, DeviceInfo>>>) {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        devices
            .lock()
            .await
            .retain(|_, info| info.timestamp.elapsed().as_secs() <= 30);
    }
}