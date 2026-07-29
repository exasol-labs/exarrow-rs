use crate::error::TransportError;

use super::constants::{
    ATTR_AUTOCOMMIT, ATTR_CLIENT_KEYS_LEN, ATTR_CLIENT_RECEIVE_KEY, ATTR_CLIENT_SEND_KEY,
    ATTR_DATA_MESSAGE_SIZE, ATTR_ENCODED_PASSWORD, ATTR_ENCRYPTION_REQUIRED, ATTR_NUM_ROWS,
    ATTR_PROTOCOL_VERSION, ATTR_PUBLIC_KEY, ATTR_QUERY_CACHE_ACCESS, ATTR_QUERY_TIMEOUT,
    ATTR_RANDOM_PHRASE, ATTR_RESULT_SET_HANDLE, ATTR_SESSIONID, ATTR_SNAPSHOT_TRANSACTIONS_ENABLED,
    ATTR_STATEMENT_HANDLE, ATTR_TRANSACTION_STATE, ATTR_TSUTC_ENABLED,
};

/// A single attribute value in the native binary protocol.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeValue {
    String(String),
    Binary(Vec<u8>),
    Int32(i32),
    Int64(i64),
    Bool(bool),
}

/// A key-value pair representing one protocol attribute.
#[derive(Debug, Clone)]
pub struct Attribute {
    pub id: u16,
    pub value: AttributeValue,
}

/// A collection of protocol attributes with binary serialization support.
#[derive(Debug, Clone)]
pub struct AttributeSet {
    attrs: Vec<Attribute>,
}

impl AttributeSet {
    pub fn new() -> Self {
        Self { attrs: Vec::new() }
    }

    /// Add an attribute to the set.
    pub fn add(&mut self, id: u16, value: AttributeValue) {
        self.attrs.push(Attribute { id, value });
    }

    /// Look up an attribute by ID.
    pub fn get(&self, id: u16) -> Option<&AttributeValue> {
        self.attrs.iter().find(|a| a.id == id).map(|a| &a.value)
    }

    /// Number of attributes in the set.
    pub fn num_attributes(&self) -> u32 {
        self.attrs.len() as u32
    }

    /// Total byte length when serialized (attribute data only, no outer framing).
    pub fn data_len(&self) -> u32 {
        self.attrs.iter().map(serialized_len).sum::<usize>() as u32
    }

    /// Serialize all attributes into a binary buffer.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.data_len() as usize);
        for attr in &self.attrs {
            serialize_attribute(attr, &mut buf);
        }
        buf
    }
}

impl Default for AttributeSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a binary buffer into an `AttributeSet`.
///
/// The caller must supply the expected number of attributes so we know
/// when to stop parsing. Each attribute is read according to its type tag
/// embedded in the protocol (the type is inferred from the attribute ID).
pub fn parse_attributes(data: &[u8], count: u32) -> Result<AttributeSet, TransportError> {
    let mut set = AttributeSet::new();
    let mut offset = 0;

    for _ in 0..count {
        let id = read_attribute_id(data, &mut offset)?;
        let value = read_attribute_value(data, &mut offset, id)?;
        set.add(id, value);
    }
    Ok(set)
}

// --- private helpers ---

fn read_attribute_id(data: &[u8], offset: &mut usize) -> Result<u16, TransportError> {
    if *offset + 2 > data.len() {
        return Err(TransportError::ProtocolError(
            "attribute data truncated: missing attr_id".into(),
        ));
    }
    let id = u16::from_le_bytes([data[*offset], data[*offset + 1]]);
    *offset += 2;
    Ok(id)
}

fn read_attribute_value(
    data: &[u8],
    offset: &mut usize,
    id: u16,
) -> Result<AttributeValue, TransportError> {
    match attribute_type(id) {
        AttrType::Str => read_string(data, offset, id),
        AttrType::Bin => Ok(AttributeValue::Binary(
            read_length_prefixed(data, offset)?.to_vec(),
        )),
        AttrType::I32 => read_int32(data, offset),
        AttrType::I64 => read_int64(data, offset),
        AttrType::Bool => read_bool(data, offset),
    }
}

