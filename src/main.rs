mod crypto;
mod handshake;
mod identity;
mod protocol;
mod session;
mod transfer;
use crate::handshake::{run_client_handshake, run_handshake};
use crate::identity::{Store, load_store};
use crate::protocol::{
    FILE_OFFER, FILE_UPLOAD, OFFER_ACCEPT, SecureChannel, secure_read, secure_write,
};
use crate::session::run_session;
use crate::transfer::send_file;
use ed25519_dalek::{SigningKey, VerifyingKey};
use local_ip_address::local_ip;
use rand::rngs::OsRng;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};

use std::{fs, io, path::Path};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store_path = "./store.json";
    let store = load_store(store_path);
    let local_name: &str = "amogh";
    let download_dir = Path::new("./downloads");
    if !download_dir.exists() {
        fs::create_dir_all(download_dir)?;
    }

    let my_local_ip = local_ip()?;
    println!("Share initialized on: {:?}:2222", my_local_ip);

    let path = Path::new("identity.key");
    let signing_key = if path.exists() {
        let bytes = std::fs::read("identity.key")?;
        let key_bytes: [u8; 32] = bytes.try_into().expect("identity.key must be 32 bytes");
        SigningKey::from_bytes(&key_bytes)
    } else {
        let signing_key = SigningKey::generate(&mut OsRng);
        std::fs::write("identity.key", signing_key.to_bytes())?;
        signing_key
    };

    let verifying_key = signing_key.verifying_key();

    // println!("Public key: {:?}", verifying_key);
    // println!("Secret key: {:?}", signing_key);

    let recv_store = Arc::clone(&store);

    tokio::spawn(async move {
        receive(
            recv_store,
            store_path.to_string(),
            local_name.to_string(),
            verifying_key,
            download_dir.to_path_buf(),
        ).await.unwrap()
    });

    loop {
        let mut command = String::new();
        io::stdin().read_line(&mut command)?;
        let chunks: Vec<&str> = command.trim().split_whitespace().collect();
        match chunks.as_slice() {
            ["send", file_path, target] => {
                println!("{}:2222", target);
                let mut stream = TcpStream::connect(format!("{}:2222", target)).await?;
                let session_key = run_client_handshake(
                    &mut stream,
                    &signing_key,
                    local_name,
                    Arc::clone(&store),
                    store_path,
                    verifying_key.as_bytes(),
                    local_name,
                )
                .await?;
                let mut channel = SecureChannel::new(session_key);
                let path = Path::new(file_path);
                let file_size = fs::metadata(&path)?.len();
                let file_name = path.file_name().unwrap().to_str().unwrap();
                let mut offer_payload = Vec::new();
                offer_payload.extend_from_slice(&file_size.to_be_bytes());
                offer_payload.extend_from_slice(&(file_name.len() as u16).to_be_bytes());
                offer_payload.extend_from_slice(file_name.as_bytes());

                secure_write(&mut stream, &mut channel, FILE_OFFER, &offer_payload).await?;
                let (msg_type, _) = secure_read(&mut stream, &mut channel).await?;
                if msg_type == OFFER_ACCEPT {
                    println!("sending...");
                    secure_write(&mut stream, &mut channel, FILE_UPLOAD, &[]).await?;
                    send_file(&mut stream, &mut channel, &path).await?;
                    println!("Transfer complete");
                } else {
                    println!("rejected");
                }
            }
            ["exit"] => break,
            _ => println!("Invalid"),
        };
    }

    Ok(())
}

async fn receive(
    store: Arc<Mutex<Store>>,
    store_path: String,
    local_name: String,
    verifying_key: VerifyingKey,
    download_dir: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("0.0.0.0:2222").await?;
    loop {
        println!("Listening for incoming files...");
        let store = Arc::clone(&store);
        let store_path = store_path.clone();
        let local_name = local_name.clone();
        let download_dir = download_dir.clone();
        let verifying_key = verifying_key;
        let (mut stream, _addr) = listener.accept().await?;
        tokio::spawn(async move {
            let (session_key, peer_public_key) = run_handshake(
                &mut stream,
                Arc::clone(&store),
                &store_path,
                verifying_key.as_bytes(),
                &local_name,
            )
            .await.unwrap();
            let mut channel = SecureChannel::new(session_key);
            run_session(
                &mut stream,
                &mut channel,
                Arc::clone(&store),
                &download_dir,
                peer_public_key,
            )
            .await.unwrap();
        });
    }
}
