use base64::{Engine as _, engine::general_purpose};
use rand::Rng;
use rand::rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use hex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    New,
    HelloReceived,
    ChallengeSent,
    Authenticated,
    Rejected,
}

#[derive(Serialize, Deserialize)]
pub struct TrustedDevice {
    pub name: String,
    pub public_key: String,
}

#[derive(Serialize, Deserialize)]
pub struct Store {
    pub devices: HashMap<String, TrustedDevice>,
}

pub const NONCE_SIZE: usize = 32;

pub fn generate_nonce() -> [u8; NONCE_SIZE] {
    let mut nonce = [0u8; NONCE_SIZE];
    rng().fill(&mut nonce);
    nonce
}

pub fn device_id(public_key: &[u8]) -> String {
    let hash = Sha256::digest(public_key);
    hex::encode(hash)
}

pub fn load_store(path: &str) -> Store {
    let empty = Store {
        devices: HashMap::new(),
    };
    if let Ok(mut file) = File::open(path) {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            if let Ok(store) = serde_json::from_str::<Store>(&contents) {
                return store;
            }
        }
    }
    empty
}

pub fn is_trusted(store: &Store, public_key: &[u8]) -> bool {
    store.devices.contains_key(&device_id(public_key))
}

pub fn add_trusted(store: &mut Store, public_key: &[u8], name: &str, path: &str) {
    if !is_trusted(store, public_key) {
        let device = TrustedDevice {
            name: name.to_string(),
            public_key: general_purpose::STANDARD.encode(public_key),
        };
        store.devices.insert(device_id(public_key), device);
        if let Ok(json) = serde_json::to_string_pretty(&store) {
            let _ = std::fs::write(path, json);
        }
    }
}