fn read_length_prefixed<'a>(
    data: &'a [u8],
    offset: &mut usize,
) -> Result<&'a [u8], TransportError> {
    if *offset + 4 > data.len() {
        return Err(TransportError::ProtocolError(
            "attribute data truncated: missing length".into(),
        ));
    }
    let len = u32::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]) as usize;
    *offset += 4;
    if *offset + len > data.len() {
        return Err(TransportError::ProtocolError(format!(
            "attribute data truncated: need {} bytes, have {}",
            len,
            data.len() - *offset
        )));
    }
    let bytes = &data[*offset..*offset + len];
    *offset += len;
    Ok(bytes)
}

fn read_string(data: &[u8], offset: &mut usize, id: u16) -> Result<AttributeValue, TransportError> {
    let bytes = read_length_prefixed(data, offset)?;
    let s = std::str::from_utf8(bytes).map_err(|e| {
        TransportError::ProtocolError(format!("invalid UTF-8 in attribute {id}: {e}"))
    })?;
    Ok(AttributeValue::String(s.to_owned()))
}

fn read_int32(data: &[u8], offset: &mut usize) -> Result<AttributeValue, TransportError> {
    if *offset + 4 > data.len() {
        return Err(TransportError::ProtocolError(
            "attribute data truncated: missing i32".into(),
        ));
    }
    let v = i32::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset += 4;
    Ok(AttributeValue::Int32(v))
}

fn read_int64(data: &[u8], offset: &mut usize) -> Result<AttributeValue, TransportError> {
    if *offset + 8 > data.len() {
        return Err(TransportError::ProtocolError(
            "attribute data truncated: missing i64".into(),
        ));
    }
    let v = i64::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
        data[*offset + 4],
        data[*offset + 5],
        data[*offset + 6],
        data[*offset + 7],
    ]);
    *offset += 8;
    Ok(AttributeValue::Int64(v))
}

