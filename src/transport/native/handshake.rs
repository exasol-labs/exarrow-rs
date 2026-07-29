use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use num_bigint::BigUint;

use crate::error::TransportError;

use super::attributes::{AttributeSet, AttributeValue};
use super::constants::{
    ATTR_CLIENTNAME, ATTR_CLIENTOS, ATTR_CLIENTVERSION, ATTR_CLIENT_KEYS_LEN,
    ATTR_CLIENT_RECEIVE_KEY, ATTR_CLIENT_SEND_KEY, ATTR_DRIVERNAME, ATTR_ENCODED_PASSWORD,
    ATTR_USERNAME, CHACHA20_KEY_LEN, CHANGE_DATE, CMD_SET_ATTRIBUTES, HEADER_SIZE, LOGIN_MAGIC,
    PROTOCOL_VERSION,
};
use super::encryption::ChaCha20Encryptor;
use super::framing::{MessageHeader, SerialCounter};

/// Result of building the authentication message.
pub struct AuthMessage {
    pub wire_bytes: Vec<u8>,
    pub send_key: Vec<u8>,
    pub recv_key: Vec<u8>,
}

/// Build the initial login packet sent when first connecting.
///
/// Format: [MAGIC 4B] [msg_len 4B] [protocol_version 4B] [change_date 4B] [attributes...]
///
/// The Exasol C++ SDK writes these fields through `exaBswap32`, but that helper only swaps on
/// big-endian hosts. On the little-endian platforms we support, the login packet stays LE.
pub fn build_login_packet(username: &str) -> Vec<u8> {
    let mut attrs = AttributeSet::new();
    attrs.add(ATTR_USERNAME, AttributeValue::String(username.to_owned()));
    attrs.add(
        ATTR_CLIENTNAME,
        AttributeValue::String("exarrow-rs".to_owned()),
    );
    attrs.add(
        ATTR_DRIVERNAME,
        AttributeValue::String("exarrow-rs".to_owned()),
    );
    attrs.add(
        ATTR_CLIENTOS,
        AttributeValue::String(std::env::consts::OS.to_owned()),
    );
    attrs.add(
        ATTR_CLIENTVERSION,
        AttributeValue::String(env!("CARGO_PKG_VERSION").to_owned()),
    );

    let attr_bytes = attrs.serialize();
    // msg_len covers protocol_version(4) + change_date(4) + attributes
    let msg_len: u32 = 8 + attr_bytes.len() as u32;

    let mut buf = Vec::with_capacity(8 + msg_len as usize);
    buf.extend_from_slice(&LOGIN_MAGIC.to_le_bytes());
    buf.extend_from_slice(&msg_len.to_le_bytes());
    buf.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    buf.extend_from_slice(&CHANGE_DATE.to_le_bytes());
    buf.extend_from_slice(&attr_bytes);
    buf
}

