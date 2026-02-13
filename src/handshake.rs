use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::thread::LocalKey;

use crate::crypto::derive_session_key;
use crate::protocol::{
    ACCEPT, HEADER_SIZE, IDENTITY_CHALLENGE, IDENTITY_HELLO, IDENTITY_PROOF, REJECT, make_header,
    parse_header,
};

use crate::identity::{NONCE_SIZE, State, Store, generate_nonce, is_trusted};

use crate::protocol::{read_message, write_message};

struct HandshakeContext<'a> {
    store: &'a mut Store,
    store_path: &'a str,
    state: State,
    nonce: Option<[u8; NONCE_SIZE]>,
    peer_public_key: Option<Vec<u8>>,
    peer_name: Option<String>,
}

impl<'a> HandshakeContext<'a> {
    fn new(store: &'a mut Store, store_path: &'a str) -> Self {
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

pub fn run_handshake<'a>(
    stream: &mut TcpStream,
    store: &'a mut Store,
    store_path: &'a str,
    local_public_key: &[u8; 32],
) -> io::Result<[u8; 32]> {
    let mut ctx = HandshakeContext::new(store, store_path);

    let (msg_type, payload) = read_message(stream)?;
    handle_identity_hello(&mut ctx, msg_type, &payload)?;

    send_challenge(&mut ctx, stream)?;

    let (msg_type, payload) = read_message(stream)?;
    handle_identity_proof(stream, &mut ctx, msg_type, &payload)?;

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
    
    let nonce = ctx
        .nonce
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing nonce"))?;

    let session_key = derive_session_key(nonce, local_public_key, peer_key);

    ctx.nonce = None;

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
    stream: &mut TcpStream,
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

    if payload.len() != 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid signature",
        ));
    }

    let signature = payload;

    let nonce = ctx
        .nonce
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing nonce"))?;

    let public_key = ctx
        .peer_public_key
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing public key"))?;

    if !verify_signature(public_key, nonce, signature) {
        send_reject(stream)?;
        ctx.state = State::Rejected;
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "permission denied",
        ));
    }

    ctx.state = State::Authenticated;
    let trusted = is_trusted(ctx.store, public_key);
    send_accept(stream, trusted)?;

    Ok(())
}

fn verify_signature(public_key: &[u8], nonce: &[u8], signature: &[u8]) -> bool {
    let pk = match public_key.try_into() {
        Ok(pk) => pk,
        Err(_) => return false,
    };

    let sig: Signature = Signature::from_bytes(match signature.try_into() {
        Ok(s) => s,
        Err(_) => return false,
    });
    let mut message = Vec::with_capacity(64);
    message.extend_from_slice(nonce);
    let hash = Sha256::digest(pk);
    message.extend_from_slice(&hash);
    let verifying_key = VerifyingKey::from_bytes(pk);
    match verifying_key {
        Ok(verifying_key) => verifying_key.verify(&message, &sig).is_ok(),
        Err(_) => return false,
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

fn send_accept(stream: &mut TcpStream, auto_send: bool) -> io::Result<()> {
    let payload = [auto_send as u8];
    write_message(stream, ACCEPT, &payload)
}

fn send_reject(stream: &mut TcpStream) -> io::Result<()> {
    write_message(stream, REJECT, &[])
}
