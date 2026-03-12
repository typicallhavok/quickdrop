use ed25519_dalek::{PUBLIC_KEY_LENGTH, Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::io;
use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;

use crate::crypto::derive_session_key;
use crate::protocol::{
    ACCEPT, HEADER_SIZE, IDENTITY_CHALLENGE, IDENTITY_HELLO, IDENTITY_PROOF, OFFER_ACCEPT,
    OFFER_REJECT, REJECT, make_header, parse_header,
};

use crate::identity::{NONCE_SIZE, State, Store, generate_nonce, is_trusted};

use crate::protocol::{read_message, write_message};

pub struct HandshakeContext<'a> {
    store: Arc<Mutex<Store>>,
    store_path: &'a str,
    state: State,
    nonce: Option<[u8; NONCE_SIZE]>,
    client_nonce: Option<[u8; NONCE_SIZE]>,
    peer_public_key: Option<Vec<u8>>,
    peer_name: Option<String>,
    local_public_key: [u8; 32],
    local_name: &'a str,
}

impl<'a> HandshakeContext<'a> {
    pub fn new(
        store: Arc<Mutex<Store>>,
        store_path: &'a str,
        local_public_key: [u8; 32],
        local_name: &'a str,
    ) -> Self {
        Self {
            store,
            store_path,
            state: State::New,
            nonce: None,
            client_nonce: None,
            peer_public_key: None,
            peer_name: None,
            local_public_key,
            local_name,
        }
    }
}

pub async fn run_handshake<'a>(
    stream: &mut TcpStream,
    store: Arc<Mutex<Store>>,
    store_path: &'a str,
    local_public_key: &[u8; 32],
    local_name: &str,
) -> io::Result<([u8; 32],[u8; 32])> {
    let mut ctx = HandshakeContext::new(store, store_path, *local_public_key, local_name);

    let (msg_type, payload) = read_message(stream).await?;

    handle_identity_hello(&mut ctx, msg_type, &payload)?;

    send_challenge(&mut ctx, stream).await?;

    let (msg_type, payload) = read_message(stream).await?;

    handle_identity_proof(stream, &mut ctx, msg_type, &payload).await?;

    if ctx.state != State::Authenticated {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "permission denied",
        ));
    }

    let peer_vec = ctx
        .peer_public_key
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no public key"))?;

    let public_key_bytes = peer_vec.as_slice();
    let peer_key_check: &[u8; 32] = public_key_bytes
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid public key"))?;

    let peer_key = peer_key_check;
    let server_nonce = ctx
        .nonce
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing nonce"))?;
    let client_nonce = ctx
        .client_nonce
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing nonce"))?;

    let session_key =
        derive_session_key(server_nonce, client_nonce, &ctx.local_public_key, peer_key);

    ctx.nonce = None;
    Ok((session_key, *peer_key))
}

pub async fn run_client_handshake<'a>(
    stream: &mut TcpStream,
    signing_key: &SigningKey,
    peer_name: &str,
    store: Arc<std::sync::Mutex<Store>>,
    store_path: &'a str,
    local_public_key: &[u8; 32],
    local_name: &str,
) -> io::Result<[u8; 32]> {

    let mut ctx = HandshakeContext::new(store, store_path, *local_public_key, local_name);

    let client_nonce = generate_nonce();
    let mut payload = Vec::with_capacity(NONCE_SIZE + 32 + peer_name.len());
    payload.extend_from_slice(&client_nonce);
    payload.extend_from_slice(&(PUBLIC_KEY_LENGTH as u16).to_be_bytes());
    payload.extend_from_slice(&ctx.local_public_key);
    payload.extend_from_slice(&(ctx.local_name.len() as u16).to_be_bytes());
    payload.extend_from_slice(ctx.local_name.as_bytes());

    write_message(stream, IDENTITY_HELLO, &payload).await?;
    let (msg_type, payload) = read_message(stream).await?;

    let peer_public_key: [u8; 32];
    let server_nonce: [u8; NONCE_SIZE];

    match msg_type {
        IDENTITY_CHALLENGE => {
            server_nonce = payload[..NONCE_SIZE].try_into().unwrap();
            let mut bytes_sign = Vec::with_capacity(2 * NONCE_SIZE + 2 * PUBLIC_KEY_LENGTH);
            bytes_sign.extend_from_slice(&payload[..NONCE_SIZE]);
            bytes_sign.extend_from_slice(&client_nonce);
            peer_public_key = payload[NONCE_SIZE + 32..NONCE_SIZE + 64]
                .try_into()
                .expect("invalid key");
            ctx.peer_public_key = Some(peer_public_key.to_vec());
            let peer_hash = Sha256::digest(peer_public_key);
            let client_hash = Sha256::digest(ctx.local_public_key);
            bytes_sign.extend_from_slice(&peer_hash);
            bytes_sign.extend_from_slice(&client_hash);
            let signature = signing_key.sign(&bytes_sign);
            write_message(stream, IDENTITY_PROOF, &signature.to_bytes()).await?;
        }

        _ => {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid data"));
        }
    }
    let (msg_type, _) = read_message(stream).await?;
    if msg_type != ACCEPT {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Server rejected handshake",
        ));
    }
    ctx.state = State::Authenticated;

    let session_key = derive_session_key(
        &server_nonce,
        &client_nonce,
        &ctx.local_public_key,
        &peer_public_key,
    );

    Ok(session_key)
}

