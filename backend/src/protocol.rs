use std::io;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use uuid::uuid;

use crate::crypto::{decrypt, encrypt};

pub const IDENTITY_HELLO: u8 = 0x10;
pub const IDENTITY_CHALLENGE: u8 = 0x11;
pub const IDENTITY_PROOF: u8 = 0x12;
pub const ACCEPT: u8 = 0x13;
pub const REJECT: u8 = 0x14;
pub const HEADER_SIZE: usize = 9;

pub const MODE_RAW: u8 = 0x01;
pub const MODE_AES: u8 = 0x02;

pub const FILE_BEGIN: u8 = 0x19;
pub const FILE_UPLOAD: u8 = 0x20;
pub const FILE_CHUNK: u8 = 0x21;
pub const FILE_END: u8 = 0x22;
pub const FILE_OFFER: u8 = 0x23;
pub const OFFER_ACCEPT: u8 = 0x24;
pub const OFFER_REJECT: u8 = 0x25;

pub const FILE_CHUNK_SIZE: usize = 4 * 1024 * 1024;

pub const BLE_UUID: u128 = uuid!("00001d09-0000-1000-8000-00805f9b34fb").as_u128();
pub const WIFI_INFO_UUID: u128 = uuid!("00001d0a-0000-1000-8000-00805f9b34fb").as_u128();
pub const WIFIDIRECT_INFO_UUID: u128 = uuid!("00001d0b-0000-1000-8000-00805f9b34fb").as_u128();

pub const TCP_PORT: u16 = 55432; // file transfer (TCP)
pub const UDP_DISCOVERY_PORT: u16 = 55433; // device discovery (UDP)

pub struct SecureChannel {
    pub key: [u8; 32],
    pub send_ctr: u64,
    pub recv_ctr: u64,
}

impl SecureChannel {
    pub fn new(key: [u8; 32]) -> Self {
        Self {
            key,
            send_ctr: 0,
            recv_ctr: 0,
        }
    }
}

pub fn make_header(msg_type: u8, payload_len: u64) -> [u8; HEADER_SIZE] {
    let mut header = [0u8; HEADER_SIZE];

    header[0] = msg_type;
    header[1..HEADER_SIZE].copy_from_slice(&payload_len.to_be_bytes());
    header
}

pub fn parse_header(header: [u8; HEADER_SIZE]) -> (u8, u64) {
    let msg_type = header[0];
    let payload_len = u64::from_be_bytes(header[1..HEADER_SIZE].try_into().unwrap());

    (msg_type, payload_len)
}

pub async fn write_message(stream: &mut TcpStream, msg_type: u8, payload: &[u8]) -> io::Result<()> {
    let header = make_header(msg_type, payload.len() as u64);

    stream.write_all(&header).await?;
    stream.write_all(payload).await?;
    Ok(())
}

pub async fn read_message(stream: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; HEADER_SIZE];

    stream.read_exact(&mut header).await?;

    let (msg_type, payload_len) = parse_header(header);
    let mut payload = vec![0u8; payload_len as usize];

    stream.read_exact(&mut payload).await?;

    Ok((msg_type, payload))
}

pub async fn secure_write(
    stream: &mut TcpStream,
    channel: &mut SecureChannel,
    msg_type: u8,
    payload: &[u8],
) -> io::Result<()> {
    let header = make_header(msg_type, payload.len() as u64);
    let mut plain: Vec<u8> = Vec::with_capacity(HEADER_SIZE + payload.len());

    plain.extend_from_slice(&header);
    plain.extend_from_slice(&payload);

    let encrypted_payload = encrypt(&channel.key, channel.send_ctr, &plain);
    channel.send_ctr += 1;

    let len = encrypted_payload.len();
    if len > u32::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let len = len as u32;

    let mut message: Vec<u8> = Vec::with_capacity(4 + len as usize);

    message.extend_from_slice(&len.to_be_bytes());
    message.extend_from_slice(&encrypted_payload);
    stream.write_all(&message).await?;

    Ok(())
}

pub async fn secure_read(
    stream: &mut TcpStream,
    channel: &mut SecureChannel,
) -> io::Result<(u8, Vec<u8>)> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await?;

    let frame_len = u32::from_be_bytes(len);

    const MAX_FRAME: u32 = 1024 * 1024 * 64;
    if frame_len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }

    let mut message = vec![0u8; frame_len as usize];
    stream.read_exact(&mut message).await?;

    let decrypted_payload = decrypt(&channel.key, channel.recv_ctr, &message)?;

    if decrypted_payload.len() < HEADER_SIZE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid header"));
    }

    channel.recv_ctr += 1;
    let header_slice = &decrypted_payload[..HEADER_SIZE];
    let header: [u8; HEADER_SIZE] = header_slice
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid header"))?;
    let (msg_type, payload_len) = parse_header(header);

    if decrypted_payload.len() as u64 != HEADER_SIZE as u64 + payload_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "payload too short",
        ));
    }

    let payload = decrypted_payload[HEADER_SIZE..HEADER_SIZE + payload_len as usize].to_vec();

    Ok((msg_type, payload))
}