/// Build the second-phase authentication message containing encrypted password and keys.
///
/// Sends CMD_SET_ATTRIBUTES with the RSA-encrypted password and ChaCha20 keys.
/// Uses Exasol's custom RSA encoding: interleave data with random phrase, then
/// encrypt in 64-byte blocks via raw RSA (no PKCS#1 padding).
pub fn build_auth_message(
    password: &str,
    public_key_data: &[u8],
    random_phrase: &[u8],
    serial: &SerialCounter,
    use_chacha20: bool,
) -> Result<AuthMessage, TransportError> {
    let (n, e) = parse_rsa_public_key(public_key_data)?;

    // Pad password: [password bytes][0x00][random padding] to match phrase length
    let pwd_bytes = password.as_bytes();
    let padded_pwd = if pwd_bytes.len() + 2 < random_phrase.len() {
        let padding_len = random_phrase.len() - pwd_bytes.len() - 2;
        let rng = SystemRandom::new();
        let mut padding = vec![0u8; padding_len];
        rng.fill(&mut padding)
            .map_err(|_| TransportError::ProtocolError("RNG failure".into()))?;
        let mut padded = Vec::with_capacity(random_phrase.len() - 1);
        padded.extend_from_slice(pwd_bytes);
        padded.push(0x00); // null terminator
        padded.extend_from_slice(&padding);
        padded
    } else {
        pwd_bytes.to_vec()
    };

    let encrypted_password = exasol_encode_pwd(&padded_pwd, random_phrase, &n, &e)?;

    let mut attrs = AttributeSet::new();
    attrs.add(
        ATTR_ENCODED_PASSWORD,
        AttributeValue::Binary(encrypted_password),
    );

    let (send_key, recv_key) = if use_chacha20 {
        let (sk, rk) = ChaCha20Encryptor::generate_keys();
        let encrypted_send_key = exasol_encode_pwd(&sk, random_phrase, &n, &e)?;
        let encrypted_recv_key = exasol_encode_pwd(&rk, random_phrase, &n, &e)?;
        attrs.add(
            ATTR_CLIENT_SEND_KEY,
            AttributeValue::Binary(encrypted_send_key),
        );
        attrs.add(
            ATTR_CLIENT_RECEIVE_KEY,
            AttributeValue::Binary(encrypted_recv_key),
        );
        attrs.add(
            ATTR_CLIENT_KEYS_LEN,
            AttributeValue::Int32(CHACHA20_KEY_LEN as i32),
        );
        (sk, rk)
    } else {
        (Vec::new(), Vec::new())
    };

    let attr_bytes = attrs.serialize();
    let header = MessageHeader::new(
        CMD_SET_ATTRIBUTES,
        serial.next(),
        attrs.num_attributes(),
        attr_bytes.len() as u32,
        0,
    );
    let header_bytes = header.serialize();

    let mut message = Vec::with_capacity(HEADER_SIZE + attr_bytes.len());
    message.extend_from_slice(&header_bytes);
    message.extend_from_slice(&attr_bytes);

    Ok(AuthMessage {
        wire_bytes: message,
        send_key,
        recv_key,
    })
}

/// Parse an RSA public key into (n, e).
///
/// Supports two formats:
/// 1. **PKCS#1 DER** (starts with 0x30): standard ASN.1 SEQUENCE(INTEGER, INTEGER)
/// 2. **Raw Exasol native format**: `[exponent: k/2 bytes BE] [modulus: k/2 bytes BE]`
///    where the exponent is zero-padded to half the total key size.
fn parse_rsa_public_key(key_data: &[u8]) -> Result<(BigUint, BigUint), TransportError> {
    if key_data.first() == Some(&0x30) {
        // PKCS#1 DER format
        parse_rsa_public_key_pkcs1_der(key_data)
    } else {
        // Raw Exasol native format: first half = exponent (BE, zero-padded), second half = modulus (BE)
        if key_data.len() < 4 || !key_data.len().is_multiple_of(2) {
            return Err(TransportError::ProtocolError(format!(
                "Invalid raw RSA key: unexpected length {}",
                key_data.len()
            )));
        }
        let half = key_data.len() / 2;
        let e = BigUint::from_bytes_be(&key_data[..half]);
        let n = BigUint::from_bytes_be(&key_data[half..]);
        if n.bits() == 0 || e.bits() == 0 {
            return Err(TransportError::ProtocolError(
                "Invalid raw RSA key: zero modulus or exponent".into(),
            ));
        }
        Ok((n, e))
    }
}

fn parse_rsa_public_key_pkcs1_der(der: &[u8]) -> Result<(BigUint, BigUint), TransportError> {
    let mut pos = 0;

    if der.get(pos) != Some(&0x30) {
        return Err(TransportError::ProtocolError(
            "Invalid DER: expected SEQUENCE".into(),
        ));
    }
    pos += 1;

    let (seq_len, len_bytes) = read_der_length(&der[pos..])?;
    pos += len_bytes;

    let seq_end = pos + seq_len;
    if seq_end > der.len() {
        return Err(TransportError::ProtocolError(
            "Invalid DER: truncated".into(),
        ));
    }

    let (n_bytes, consumed) = read_der_integer(&der[pos..seq_end])?;
    pos += consumed;

    let (e_bytes, consumed) = read_der_integer(&der[pos..seq_end])?;
    pos += consumed;

    if pos != seq_end {
        return Err(TransportError::ProtocolError(
            "Invalid DER: trailing bytes".into(),
        ));
    }

    Ok((
        BigUint::from_bytes_be(n_bytes),
        BigUint::from_bytes_be(e_bytes),
    ))
}