fn handle_identity_hello(
    ctx: &mut HandshakeContext,
    msg_type: u8,
    payload: &[u8],
) -> io::Result<()> {
    if msg_type != IDENTITY_HELLO {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected IDENTITY_HELLO",
        ));
    }

    let (client_nonce, public_key, name) = parse_identity_hello(payload)?;

    ctx.peer_public_key = Some(public_key);
    ctx.peer_name = Some(name);
    ctx.state = State::HelloReceived;
    ctx.client_nonce = Some(client_nonce);
    Ok(())
}

async fn send_challenge<'a> (ctx: &mut HandshakeContext<'a>, stream: &mut TcpStream) -> io::Result<()> {
    let nonce = generate_nonce();
    let public_key = ctx
        .peer_public_key
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no public key"))?;
    let hash = Sha256::digest(public_key);
    let mut payload = Vec::with_capacity(96);

    payload.extend_from_slice(&nonce);
    payload.extend_from_slice(&hash);
    payload.extend_from_slice(&ctx.local_public_key);
    write_message(stream, IDENTITY_CHALLENGE, &payload).await?;

    ctx.nonce = Some(nonce);
    ctx.state = State::ChallengeSent;
    Ok(())
}

async fn handle_identity_proof<'a> (
    stream: &mut TcpStream,
    ctx: &mut HandshakeContext<'a>,
    msg_type: u8,
    payload: &[u8],
) -> io::Result<()> {
    if msg_type != IDENTITY_PROOF {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected IDENTITY_PROOF",
        ));
    }

    if payload.len() != 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid signature",
        ));
    }

    let signature = payload;

    let server_nonce = ctx
        .nonce
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing nonce"))?;

    let client_nonce = ctx
        .client_nonce
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing nonce"))?;

    let peer_public_key = ctx
        .peer_public_key
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing public key"))?;

    let server_public_key = &ctx.local_public_key;

    if !verify_signature(
        peer_public_key,
        server_nonce,
        client_nonce,
        server_public_key,
        signature,
    ) {
        send_handshake_reject(stream).await?;
        ctx.state = State::Rejected;
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "permission denied",
        ));
    }

    ctx.state = State::Authenticated;

    let trusted = is_trusted(ctx.store.clone(), peer_public_key);

    send_handshake_accept(stream, trusted).await?;
    Ok(())
}

fn verify_signature(
    public_key: &[u8],
    nonce: &[u8],
    client_nonce: &[u8],
    server_public_key: &[u8],
    signature: &[u8],
) -> bool {
    let pk = match public_key.try_into() {
        Ok(pk) => pk,
        Err(_) => return false,
    };

    let sig: Signature = Signature::from_bytes(match signature.try_into() {
        Ok(s) => s,
        Err(_) => return false,
    });
    let mut message = Vec::with_capacity(128);

    message.extend_from_slice(nonce);

    message.extend_from_slice(client_nonce);

    let peer_hash = Sha256::digest(pk);
    let server_hash = Sha256::digest(server_public_key);

    message.extend_from_slice(&server_hash);
    message.extend_from_slice(&peer_hash);

    let verifying_key = VerifyingKey::from_bytes(pk);
    match verifying_key {
        Ok(verifying_key) => verifying_key.verify(&message, &sig).is_ok(),
        Err(_) => return false,
    }
}

fn parse_identity_hello(payload: &[u8]) -> io::Result<([u8; NONCE_SIZE], Vec<u8>, String)> {
    if payload.len() < NONCE_SIZE + 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "payload too short",
        ));
    }

    let mut client_nonce = [0u8; NONCE_SIZE];
    client_nonce.copy_from_slice(&payload[..NONCE_SIZE]);

    let mut offset = NONCE_SIZE;
    let pk_len = u16::from_be_bytes([payload[offset], payload[offset + 1]]) as usize;
    offset += 2;

    if payload.len() < offset + pk_len + 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid public key length",
        ));
    }

    let public_key = payload[offset..offset + pk_len].to_vec();
    offset += pk_len;

    let name_len = u16::from_be_bytes([payload[offset], payload[offset + 1]]) as usize;
    offset += 2;

    if payload.len() != offset + name_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid name length",
        ));
    }

    let name = String::from_utf8(payload[offset..].to_vec())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid utf-8 name"))?;

    Ok((client_nonce, public_key, name))
}

async fn send_handshake_accept(stream: &mut TcpStream, auto_send: bool) -> io::Result<()> {
    let payload = [auto_send as u8];

    write_message(stream, ACCEPT, &payload).await
}

async fn send_handshake_reject(stream: &mut TcpStream) -> io::Result<()> {
    write_message(stream, REJECT, &[]).await
}
