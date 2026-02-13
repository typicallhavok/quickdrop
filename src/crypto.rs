use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use hkdf::Hkdf;
use sha2::Sha256;
use std::io;

pub fn derive_session_key(nonce: &[u8; 32], our_pub: &[u8; 32], peer_pub: &[u8; 32]) -> [u8; 32] {
    let (pk1, pk2) = if our_pub <= peer_pub {
        (our_pub, peer_pub)
    } else {
        (peer_pub, our_pub)
    };

    let mut ikm = Vec::with_capacity(32 + 32 + 32);
    ikm.extend_from_slice(nonce);
    ikm.extend_from_slice(pk1);
    ikm.extend_from_slice(pk2);

    let hk = Hkdf::<Sha256>::new(None, &ikm);
    let mut key = [0u8; 32];
    hk.expand(b"fastshare-session", &mut key)
        .expect("HKDF expand should never fail");
    key
}

fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).unwrap();
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .expect("encryption failed");
    let mut out = Vec::with_capacity(12 + ciphertext.len());

    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    out
}

fn decrypt(key: &[u8; 32], data: &[u8]) -> io::Result<Vec<u8>> {
    if data.len() < 12 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ciphertext too short",
        ));
    }

    let cipher = Aes256Gcm::new_from_slice(key).unwrap();

    let (nonce_bytes, ciphertext) = data.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "decryption failed"))
}
