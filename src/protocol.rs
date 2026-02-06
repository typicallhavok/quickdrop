pub const IDENTITY_HELLO: u8 = 0x10;
pub const IDENTITY_CHALLENGE: u8 = 0x11;
pub const IDENTITY_PROOF: u8 = 0x12;
pub const ACCEPT: u8 = 0x13;
pub const REJECT: u8 = 0x14;
pub const FILE_UPLOAD: u8 = 0x20;
pub const HEADER_SIZE:usize = 9;

pub fn make_header(msg_type: u8, payload_len: u64) -> [u8; HEADER_SIZE] {
    let mut header = [0u8; HEADER_SIZE];
    header[0] = msg_type;
    header[1..HEADER_SIZE].copy_from_slice(&payload_len.to_be_bytes());
    header
}

pub fn parse_header(header: [u8; HEADER_SIZE]) -> (u8, u64) {
    let msg_type = header[0];
    let payload_len = u64::from_be_bytes(
        header[1..HEADER_SIZE].try_into().unwrap()
    );
    (msg_type,payload_len)
}

