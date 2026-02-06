use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use std::net::TcpStream;

use crate::protocol::{
    HEADER_SIZE, IDENTITY_CHALLENGE, IDENTITY_HELLO, IDENTITY_PROOF, make_header, parse_header,
};

use crate::identity::{NONCE_SIZE, State, Store, device_id, generate_nonce};

struct HandshakeContext {
    store: Store,
    store_path: String,
    state: State,
    nonce: Option<[u8; NONCE_SIZE]>,
    peer_public_key: Option<Vec<u8>>,
    peer_name: Option<String>,
}

impl HandshakeContext {
    fn new(store: Store, store_path: String) -> Self {
        Self {
            store,
            store_path,
            state: State::New,
            nonce: None,
            peer_public_key: None,
            peer_name: None,
        }
    }
}

pub fn run_handshake(stream: &mut TcpStream, store: Store, store_path: String) -> io::Result<()> {
    let mut ctx = HandshakeContext::new(store, store_path);

    // 1. Read IDENTITY_HELLO
    let (msg_type, payload) = read_message(stream)?;
    handle_identity_hello(&mut ctx, msg_type, &payload)?;

    // 2. Send challenge
    send_challenge(&mut ctx, stream)?;

    // 3. Read IDENTITY_PROOF
    let (msg_type, payload) = read_message(stream)?;
    handle_identity_proof(&mut ctx, msg_type, &payload)?;

    // 4–6 (accept / reject) handled later
    Ok(())
}

fn read_message(stream: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; HEADER_SIZE];
    stream.read_exact(&mut header)?;

    let (msg_type, payload_len) = parse_header(header);

    let mut payload = vec![0u8; payload_len as usize];
    stream.read_exact(&mut payload)?;

    Ok((msg_type, payload))
}

fn write_message(stream: &mut TcpStream, msg_type: u8, payload: &[u8]) -> io::Result<()> {
    let header = make_header(msg_type, payload.len() as u64);
    stream.write_all(&header)?;
    stream.write_all(payload)?;
    Ok(())
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

    let (public_key, name): (Vec<u8>, String) = parse_identity_hello(payload)?;
    ctx.peer_public_key = Some(public_key);
    ctx.peer_name = Some(name);
    ctx.state = State::HelloReceived;
    Ok(())
}

fn send_challenge(ctx: &mut HandshakeContext, stream: &mut TcpStream) -> io::Result<()> {
    let nonce = generate_nonce();
    let public_key = ctx
        .peer_public_key
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no public key"))?;
    let hash = Sha256::digest(public_key);
    let mut payload = Vec::with_capacity(64);
    payload.extend_from_slice(&nonce);
    payload.extend_from_slice(&hash);
    write_message(stream, IDENTITY_CHALLENGE, &payload)?;
    ctx.nonce = Some(nonce);
    ctx.state = State::ChallengeSent;
    Ok(())
}

fn handle_identity_proof(
    ctx: &mut HandshakeContext,
    msg_type: u8,
    payload: &[u8],
) -> io::Result<()> {
    if msg_type != IDENTITY_PROOF {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected IDENTITY_PROOF",
        ));
    }

    let signature = parse_identity_proof(payload)?;

    let nonce = ctx
        .nonce
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing nonce"))?;

    let public_key = ctx
        .peer_public_key
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing public key"))?;

    if signature.len() != 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid signature",
        ));
    }

    if !verify_signature(public_key, &nonce, signature) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid signature",
        ));
    }

    ctx.state = State::Authenticated;
    Ok(())
}

fn parse_identity_proof(payload: &[u8]) -> io::Result<&[u8]> {
    if payload.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid payload",
        ));
    }

    let sig_len = u16::from_be_bytes([payload[0], payload[1]]) as usize;

    if payload.len() != 2 + sig_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid signature length",
        ));
    }

    Ok(&payload[2..])
}

fn verify_signature(public_key: &[u8], nonce: &[u8], signature: &[u8]) -> bool {
    let pk = match public_key.try_into() {
        Ok(pk) => pk,
        Err(_) => return false,
    };

    let sig: Signature = Signature::from_bytes(
        match signature.try_into() {
            Ok(s) => s,
            Err(_) => return false,
        }
    );
    let mut message = Vec::with_capacity(64);
    message.extend_from_slice(nonce);
    let hash= Sha256::digest(pk);
    message.extend_from_slice(&hash);
    let verifying_key = VerifyingKey::from_bytes(pk);
    match verifying_key {
        Ok(verifying_key) => verifying_key.verify(&message, &sig).is_ok(),
        Err(_) => return false
    }
}

fn parse_identity_hello(payload: &[u8]) -> io::Result<(Vec<u8>, String)> {
    let mut offset = 0;

    if payload.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "payload too short",
        ));
    }

    let pk_len = u16::from_be_bytes([payload[0], payload[1]]) as usize;
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

    Ok((public_key, name))
}