fn read_bool(data: &[u8], offset: &mut usize) -> Result<AttributeValue, TransportError> {
    if *offset >= data.len() {
        return Err(TransportError::ProtocolError(
            "attribute data truncated: missing bool".into(),
        ));
    }
    let v = data[*offset] != 0;
    *offset += 1;
    Ok(AttributeValue::Bool(v))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttrType {
    Str,
    Bin,
    I32,
    I64,
    Bool,
}

/// Map an attribute ID to its wire type.
///
/// Boolean attributes: AUTOCOMMIT(7), TSUTC_ENABLED(51), ENCRYPTION_REQUIRED(56),
///                     SQL_TEXT(57).
/// I32 attributes: SESSIONID(6), TRANSACTION_STATE(17), PROTOCOL_VERSION(19),
///                 DATA_MESSAGE_SIZE(26), QUERY_TIMEOUT(35), QUERY_CACHE_ACCESS(52),
///                 CLIENT_KEYS_LEN(55).
/// I64 attributes: RESULT_SET_HANDLE(58), STATEMENT_HANDLE(59), NUM_ROWS(60).
/// Binary attributes: PUBLIC_KEY(32), RANDOM_PHRASE(33), ENCODED_PASSWORD(34),
///                    CLIENT_RECEIVE_KEY(53), CLIENT_SEND_KEY(54).
/// Everything else is a string.
fn attribute_type(id: u16) -> AttrType {
    match id {
        // T_boolean attributes (1 byte)
        ATTR_AUTOCOMMIT              // 7
        | ATTR_TSUTC_ENABLED         // 51
        | ATTR_ENCRYPTION_REQUIRED   // 57
        | ATTR_SNAPSHOT_TRANSACTIONS_ENABLED // 55
        | ATTR_TRANSACTION_STATE     // 17 (T_byte)
        | 15  // ATTR_COMMIT_ON_EXIT
        | 41  // ATTR_CONSTRAINT_ENABLED
        | 45  // ATTR_PROFILING_ENABLED
        | 50  // ATTR_NICE_VALUE
        | 61  // ATTR_SUPER_CONNECTION
        | 62  // ATTR_COMPRESSION_ENABLED
        => AttrType::Bool,

        // T_smallint attributes (i32, 4 bytes)
        ATTR_PROTOCOL_VERSION        // 19
        | ATTR_QUERY_TIMEOUT         // 35
        | ATTR_QUERY_CACHE_ACCESS    // 52
        | ATTR_CLIENT_KEYS_LEN      // 56
        | 16  // ATTR_PREPARED_PARAMCOUNT
        | 36  // ATTR_PACKET_PART_NUMBER
        | 40  // ATTR_MAX_IDENTIFIER_LENGTH
        | 44  // ATTR_FEEDBACK_INTERVAL
        | 46  // ATTR_FIRST_DAY_OF_WEEK
        | 60  // ATTR_TIMESTAMP_ARITHMETIC_BEHAVIOR
        | 69  // ATTR_ST_MAX_DECIMAL_DIGITS
        | 72  // ATTR_IDLE_TIMEOUT
        | 73  // ATTR_TRANSACTION_SNAPSHOT_MODE
        => AttrType::I32,

        // T_integer attributes (i64, 8 bytes)
        ATTR_SESSIONID               // 6
        | ATTR_DATA_MESSAGE_SIZE     // 26
        | ATTR_RESULT_SET_HANDLE     // 201 (command pseudo-attr)
        | ATTR_STATEMENT_HANDLE      // 202 (command pseudo-attr)
        | ATTR_NUM_ROWS              // 203 (command pseudo-attr)
        | 18  // ATTR_PARALLEL_TOKEN
        | 24  // ATTR_MAX_STATEMENT_LENGTH
        | 25  // ATTR_GENERIC_MESSAGE_SIZE
        | 42  // ATTR_MAX_VARCHAR_LENGTH
        | 43  // ATTR_UNKNOWN_TYPE_MAX_LEN
        | 68  // ATTR_SESSION_TEMP_DB_RAM_LIMIT
        | 70  // ATTR_RESULT_MAX_ROWS
        => AttrType::I64,

        // T_binary attributes (length-prefixed)
        ATTR_CLIENT_RECEIVE_KEY
        | ATTR_CLIENT_SEND_KEY
        | ATTR_PUBLIC_KEY
        | ATTR_ENCODED_PASSWORD
        | ATTR_RANDOM_PHRASE
        | 65  // ATTR_KERBEROS_TICKET
        | 66  // ATTR_KERBEROS_TOKEN
        => AttrType::Bin,

        // Everything else is T_char (string, length-prefixed)
        _ => AttrType::Str,
    }
}

fn serialized_len(attr: &Attribute) -> usize {
    // 2 bytes for attr_id + type-specific payload
    2 + match &attr.value {
        AttributeValue::String(s) => 4 + s.len(),
        AttributeValue::Binary(b) => 4 + b.len(),
        AttributeValue::Int32(_) => 4,
        AttributeValue::Int64(_) => 8,
        AttributeValue::Bool(_) => 1,
    }
}

fn serialize_attribute(attr: &Attribute, buf: &mut Vec<u8>) {
    buf.extend_from_slice(&attr.id.to_le_bytes());
    match &attr.value {
        AttributeValue::String(s) => {
            buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
            buf.extend_from_slice(s.as_bytes());
        }
        AttributeValue::Binary(b) => {
            buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
            buf.extend_from_slice(b);
        }
        AttributeValue::Int32(v) => {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        AttributeValue::Int64(v) => {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        AttributeValue::Bool(v) => {
            buf.push(if *v { 1 } else { 0 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::constants::ATTR_USERNAME;
    use super::*;

    #[test]
    fn test_string_attribute() {
        let mut set = AttributeSet::new();
        set.add(
            super::super::constants::ATTR_USERNAME,
            AttributeValue::String("sys".into()),
        );

        let bytes = set.serialize();
        let parsed = parse_attributes(&bytes, 1).unwrap();
        assert_eq!(
            parsed.get(super::super::constants::ATTR_USERNAME),
            Some(&AttributeValue::String("sys".into()))
        );
    }

    #[test]
    fn test_binary_attribute() {
        let key = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let mut set = AttributeSet::new();
        set.add(
            super::super::constants::ATTR_CLIENT_SEND_KEY,
            AttributeValue::Binary(key.clone()),
        );

        let bytes = set.serialize();
        let parsed = parse_attributes(&bytes, 1).unwrap();
        assert_eq!(
            parsed.get(super::super::constants::ATTR_CLIENT_SEND_KEY),
            Some(&AttributeValue::Binary(key))
        );
    }

    #[test]
    fn test_int32_attribute() {
        let mut set = AttributeSet::new();
        set.add(
            super::super::constants::ATTR_PROTOCOL_VERSION,
            AttributeValue::Int32(21),
        );

        let bytes = set.serialize();
        let parsed = parse_attributes(&bytes, 1).unwrap();
        assert_eq!(
            parsed.get(super::super::constants::ATTR_PROTOCOL_VERSION),
            Some(&AttributeValue::Int32(21))
        );
    }

    #[test]
    fn test_empty_attribute_set() {
        let set = AttributeSet::new();
        assert_eq!(set.num_attributes(), 0);
        assert_eq!(set.data_len(), 0);

        let bytes = set.serialize();
        assert!(bytes.is_empty());

        let parsed = parse_attributes(&bytes, 0).unwrap();
        assert_eq!(parsed.num_attributes(), 0);
    }

    /// Parse and unwrap the protocol-error message, failing the test on success.
    fn parse_error(data: &[u8], count: u32) -> String {
        match parse_attributes(data, count) {
            Err(TransportError::ProtocolError(msg)) => msg,
            other => panic!("expected ProtocolError, got {other:?}"),
        }
    }

    #[test]
    fn default_attribute_set_is_empty() {
        let set = AttributeSet::default();
        assert_eq!(set.num_attributes(), 0);
        assert_eq!(set.data_len(), 0);
        assert!(set.serialize().is_empty());
    }

    #[test]
    fn get_returns_none_for_absent_attribute() {
        let mut set = AttributeSet::new();
        set.add(ATTR_AUTOCOMMIT, AttributeValue::Bool(false));
        assert_eq!(set.get(ATTR_PROTOCOL_VERSION), None);
        assert_eq!(set.get(ATTR_AUTOCOMMIT), Some(&AttributeValue::Bool(false)));
    }

    #[test]
    fn single_byte_buffer_is_too_short_for_an_attribute_id() {
        assert_eq!(
            parse_error(&[0x01], 1),
            "attribute data truncated: missing attr_id"
        );
    }

    #[test]
    fn string_attribute_without_full_length_prefix_is_truncated() {
        let mut data = ATTR_USERNAME.to_le_bytes().to_vec();
        data.extend_from_slice(&[0x00, 0x00]);
        assert_eq!(
            parse_error(&data, 1),
            "attribute data truncated: missing length"
        );
    }

    #[test]
    fn string_attribute_shorter_than_its_length_prefix_reports_both_sizes() {
        let mut data = ATTR_USERNAME.to_le_bytes().to_vec();
        data.extend_from_slice(&5u32.to_le_bytes());
        data.extend_from_slice(b"ab");
        assert_eq!(
            parse_error(&data, 1),
            "attribute data truncated: need 5 bytes, have 2"
        );
    }

    #[test]
    fn binary_attribute_shorter_than_its_length_prefix_reports_both_sizes() {
        let mut data = ATTR_PUBLIC_KEY.to_le_bytes().to_vec();
        data.extend_from_slice(&4u32.to_le_bytes());
        data.push(0xAB);
        assert_eq!(
            parse_error(&data, 1),
            "attribute data truncated: need 4 bytes, have 1"
        );
    }

    #[test]
    fn int32_attribute_with_two_payload_bytes_is_truncated() {
        let mut data = ATTR_PROTOCOL_VERSION.to_le_bytes().to_vec();
        data.extend_from_slice(&[0x00, 0x00]);
        assert_eq!(
            parse_error(&data, 1),
            "attribute data truncated: missing i32"
        );
    }

    #[test]
    fn int64_attribute_with_four_payload_bytes_is_truncated() {
        let mut data = ATTR_SESSIONID.to_le_bytes().to_vec();
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        assert_eq!(
            parse_error(&data, 1),
            "attribute data truncated: missing i64"
        );
    }

    #[test]
    fn bool_attribute_without_payload_byte_is_truncated() {
        let data = ATTR_AUTOCOMMIT.to_le_bytes().to_vec();
        assert_eq!(
            parse_error(&data, 1),
            "attribute data truncated: missing bool"
        );
    }

    #[test]
    fn non_utf8_string_attribute_names_the_offending_attribute_id() {
        let mut data = ATTR_USERNAME.to_le_bytes().to_vec();
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&[0xFF, 0xFE]);

        let msg = parse_error(&data, 1);
        assert!(
            msg.starts_with(&format!("invalid UTF-8 in attribute {ATTR_USERNAME}: ")),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn bool_attribute_round_trips_false() {
        let mut set = AttributeSet::new();
        set.add(ATTR_AUTOCOMMIT, AttributeValue::Bool(false));

        let parsed = parse_attributes(&set.serialize(), 1).unwrap();
        assert_eq!(
            parsed.get(ATTR_AUTOCOMMIT),
            Some(&AttributeValue::Bool(false))
        );
    }

    #[test]
    fn unknown_attribute_id_is_parsed_as_a_string() {
        const UNKNOWN_ATTR_ID: u16 = 4_242;
        let mut set = AttributeSet::new();
        set.add(UNKNOWN_ATTR_ID, AttributeValue::String("free text".into()));

        let parsed = parse_attributes(&set.serialize(), 1).unwrap();
        assert_eq!(
            parsed.get(UNKNOWN_ATTR_ID),
            Some(&AttributeValue::String("free text".into()))
        );
    }

    #[test]
    fn test_attribute_roundtrip() {
        let mut set = AttributeSet::new();
        set.add(
            super::super::constants::ATTR_USERNAME,
            AttributeValue::String("admin".into()),
        );
        set.add(
            super::super::constants::ATTR_AUTOCOMMIT,
            AttributeValue::Bool(true),
        );
        set.add(
            super::super::constants::ATTR_PROTOCOL_VERSION,
            AttributeValue::Int32(21),
        );
        set.add(
            super::super::constants::ATTR_NUM_ROWS,
            AttributeValue::Int64(42),
        );
        set.add(
            super::super::constants::ATTR_CLIENT_RECEIVE_KEY,
            AttributeValue::Binary(vec![1, 2, 3, 4]),
        );

        let bytes = set.serialize();
        assert_eq!(bytes.len() as u32, set.data_len());

        let parsed = parse_attributes(&bytes, set.num_attributes()).unwrap();
        assert_eq!(parsed.num_attributes(), 5);

        assert_eq!(
            parsed.get(super::super::constants::ATTR_USERNAME),
            Some(&AttributeValue::String("admin".into()))
        );
        assert_eq!(
            parsed.get(super::super::constants::ATTR_AUTOCOMMIT),
            Some(&AttributeValue::Bool(true))
        );
        assert_eq!(
            parsed.get(super::super::constants::ATTR_PROTOCOL_VERSION),
            Some(&AttributeValue::Int32(21))
        );
        assert_eq!(
            parsed.get(super::super::constants::ATTR_NUM_ROWS),
            Some(&AttributeValue::Int64(42))
        );
        assert_eq!(
            parsed.get(super::super::constants::ATTR_CLIENT_RECEIVE_KEY),
            Some(&AttributeValue::Binary(vec![1, 2, 3, 4]))
        );
    }
}
