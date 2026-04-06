use crate::protocol::BLE_UUID;
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

use windows::Devices::Bluetooth::GenericAttributeProfile::GattServiceProvider;

static BLE_PROVIDER: std::sync::OnceLock<std::sync::Mutex<Option<GattServiceProvider>>> =
    std::sync::OnceLock::new();

pub async fn broadcast_ble(ip_bytes: [u8; 4]) -> Result<(), std::io::Error> {
    use windows::Devices::Bluetooth::GenericAttributeProfile::{
        GattServiceProviderAdvertisingParameters,
        GattLocalCharacteristicParameters, GattCharacteristicProperties
    };
    use windows::core::GUID;
    use crate::protocol::{BLE_UUID, WIFI_INFO_UUID, WIFIDIRECT_INFO_UUID};
    use windows::Security::Cryptography::CryptographicBuffer;

    let uuid = GUID::from_u128(BLE_UUID);
    let provider_result = GattServiceProvider::CreateAsync(uuid);
    if let Ok(provider_op) = provider_result {
        if let Ok(result) = provider_op.await {
            if let Ok(provider) = result.ServiceProvider() {
                let char_uuid = GUID::from_u128(WIFI_INFO_UUID);
                let char_params = GattLocalCharacteristicParameters::new().unwrap();
                char_params.SetCharacteristicProperties(GattCharacteristicProperties::Read).unwrap();
                
                if let Ok(buffer) = CryptographicBuffer::CreateFromByteArray(&ip_bytes) {
                    char_params.SetStaticValue(&buffer).unwrap();
                }

                if let Ok(service) = provider.Service() {
                    let _ = service.CreateCharacteristicAsync(char_uuid, &char_params).unwrap().await;
                    
                    let wd_char_uuid = GUID::from_u128(WIFIDIRECT_INFO_UUID);
                    let wd_char_params = GattLocalCharacteristicParameters::new().unwrap();
                    wd_char_params.SetCharacteristicProperties(GattCharacteristicProperties::Read).unwrap();
                    let wd_info = vec![0u8, 0, 0, 0, 0, 0, 0];
                    if let Ok(buffer) = CryptographicBuffer::CreateFromByteArray(&wd_info) {
                        wd_char_params.SetStaticValue(&buffer).unwrap();
                    }
                    let _ = service.CreateCharacteristicAsync(wd_char_uuid, &wd_char_params).unwrap().await;
                }

                let adv_params = GattServiceProviderAdvertisingParameters::new().unwrap();
                adv_params.SetIsDiscoverable(true).unwrap();
                adv_params.SetIsConnectable(true).unwrap();
                provider.StartAdvertisingWithParameters(&adv_params).unwrap();
                
                eprintln!("[BLE] GATT advertising started successfully");

                // Keep the provider alive for the lifetime of the app
                let storage = BLE_PROVIDER.get_or_init(|| std::sync::Mutex::new(None));
                *storage.lock().unwrap() = Some(provider);
            }
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

    let target_uuid = Uuid::from_u128(BLE_UUID);

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
