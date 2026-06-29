use crate::crypto::create_stream_cipher;
use crate::protocol::{
    FILE_BEGIN, FILE_CHUNK_SIZE, FILE_END, SecureChannel, secure_read, secure_write,
};
use ctr::cipher::StreamCipher;
use std::{io, path::Path};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Configure TCP socket for large, sustained transfers.
///
/// We deliberately do NOT pin `SO_SNDBUF`/`SO_RCVBUF` here: on Windows, setting
/// the receive buffer manually disables TCP receive-window auto-tuning, which
/// caps throughput at the fixed buffer size instead of letting the window grow
/// to fill a fast link. Leaving them unset lets Windows auto-tune the window,
/// which is faster on a normal LAN. We still disable Nagle (low latency for the
/// small control frames) and enable keepalive (so a dead peer is noticed).
pub fn configure_socket_for_transfer(stream: &TcpStream) {
    use socket2::Socket;
    use std::os::windows::io::{AsRawSocket, FromRawSocket};

    let raw = stream.as_raw_socket();
    let sock = unsafe { Socket::from_raw_socket(raw) };

    let _ = stream.set_nodelay(true);

    let keepalive = socket2::TcpKeepalive::new()
        .with_time(std::time::Duration::from_secs(15))
        .with_interval(std::time::Duration::from_secs(10));
    let _ = sock.set_tcp_keepalive(&keepalive);

    std::mem::forget(sock);
}

/// Find a name that collides with neither an existing final file nor an
/// existing `.unconfirmed` partial, inserting " (n)" before the extension as
/// needed. Used when resume is disabled so a fresh file is always created.
fn unique_base_name(dir: &Path, name: &str) -> String {
    fn is_free(dir: &Path, cand: &str) -> bool {
        !dir.join(cand).exists() && !dir.join(format!("{}.unconfirmed", cand)).exists()
    }
    if is_free(dir, name) {
        return name.to_string();
    }
    let p = Path::new(name);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    let ext_str = if ext.is_empty() {
        String::new()
    } else {
        format!(".{}", ext)
    };
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string();
    let mut i = 1;
    loop {
        let cand = format!("{} ({}){}", stem, i, ext_str);
        if is_free(dir, &cand) {
            return cand;
        }
        i += 1;
    }
}

pub async fn offer_and_send_file<F>(
    stream: &mut TcpStream,
    channel: &mut SecureChannel,
    path: &Path,
    on_progress: F,
) -> io::Result<()>
where
    F: Fn(u64) -> bool + Send + Sync,
{
    let metadata = fs::metadata(path).await?;
    let file_name = path.file_name().unwrap().to_str().unwrap();
    let file_size: u64 = metadata.len();

    let mut offer_payload = Vec::new();
    offer_payload.extend_from_slice(&file_size.to_be_bytes());
    offer_payload.extend_from_slice(&(file_name.len() as u16).to_be_bytes());
    offer_payload.extend_from_slice(file_name.as_bytes());

    secure_write(stream, channel, crate::protocol::FILE_OFFER, &offer_payload).await?;
    let (msg_type, accept_payload) = secure_read(stream, channel).await?;

    if msg_type == crate::protocol::OFFER_ACCEPT {
        let mut resume_offset = 0u64;
        if accept_payload.len() == 8 {
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&accept_payload);
            resume_offset = u64::from_be_bytes(arr);
        }

        secure_write(stream, channel, crate::protocol::FILE_UPLOAD, &[]).await?;
        send_file(stream, channel, path, resume_offset, on_progress).await?;
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "Offer rejected by peer",
        ))
    }
}

pub async fn send_file<F>(
    stream: &mut TcpStream,
    channel: &mut SecureChannel,
    path: &Path,
    resume_offset: u64,
    on_progress: F,
) -> io::Result<()>
where
    F: Fn(u64) -> bool + Send + Sync,
{
    configure_socket_for_transfer(stream);

    let metadata = fs::metadata(path).await?;
    let total_size = metadata.len();
    let file_name_str = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8 in file name"))?;
    let file_name = file_name_str.as_bytes();

    let file_size: [u8; 8] = total_size.to_be_bytes();
    let mut payload = Vec::with_capacity(2 + file_name.len() + 8);

    payload.extend_from_slice(&file_size);
    payload.extend_from_slice(&(file_name.len() as u16).to_be_bytes());
    payload.extend_from_slice(file_name);

    eprintln!(
        "[resume] send start: name={} total_size={} resume_offset={} will_send={}",
        file_name_str,
        total_size,
        resume_offset,
        total_size.saturating_sub(resume_offset)
    );

    secure_write(stream, channel, FILE_BEGIN, &payload).await?;

    let mut cipher = create_stream_cipher(&channel.key, channel.send_ctr);
    channel.send_ctr += 1;

    let path_buf = path.to_owned();
    let (data_tx, mut data_rx) = tokio::sync::mpsc::channel::<(Vec<u8>, usize)>(3);
    let (free_tx, mut free_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(3);

    for _ in 0..3 {
        let _ = free_tx.send(vec![0u8; FILE_CHUNK_SIZE]).await;
    }

    let mut file = File::open(&path_buf).await?;
    if resume_offset > 0 {
        file.seek(std::io::SeekFrom::Start(resume_offset)).await?;
    }

    let _reader_handle = tokio::spawn(async move {
        loop {
            let mut buf = match free_rx.recv().await {
                Some(b) => b,
                None => break,
            };

            let bytes_read = match file.read(&mut buf).await {
                Ok(n) => n,
                Err(_) => break,
            };

            if bytes_read == 0 {
                break;
            }

            cipher.apply_keystream(&mut buf[..bytes_read]);

            if data_tx.send((buf, bytes_read)).await.is_err() {
                break;
            }
        }
    });

    let mut bytes_sent: u64 = resume_offset;
    while let Some((buf, len)) = data_rx.recv().await {
        stream.write_all(&buf[..len]).await?;
        bytes_sent += len as u64;
        if !on_progress(bytes_sent) {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Cancelled by user",
            ));
        }

        let _ = free_tx.send(buf).await;
    }

    secure_write(stream, channel, FILE_END, &[]).await?;

    Ok(())
}