fn read_der_length(data: &[u8]) -> Result<(usize, usize), TransportError> {
    if data.is_empty() {
        return Err(TransportError::ProtocolError(
            "Invalid DER: truncated".into(),
        ));
    }
    let first = data[0];
    if first < 0x80 {
        Ok((first as usize, 1))
    } else {
        let num_bytes = (first & 0x7F) as usize;
        if num_bytes == 0 || num_bytes > 3 || data.len() < 1 + num_bytes {
            return Err(TransportError::ProtocolError(
                "Invalid DER: bad length".into(),
            ));
        }
        let mut len: usize = 0;
        for &b in &data[1..1 + num_bytes] {
            len = (len << 8) | (b as usize);
        }
        Ok((len, 1 + num_bytes))
    }
}

fn read_der_integer(data: &[u8]) -> Result<(&[u8], usize), TransportError> {
    if data.is_empty() || data[0] != 0x02 {
        return Err(TransportError::ProtocolError(
            "Invalid DER: expected INTEGER".into(),
        ));
    }
    let (int_len, len_bytes) = read_der_length(&data[1..])?;
    let header_len = 1 + len_bytes;
    if data.len() < header_len + int_len {
        return Err(TransportError::ProtocolError(
            "Invalid DER: truncated integer".into(),
        ));
    }
    let mut value = &data[header_len..header_len + int_len];
    if value.len() > 1 && value[0] == 0x00 {
        value = &value[1..];
    }
    Ok((value, header_len + int_len))
}

/// Exasol's custom RSA password encoding.
///
/// 1. Null-terminate the data
/// 2. Interleave data bytes with phrase bytes: [data[0], phrase[0], data[1], phrase[1], ...]
/// 3. Encrypt in 64-byte blocks → 128-byte output blocks (raw RSA: plaintext^e mod N)
///
/// The interleaving cycles through the phrase, so a server that sends an empty random
/// phrase is rejected as a protocol violation rather than encoded without it.
fn exasol_encode_pwd(
    data: &[u8],
    phrase: &[u8],
    n: &BigUint,
    e: &BigUint,
) -> Result<Vec<u8>, TransportError> {
    if phrase.is_empty() {
        return Err(TransportError::ProtocolError(
            "Cannot encode password: server-supplied random phrase is empty".into(),
        ));
    }

    // Null-terminate the data
    let mut pwd = Vec::with_capacity(data.len() + 1);
    pwd.extend_from_slice(data);
    pwd.push(0x00);
    let pwd_len = pwd.len();
    let phrase_len = phrase.len();

    // Calculate interleaved plaintext size
    let mut encoded_output_len = if pwd_len > phrase_len {
        pwd_len * 2
    } else {
        phrase_len * 2
    };

    // Round up to multiple of RSA_KEY_LENGTH (128)
    let rsa_key_len = 128usize; // Protocol::RSA_KEY_LENGTH
    if encoded_output_len % rsa_key_len != 0 {
        encoded_output_len += rsa_key_len - (encoded_output_len % rsa_key_len);
    }

    // Interleave password and phrase bytes
    let interleaved_len = encoded_output_len; // before doubling
    let mut interleaved = vec![0u8; interleaved_len];
    let iterations = encoded_output_len / 2; // number of pairs (before the *2 in C++)

    // The C++ code does: encodedOutputLen *= 2; for(i=0; i<encodedOutputLen/4; i++)
    // which means iterations = encodedOutputLen/2 (after rounding, before doubling)
    // Each iteration writes 2 bytes: tmp[i*2] = pwd[i%pwdLen], tmp[i*2+1] = phrase[i%phraseLen]
    for i in 0..iterations {
        interleaved[i * 2] = pwd[i % pwd_len];
        interleaved[i * 2 + 1] = phrase[i % phrase_len];
    }

    // Now encoded_output_len doubles for the RSA output
    let rsa_output_len = encoded_output_len * 2;
    let block_input_size = rsa_key_len / 2; // 64 bytes
    let num_blocks = rsa_output_len / rsa_key_len;
    let mut encrypted = vec![0u8; rsa_output_len];

    for i in 0..num_blocks {
        let input_start = i * block_input_size;
        let input_end = input_start + block_input_size;

        // Read 64 bytes as a big-endian integer
        let plaintext = BigUint::from_bytes_be(&interleaved[input_start..input_end]);

        // Raw RSA: ciphertext = plaintext^e mod N
        let ciphertext = plaintext.modpow(e, n);

        // Export as 128-byte big-endian, zero-padded
        let c_bytes = ciphertext.to_bytes_be();
        let output_start = i * rsa_key_len;
        let offset = rsa_key_len - c_bytes.len();
        encrypted[output_start + offset..output_start + offset + c_bytes.len()]
            .copy_from_slice(&c_bytes);
    }

    Ok(encrypted)
}

