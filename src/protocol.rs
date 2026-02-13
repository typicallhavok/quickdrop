use std::io::{self, Read, Write};
use std::net::TcpStream;

pub const IDENTITY_HELLO: u8 = 0x10;
pub const IDENTITY_CHALLENGE: u8 = 0x11;
pub const IDENTITY_PROOF: u8 = 0x12;
pub const ACCEPT: u8 = 0x13;
pub const REJECT: u8 = 0x14;
pub const FILE_UPLOAD: u8 = 0x20;
pub const HEADER_SIZE: usize = 9;

pub const MODE_RAW: u8 = 0x01;
pub const MODE_AES: u8 = 0x02;

pub const FILE_BEGIN: u8 = 0x20;
pub const FILE_CHUNK: u8 = 0x21;
pub const FILE_END: u8 = 0x22;

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

pub fn write_message(stream: &mut TcpStream, msg_type: u8, payload: &[u8]) -> io::Result<()> {
    let header = make_header(msg_type, payload.len() as u64);
    stream.write_all(&header)?;
    stream.write_all(payload)?;
    Ok(())
}

pub fn read_message(stream: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; HEADER_SIZE];
    stream.read_exact(&mut header)?;

    let (msg_type, payload_len) = parse_header(header);

    let mut payload = vec![0u8; payload_len as usize];
    stream.read_exact(&mut payload)?;

    Ok((msg_type, payload))
}