pub async fn receive_file<F>(
    stream: &mut TcpStream,
    channel: &mut SecureChannel,
    path: &Path,
    expected_size: u64,
    expected_name: &str,
    resume: bool,
    resume_offset: u64,
    on_progress: F,
) -> io::Result<()>
where
    F: Fn(u64) -> bool + Send + Sync,
{
    configure_socket_for_transfer(stream);

    let (msg_type, payload) = match secure_read(stream, channel).await {
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

    let _ = fs::create_dir_all(path).await;

    // Decide the on-disk name. With resume enabled (default) we reuse the
    // existing `<name>.unconfirmed` partial and continue into it. With resume
    // disabled, pick a fresh unique base so an interrupted partial is never
    // appended to — the result always lands as a new file (with a " (n)" suffix
    // when a file or partial of that name already exists).
    let base_name = if resume {
        file_name.clone()
    } else {
        unique_base_name(path, &file_name)
    };

    let unconfirmed_path = path.join(format!("{}.unconfirmed", base_name));
    let mut final_path = path.join(&base_name);

    // The resume point was negotiated at accept time and sent to the sender, who
    // streams exactly `file_size - resume_offset` body bytes. Treat that offset
    // as authoritative and force the partial file to exactly that length, rather
    // than recomputing it from the on-disk length here. The two can disagree
    // (e.g. if the accept-time truncation didn't persist), and any mismatch makes
    // us read the wrong number of body bytes — corrupting the FILE_END frame,
    // throwing, and aborting the transfer (which shows as a failure on the
    // sender). Forcing the length keeps both sides in lock-step.
    let existing_length = if resume { resume_offset } else { 0 };
    let total_expected_size = file_size;
    let bytes_to_receive = file_size.saturating_sub(existing_length);

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(&unconfirmed_path)
        .await?;
    file.set_len(existing_length).await?;
    file.seek(std::io::SeekFrom::Start(existing_length)).await?;

    let mut remaining = bytes_to_receive;
    let mut bytes_received: u64 = 0;

    let mut cipher = create_stream_cipher(&channel.key, channel.recv_ctr);
    channel.recv_ctr += 1;

    let (data_tx, mut data_rx) = tokio::sync::mpsc::channel::<(Vec<u8>, usize)>(3);
    let (free_tx, mut free_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(3);

    for _ in 0..3 {
        let _ = free_tx.send(vec![0u8; FILE_CHUNK_SIZE]).await;
    }

    let writer_handle = tokio::spawn(async move {
        while let Some((mut buf, len)) = data_rx.recv().await {
            cipher.apply_keystream(&mut buf[..len]);
            if file.write_all(&buf[..len]).await.is_err() {
                break;
            }
            let _ = free_tx.send(buf).await;
        }
        let _ = file.flush().await;
    });

    while remaining > 0 {
        let mut buf = free_rx
            .recv()
            .await
            .unwrap_or_else(|| vec![0u8; FILE_CHUNK_SIZE]);
        let to_read = std::cmp::min(remaining, buf.len() as u64) as usize;
        stream.read_exact(&mut buf[..to_read]).await?;

        remaining -= to_read as u64;
        bytes_received += to_read as u64;
        if !on_progress(existing_length + bytes_received) {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Cancelled by user",
            ));
        }

        if data_tx.send((buf, to_read)).await.is_err() {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "writer failed",
            ));
        }
    }

    drop(data_tx);
    let _ = writer_handle.await;

    let (msg_type, payload) = secure_read(stream, channel).await?;
    let actual_len = fs::metadata(&unconfirmed_path).await?.len();
    eprintln!(
        "[resume] receive end: name={} resume={} resume_offset={} existing_length={} expected={} got_msg=0x{:02x} payload_len={} unconfirmed_len={}",
        expected_name, resume, resume_offset, existing_length, total_expected_size, msg_type, payload.len(), actual_len
    );

    if msg_type == FILE_END && payload.is_empty() && actual_len == total_expected_size {
        if final_path.exists() {
            let mut i = 1;
            let ext = final_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_string();
            let ext_str = if ext.is_empty() {
                String::new()
            } else {
                format!(".{}", ext)
            };
            let stem = final_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(expected_name)
                .to_string();
            loop {
                let new_name = format!("{} ({}){}", stem, i, ext_str);
                final_path = path.join(new_name);
                if !final_path.exists() {
                    break;
                }
                i += 1;
            }
        }
        fs::rename(&unconfirmed_path, final_path).await?;
    } else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid data"));
    }

    Ok(())
}