#[cfg(test)]
mod tests {
    use super::super::attributes::parse_attributes;
    use super::*;

    #[test]
    fn login_packet_starts_with_magic() {
        let packet = build_login_packet("sys");
        let magic = u32::from_le_bytes([packet[0], packet[1], packet[2], packet[3]]);
        assert_eq!(magic, LOGIN_MAGIC);
    }

    #[test]
    fn login_packet_contains_protocol_version() {
        let packet = build_login_packet("sys");
        let version = u32::from_le_bytes([packet[8], packet[9], packet[10], packet[11]]);
        assert_eq!(version, PROTOCOL_VERSION);
    }

    #[test]
    fn login_packet_contains_change_date() {
        let packet = build_login_packet("sys");
        let date = u32::from_le_bytes([packet[12], packet[13], packet[14], packet[15]]);
        assert_eq!(date, CHANGE_DATE);
    }

    /// Number of bytes in each half of a raw Exasol public key.
    const RAW_KEY_HALF: usize = 128;

    /// A raw Exasol-format public key: `[exponent 128B BE][modulus 128B BE]`.
    fn raw_public_key() -> Vec<u8> {
        let mut key = vec![0u8; RAW_KEY_HALF];
        key[RAW_KEY_HALF - 3] = 0x01;
        key[RAW_KEY_HALF - 1] = 0x01;
        key.extend_from_slice(&[0xFFu8; RAW_KEY_HALF]);
        key
    }

    #[test]
    fn encoding_a_password_with_an_empty_random_phrase_is_a_protocol_error() {
        let (n, e) = parse_rsa_public_key(&raw_public_key()).unwrap();

        let err = exasol_encode_pwd(b"secret", &[], &n, &e).unwrap_err();

        match err {
            TransportError::ProtocolError(msg) => assert!(
                msg.contains("random phrase is empty"),
                "unexpected message: {msg}"
            ),
            other => panic!("expected ProtocolError, got {other:?}"),
        }
    }

    /// Every 64-byte input block has a matching 128-byte output block, so the
    /// encryption loop always consumes the whole interleaved buffer.
    #[test]
    fn every_interleaved_block_produces_an_encrypted_output_block() {
        let (n, e) = parse_rsa_public_key(&raw_public_key()).unwrap();
        let phrase: Vec<u8> = (0..200u32).map(|i| (i % 251 + 1) as u8).collect();

        let encoded = exasol_encode_pwd(b"secret", &phrase, &n, &e).unwrap();

        assert_eq!(encoded.len(), 1024);
        for (idx, block) in encoded.chunks(RAW_KEY_HALF).enumerate() {
            assert!(
                block.iter().any(|byte| *byte != 0),
                "output block {idx} was never encrypted"
            );
        }
    }

