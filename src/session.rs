use crate::{
    identity::{Store, add_trusted, is_trusted},
    protocol::{
        ACCEPT, FILE_OFFER, FILE_UPLOAD, OFFER_ACCEPT, OFFER_REJECT, REJECT, SecureChannel,
        secure_read, secure_write,
    },
    transfer::receive_file,
};
use std::{io, path::Path, sync::{Arc, Mutex}};
use tokio::net::TcpStream;

#[derive(PartialEq)]
enum IncomingState {
    Idle,
    AwaitingUpload {
        expected_size: u64,
        expected_name: String,
    },
}

pub async fn run_session(
    stream: &mut TcpStream,
    channel: &mut SecureChannel,
    store: Arc<Mutex<Store>>,
    download_dir: &Path,
    peer_public_key: [u8; 32],
) -> io::Result<()> {
    let mut state: IncomingState = IncomingState::Idle;
    loop {
        let (msg_type, payload) = match secure_read(stream, channel).await {
            Ok(res) => res,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                println!("Peer disconnected.");
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

            if file_size > 10737418240 || payload.len() != 10 + name_len as usize {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid payload",
                ));
            }

            let file_name = match std::str::from_utf8(&payload[10..10 + name_len as usize]) {
                Ok(string) => string.to_string(),
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            };

            println!("{} {}", file_size, file_name); // ADD UI HERE

            if true { // change to is_trusted(&store, &peer_public_key) later
                state = IncomingState::AwaitingUpload {
                    expected_size: file_size,
                    expected_name: file_name.clone(),
                };
                send_offer_accept(stream, channel).await?;
            } else {
                state = IncomingState::Idle;
                send_offer_reject(stream, channel).await?;
            }
        } else if msg_type == FILE_UPLOAD {
            if let IncomingState::AwaitingUpload {
                expected_size,
                expected_name,
            } = &state
            {
                receive_file(stream, channel, download_dir, *expected_size, expected_name).await?;
                state = IncomingState::Idle;
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

async fn send_offer_accept(stream: &mut TcpStream, channel: &mut SecureChannel) -> io::Result<()> {
    secure_write(stream, channel, OFFER_ACCEPT, &[]).await?;
    Ok(())
}

async fn send_offer_reject(stream: &mut TcpStream, channel: &mut SecureChannel) -> io::Result<()> {
    secure_write(stream, channel, OFFER_REJECT, &[]).await?;
    Ok(())
}
