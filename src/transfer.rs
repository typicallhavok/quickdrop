use crate::protocol::{FILE_BEGIN, FILE_CHUNK, FILE_END, SecureChannel, secure_read, secure_write};
use std::f32::consts::E;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::{io, path::Path};

pub fn send_file(
    stream: &mut TcpStream,
    channel: &mut SecureChannel,
    path: &Path,
) -> io::Result<()> {
    let metadata = fs::metadata(path)?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8 in file name"))?
        .as_bytes();
    let file_size: [u8; 8] = metadata.len().to_be_bytes();
    let mut payload = Vec::with_capacity(2 + file_name.len() + 8);

    payload.extend_from_slice(&file_size);
    payload.extend_from_slice(&(file_name.len() as u16).to_be_bytes());
    payload.extend_from_slice(file_name);

    secure_write(stream, channel, FILE_BEGIN, &payload)?;

    let mut file = File::open(path)?;
    let mut buf = [0u8; 16384];

    loop {
        let bytes_read = file.read(&mut buf)?;
        if bytes_read == 0 {
            break;
        }
        secure_write(stream, channel, FILE_CHUNK, &buf[..bytes_read])?;
    }
    secure_write(stream, channel, FILE_END, &[])?;

    Ok(())
}

pub fn receive_file(
    stream: &mut TcpStream,
    channel: &mut SecureChannel,
    path: &Path,
    expected_size: u64,
    expected_name: &str,
) -> io::Result<()> {
    let (msg_type, payload) = match secure_read(stream, channel) {
        Ok((msg_type, payload)) => (msg_type, payload),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
        Err(e) => return Err(e),
    };
    if msg_type != FILE_BEGIN || payload.len() < 10 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid data"));
    }

    let file_size = u64::from_be_bytes(
        payload[..8]
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid file size"))?,
    );
    let name_len = u16::from_be_bytes(
        payload[8..10]
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid file size"))?,
    );
    if payload.len() != 10 + name_len as usize {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid length"));
    }

    let file_name = match std::str::from_utf8(&payload[10..10 + name_len as usize]) {
        Ok(string) => string.to_string(),
        Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
    };

    if file_size != expected_size || file_name != expected_name {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "offer mismatch"));
    }

    let p = Path::new(&file_name);
    if p.components().count() != 1 || p.is_absolute() || file_name.contains("..") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "directory traversal in file name",
        ));
    }

    let unconfirmed_path = path.join(file_name.clone() + ".unconfirmed");
    let final_path = path.join(file_name);

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&unconfirmed_path)?;
    let mut remaining = file_size;

    while remaining > 0 {
        let (msg_type, payload) = secure_read(stream, channel)?;
        if msg_type != FILE_CHUNK || payload.len() > remaining as usize || payload.len() == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid data"));
        }
        remaining -= payload.len() as u64;

        file.write_all(&payload)?;
    }

    let (msg_type, payload) = secure_read(stream, channel)?;

    if msg_type == FILE_END
        && payload.is_empty()
        && fs::metadata(&unconfirmed_path)?.len() == file_size
    {
        drop(file);
        fs::rename(&unconfirmed_path, final_path)?;
    } else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid data"));
    }

    Ok(())
}
