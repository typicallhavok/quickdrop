use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use hkdf::Hkdf;
use sha2::Sha256;
use std::io;

use crate::identity::NONCE_SIZE;

pub fn derive_session_key(server_nonce: &[u8; NONCE_SIZE], client_nonce: &[u8; NONCE_SIZE], our_pub: &[u8; 32], peer_pub: &[u8; 32]) -> [u8; 32] {
    let (pk1, pk2) = if our_pub <= peer_pub {
        (our_pub, peer_pub)
    } else {
        (peer_pub, our_pub)
    };

    let mut ikm = Vec::with_capacity(32 + 32 + 32 + 32);
    ikm.extend_from_slice(server_nonce);
    ikm.extend_from_slice(client_nonce);
    ikm.extend_from_slice(pk1);
    ikm.extend_from_slice(pk2);

    let hk = Hkdf::<Sha256>::new(None, &ikm);
    let mut key = [0u8; 32];
    hk.expand(b"fastshare-session", &mut key)
        .expect("HKDF expand should never fail");
    key
}

pub fn encrypt(key: &[u8; 32], counter:u64, plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).unwrap();
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes[4..].copy_from_slice(&counter.to_be_bytes());
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .expect("encryption failed");
    let mut out = Vec::with_capacity(ciphertext.len());

    out.extend_from_slice(&ciphertext);
    out
}

pub fn decrypt(key: &[u8; 32], counter:u64, data: &[u8]) -> io::Result<Vec<u8>> {

    let cipher = Aes256Gcm::new_from_slice(key).unwrap();
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes[4..].copy_from_slice(&counter.to_be_bytes());
    let ciphertext = data;
    let nonce = Nonce::from_slice(&nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "decryption failed"))
}