    #[test]
    fn login_packet_msg_len_is_consistent() {
        let packet = build_login_packet("test_user");
        let msg_len = u32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]]) as usize;
        // Total packet = 8 (magic + msg_len) + msg_len
        assert_eq!(packet.len(), 8 + msg_len);
    }

    #[test]
    fn login_packet_identifies_the_user_and_the_driver() {
        let packet = build_login_packet("test_user");
        let attrs = parse_attributes(&packet[16..], 5).unwrap();

        assert_eq!(
            attrs.get(ATTR_USERNAME),
            Some(&AttributeValue::String("test_user".into()))
        );
        assert_eq!(
            attrs.get(ATTR_CLIENTNAME),
            Some(&AttributeValue::String("exarrow-rs".into()))
        );
        assert_eq!(
            attrs.get(ATTR_DRIVERNAME),
            Some(&AttributeValue::String("exarrow-rs".into()))
        );
        assert_eq!(
            attrs.get(ATTR_CLIENTOS),
            Some(&AttributeValue::String(std::env::consts::OS.to_owned()))
        );
        assert_eq!(
            attrs.get(ATTR_CLIENTVERSION),
            Some(&AttributeValue::String(
                env!("CARGO_PKG_VERSION").to_owned()
            ))
        );
    }

    // --- RSA key parsing ---

    fn protocol_error_message<T: std::fmt::Debug>(result: Result<T, TransportError>) -> String {
        match result {
            Err(TransportError::ProtocolError(msg)) => msg,
            other => panic!("expected ProtocolError, got {other:?}"),
        }
    }

    #[test]
    fn raw_key_splits_into_exponent_then_modulus() {
        let (n, e) = parse_rsa_public_key(&raw_public_key()).unwrap();

        assert_eq!(e, BigUint::from(65_537u32));
        assert_eq!(n, BigUint::from_bytes_be(&[0xFFu8; RAW_KEY_HALF]));
    }

    #[test]
    fn shortest_valid_raw_key_is_four_bytes() {
        let (n, e) = parse_rsa_public_key(&[0x00, 0x03, 0x00, 0x05]).unwrap();

        assert_eq!(e, BigUint::from(3u32));
        assert_eq!(n, BigUint::from(5u32));
    }

    #[test]
    fn raw_key_shorter_than_four_bytes_is_rejected() {
        assert_eq!(
            protocol_error_message(parse_rsa_public_key(&[0x01, 0x02])),
            "Invalid raw RSA key: unexpected length 2"
        );
    }

    #[test]
    fn raw_key_with_odd_length_is_rejected() {
        assert_eq!(
            protocol_error_message(parse_rsa_public_key(&[0x01; 5])),
            "Invalid raw RSA key: unexpected length 5"
        );
    }

    #[test]
    fn raw_key_with_zero_exponent_is_rejected() {
        assert_eq!(
            protocol_error_message(parse_rsa_public_key(&[0x00, 0x00, 0x00, 0x05])),
            "Invalid raw RSA key: zero modulus or exponent"
        );
    }

    #[test]
    fn raw_key_with_zero_modulus_is_rejected() {
        assert_eq!(
            protocol_error_message(parse_rsa_public_key(&[0x00, 0x03, 0x00, 0x00])),
            "Invalid raw RSA key: zero modulus or exponent"
        );
    }

    // --- DER key parsing ---

    fn der_length(len: usize) -> Vec<u8> {
        if len < 0x80 {
            vec![len as u8]
        } else if len <= 0xFF {
            vec![0x81, len as u8]
        } else {
            vec![0x82, (len >> 8) as u8, (len & 0xFF) as u8]
        }
    }

    fn der_tagged(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        out.extend_from_slice(&der_length(body.len()));
        out.extend_from_slice(body);
        out
    }

    fn der_public_key(n_bytes: &[u8], e_bytes: &[u8]) -> Vec<u8> {
        let mut body = der_tagged(0x02, n_bytes);
        body.extend_from_slice(&der_tagged(0x02, e_bytes));
        der_tagged(0x30, &body)
    }

    #[test]
    fn der_key_is_detected_by_its_sequence_tag_and_strips_the_sign_byte() {
        let der = der_public_key(&[0x00, 0x9A, 0xBC], &[0x01, 0x00, 0x01]);
        assert_eq!(der[0], 0x30);

        let (n, e) = parse_rsa_public_key(&der).unwrap();

        assert_eq!(n, BigUint::from(0x9ABCu32));
        assert_eq!(e, BigUint::from(65_537u32));
    }

    #[test]
    fn der_key_with_single_byte_long_form_length_is_parsed() {
        let n_bytes = vec![0x7Fu8; 200];
        let der = der_public_key(&n_bytes, &[0x01, 0x00, 0x01]);

        let (n, e) = parse_rsa_public_key(&der).unwrap();

        assert_eq!(n, BigUint::from_bytes_be(&n_bytes));
        assert_eq!(e, BigUint::from(65_537u32));
    }

    #[test]
    fn der_key_with_two_byte_long_form_length_is_parsed() {
        let n_bytes = vec![0x7Fu8; 300];
        let der = der_public_key(&n_bytes, &[0x01, 0x00, 0x01]);

        let (n, _) = parse_rsa_public_key(&der).unwrap();

        assert_eq!(n, BigUint::from_bytes_be(&n_bytes));
    }

    #[test]
    fn der_without_sequence_tag_is_rejected() {
        assert_eq!(
            protocol_error_message(parse_rsa_public_key_pkcs1_der(&[0x31, 0x00])),
            "Invalid DER: expected SEQUENCE"
        );
    }

    #[test]
    fn der_sequence_longer_than_the_buffer_is_rejected() {
        assert_eq!(
            protocol_error_message(parse_rsa_public_key_pkcs1_der(&[0x30, 0x10, 0x02, 0x01])),
            "Invalid DER: truncated"
        );
    }

    #[test]
    fn der_sequence_with_a_third_element_is_rejected() {
        let mut body = der_tagged(0x02, &[0x01]);
        body.extend_from_slice(&der_tagged(0x02, &[0x01]));
        body.push(0xAA);
        let der = der_tagged(0x30, &body);

        assert_eq!(
            protocol_error_message(parse_rsa_public_key_pkcs1_der(&der)),
            "Invalid DER: trailing bytes"
        );
    }

    #[test]
    fn der_sequence_holding_a_non_integer_is_rejected() {
        let der = der_tagged(0x30, &der_tagged(0x04, &[0x01]));

        assert_eq!(
            protocol_error_message(parse_rsa_public_key_pkcs1_der(&der)),
            "Invalid DER: expected INTEGER"
        );
    }

    #[test]
    fn der_integer_longer_than_the_sequence_is_rejected() {
        let der = der_tagged(0x30, &[0x02, 0x05, 0x01]);

        assert_eq!(
            protocol_error_message(parse_rsa_public_key_pkcs1_der(&der)),
            "Invalid DER: truncated integer"
        );
    }

    #[test]
    fn der_length_reads_the_short_form_from_one_byte() {
        assert_eq!(read_der_length(&[0x05, 0xFF]).unwrap(), (5, 1));
    }

    #[test]
    fn der_length_reads_the_three_byte_long_form() {
        assert_eq!(
            read_der_length(&[0x83, 0x01, 0x00, 0x00]).unwrap(),
            (65_536, 4)
        );
    }

    #[test]
    fn der_length_on_an_empty_buffer_is_rejected() {
        assert_eq!(
            protocol_error_message(read_der_length(&[])),
            "Invalid DER: truncated"
        );
    }

    #[test]
    fn der_length_with_zero_length_bytes_is_rejected() {
        assert_eq!(
            protocol_error_message(read_der_length(&[0x80])),
            "Invalid DER: bad length"
        );
    }

    #[test]
    fn der_length_wider_than_three_bytes_is_rejected() {
        assert_eq!(
            protocol_error_message(read_der_length(&[0x84, 0x01, 0x02, 0x03, 0x04])),
            "Invalid DER: bad length"
        );
    }

    #[test]
    fn der_length_missing_its_length_bytes_is_rejected() {
        assert_eq!(
            protocol_error_message(read_der_length(&[0x82, 0x01])),
            "Invalid DER: bad length"
        );
    }

    #[test]
    fn der_integer_keeps_a_single_zero_byte_value() {
        let (value, consumed) = read_der_integer(&[0x02, 0x01, 0x00]).unwrap();

        assert_eq!(value, &[0x00]);
        assert_eq!(consumed, 3);
    }

    #[test]
    fn der_integer_on_an_empty_buffer_is_rejected() {
        assert_eq!(
            protocol_error_message(read_der_integer(&[])),
            "Invalid DER: expected INTEGER"
        );
    }

    // --- Auth message ---

    fn test_phrase(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251 + 1) as u8).collect()
    }

    fn auth_attributes(auth: &AuthMessage) -> (super::MessageHeader, AttributeSet) {
        let header_bytes: &[u8; HEADER_SIZE] =
            auth.wire_bytes[..HEADER_SIZE].try_into().expect("header");
        let header = MessageHeader::parse(header_bytes).unwrap();
        let attrs =
            parse_attributes(&auth.wire_bytes[HEADER_SIZE..], header.num_attributes).unwrap();
        (header, attrs)
    }

    fn binary_attribute(attrs: &AttributeSet, id: u16) -> Vec<u8> {
        match attrs.get(id) {
            Some(AttributeValue::Binary(bytes)) => bytes.clone(),
            other => panic!("attribute {id} was not binary: {other:?}"),
        }
    }

    #[test]
    fn auth_message_without_chacha20_carries_only_the_encoded_password() {
        let auth = build_auth_message(
            "secret",
            &raw_public_key(),
            &test_phrase(16),
            &SerialCounter::new(),
            false,
        )
        .unwrap();

        assert!(auth.send_key.is_empty());
        assert!(auth.recv_key.is_empty());

        let (header, attrs) = auth_attributes(&auth);
        assert_eq!(header.command, CMD_SET_ATTRIBUTES);
        assert_eq!(header.serial, 1);
        assert_eq!(header.num_attributes, 1);
        assert_eq!(
            header.attribute_data_len as usize,
            auth.wire_bytes.len() - HEADER_SIZE
        );
        assert_eq!(binary_attribute(&attrs, ATTR_ENCODED_PASSWORD).len(), 256);
    }

    #[test]
    fn auth_message_with_chacha20_adds_both_session_keys_and_their_length() {
        let auth = build_auth_message(
            "secret",
            &raw_public_key(),
            &test_phrase(16),
            &SerialCounter::new(),
            true,
        )
        .unwrap();

        assert_eq!(auth.send_key.len(), CHACHA20_KEY_LEN);
        assert_eq!(auth.recv_key.len(), CHACHA20_KEY_LEN);
        assert_ne!(auth.send_key, auth.recv_key);

        let (header, attrs) = auth_attributes(&auth);
        assert_eq!(header.num_attributes, 4);
        assert_eq!(binary_attribute(&attrs, ATTR_ENCODED_PASSWORD).len(), 256);
        assert_eq!(binary_attribute(&attrs, ATTR_CLIENT_SEND_KEY).len(), 256);
        assert_eq!(binary_attribute(&attrs, ATTR_CLIENT_RECEIVE_KEY).len(), 256);
        assert_eq!(
            attrs.get(ATTR_CLIENT_KEYS_LEN),
            Some(&AttributeValue::Int32(CHACHA20_KEY_LEN as i32))
        );
    }

    #[test]
    fn auth_message_pads_a_password_shorter_than_the_random_phrase() {
        let auth = build_auth_message(
            "pw",
            &raw_public_key(),
            &test_phrase(64),
            &SerialCounter::new(),
            false,
        )
        .unwrap();

        let (_, attrs) = auth_attributes(&auth);
        assert_eq!(binary_attribute(&attrs, ATTR_ENCODED_PASSWORD).len(), 256);
    }

    #[test]
    fn auth_message_rejects_an_empty_random_phrase() {
        let result = build_auth_message(
            "secret",
            &raw_public_key(),
            &[],
            &SerialCounter::new(),
            false,
        );

        assert_eq!(
            protocol_error_message(result.map(|auth| auth.wire_bytes)),
            "Cannot encode password: server-supplied random phrase is empty"
        );
    }

    #[test]
    fn auth_message_rejects_an_unparsable_public_key() {
        let result = build_auth_message(
            "secret",
            &[0x01, 0x02],
            &test_phrase(16),
            &SerialCounter::new(),
            false,
        );

        assert_eq!(
            protocol_error_message(result.map(|auth| auth.wire_bytes)),
            "Invalid raw RSA key: unexpected length 2"
        );
    }

    #[test]
    fn encoded_password_length_is_rounded_up_to_whole_rsa_blocks() {
        let (n, e) = parse_rsa_public_key(&raw_public_key()).unwrap();

        assert_eq!(
            exasol_encode_pwd(b"x", &test_phrase(1), &n, &e)
                .unwrap()
                .len(),
            256
        );
        assert_eq!(
            exasol_encode_pwd(b"x", &test_phrase(64), &n, &e)
                .unwrap()
                .len(),
            256
        );
        assert_eq!(
            exasol_encode_pwd(b"x", &test_phrase(65), &n, &e)
                .unwrap()
                .len(),
            512
        );
    }
}
