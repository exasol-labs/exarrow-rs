/// Magic value sent at the start of a login handshake.
pub const LOGIN_MAGIC: u32 = 0x01121201;

/// Maximum protocol version we support (matches Exasol 2025.1).
pub const PROTOCOL_VERSION: u32 = 21;

/// Minimum protocol version (ChaCha20 required, RC4 deprecated).
pub const MIN_PROTOCOL_VERSION: u32 = 14;

/// Hardcoded change date from the C++ driver.
pub const CHANGE_DATE: u32 = 200131112;

/// Create a prepared statement.
pub const CMD_CREATE_PREPARED: u8 = 10;

/// Execute a prepared statement.
pub const CMD_EXECUTE_PREPARED: u8 = 11;

/// Execute SQL directly.
pub const CMD_EXECUTE: u8 = 12;

/// Close a result set handle.
pub const CMD_CLOSE_RESULTSET: u8 = 13;

/// Close a prepared statement handle.
pub const CMD_CLOSE_PREPARED: u8 = 18;

/// Disconnect from the server.
pub const CMD_DISCONNECT: u8 = 32;

/// Get session attributes.
pub const CMD_GET_ATTRIBUTES: u8 = 34;

/// Set session attributes.
pub const CMD_SET_ATTRIBUTES: u8 = 35;

/// Fetch rows (v2, with binary column data).
pub const CMD_FETCH2: u8 = 36;

/// Response: row count (DML result).
pub const R_ROW_COUNT: i8 = 0;

/// Response: result set with data.
pub const R_RESULT_SET: i8 = 1;

/// Response: handle only (no inline data).
pub const R_HANDLE: i8 = 2;

/// Response: column count only.
pub const R_COLUMN_COUNT: i8 = 3;

/// Response: warning message.
pub const R_WARNING: i8 = 4;

/// Response: query is still executing.
pub const R_STILL_EXECUTING: i8 = 5;

/// Response: more rows available for fetch.
pub const R_MORE_ROWS: i8 = 6;

/// Response: exception/error.
pub const R_EXCEPTION: i8 = -1;

/// Response: empty result (no data, no error).
pub const R_EMPTY: i8 = -2;

/// Handle value indicating a small (inline) result set.
pub const SMALL_RESULTSET: i32 = -3;

/// Handle value used for parameter descriptions.
pub const PARAMETER_DESCRIPTION: i32 = -5;

pub const ATTR_USERNAME: u16 = 1;
pub const ATTR_CLIENTNAME: u16 = 2;
pub const ATTR_CLIENTOS: u16 = 3;
pub const ATTR_DRIVERNAME: u16 = 4;
pub const ATTR_SESSIONID: u16 = 6;
pub const ATTR_AUTOCOMMIT: u16 = 7;
pub const ATTR_CLIENTVERSION: u16 = 10;
pub const ATTR_TRANSACTION_STATE: u16 = 17;
pub const ATTR_PROTOCOL_VERSION: u16 = 19;
pub const ATTR_DATA_MESSAGE_SIZE: u16 = 26;
pub const ATTR_PUBLIC_KEY: u16 = 32;
pub const ATTR_RANDOM_PHRASE: u16 = 33;
pub const ATTR_ENCODED_PASSWORD: u16 = 34;
pub const ATTR_QUERY_TIMEOUT: u16 = 35;
pub const ATTR_TIMEZONE: u16 = 47;
pub const ATTR_TSUTC_ENABLED: u16 = 51;
pub const ATTR_QUERY_CACHE_ACCESS: u16 = 52;
pub const ATTR_CLIENT_RECEIVE_KEY: u16 = 53;
pub const ATTR_CLIENT_SEND_KEY: u16 = 54;
pub const ATTR_SNAPSHOT_TRANSACTIONS_ENABLED: u16 = 55;
pub const ATTR_CLIENT_KEYS_LEN: u16 = 56;
pub const ATTR_ENCRYPTION_REQUIRED: u16 = 57;

pub const ATTR_RELEASE_VERSION: u16 = 8;
pub const ATTR_DATABASE_NAME: u16 = 37;
pub const ATTR_PRODUCT_NAME: u16 = 38;

// Command-level pseudo-attribute IDs used by our attribute-based encoding
// for SQL text, handles, and row counts in CMD_EXECUTE/CMD_FETCH messages.
// NOTE: These are NOT in protocolattributedecl.h. The C++ driver sends SQL text
// as raw payload after attributes. We use attribute encoding for simplicity.
// TODO: Refactor to send SQL as raw payload like the C++ driver.
pub const ATTR_SQL_TEXT: u16 = 200;
pub const ATTR_RESULT_SET_HANDLE: u16 = 201;
pub const ATTR_STATEMENT_HANDLE: u16 = 202;
pub const ATTR_NUM_ROWS: u16 = 203;

/// Wire type: small integer (i32).
pub const T_SMALLINT: u32 = 4;

/// Wire type: integer (i64).
pub const T_INTEGER: u32 = 5;

/// Wire type: decimal.
pub const T_DECIMAL: u32 = 6;

/// Wire type: real (f32).
pub const T_REAL: u32 = 7;

/// Wire type: double (f64).
pub const T_DOUBLE: u32 = 8;

/// Wire type: fixed-length char.
pub const T_CHAR: u32 = 10;

/// Wire type: date.
pub const T_DATE: u32 = 14;

/// Wire type: binary.
pub const T_BINARY: u32 = 15;

/// Wire type: interval year-to-month.
pub const T_INTERVAL_YEAR: u32 = 16;

/// Wire type: interval day-to-second.
pub const T_INTERVAL_DAY: u32 = 17;

/// Wire type: timestamp.
pub const T_TIMESTAMP: u32 = 21;

/// Wire type: boolean.
pub const T_BOOLEAN: u32 = 9;

/// Wire type: geometry.
pub const T_GEOMETRY: u32 = 123;

/// Wire type: timestamp with local time zone.
pub const T_TIMESTAMP_LOCAL_TZ: u32 = 124;

/// Wire type: timestamp with UTC.
pub const T_TIMESTAMP_UTC: u32 = 125;

/// Wire type: hashtype.
pub const T_HASHTYPE: u32 = 126;

/// Wire type: small decimal.
pub const T_SMALLDECIMAL: u32 = 63;

/// Wire type: big decimal.
pub const T_BIGDECIMAL: u32 = 64;

/// VCFlag: is varchar.
pub const IS_VARCHAR: u8 = 0x80;

/// VCFlag: is UTF-8.
pub const IS_UTF8: u8 = 0x01;

/// Size of the binary message header in bytes.
pub const HEADER_SIZE: usize = 21;

/// Maximum data message size (64 MB).
pub const MAX_DATA_MESSAGE_SIZE: u32 = 64 * 1024 * 1024;

/// ChaCha20 key length in bytes.
pub const CHACHA20_KEY_LEN: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_magic_matches_protocol_spec() {
        assert_eq!(LOGIN_MAGIC, 0x01121201);
    }

    #[test]
    fn header_size_is_21_bytes() {
        assert_eq!(HEADER_SIZE, 21);
    }

    #[test]
    fn max_data_message_size_is_64mb() {
        assert_eq!(MAX_DATA_MESSAGE_SIZE, 64 * 1024 * 1024);
    }

    #[test]
    fn chacha20_key_len_is_32_bytes() {
        assert_eq!(CHACHA20_KEY_LEN, 32);
    }

    #[test]
    fn protocol_version_range_is_valid() {
        const { assert!(MIN_PROTOCOL_VERSION <= PROTOCOL_VERSION) };
        assert_eq!(MIN_PROTOCOL_VERSION, 14);
        assert_eq!(PROTOCOL_VERSION, 21);
    }

    #[test]
    fn exception_result_type_is_negative() {
        assert_eq!(R_EXCEPTION, -1);
        assert_eq!(R_EMPTY, -2);
    }

    #[test]
    fn special_resultset_handles_are_negative() {
        assert_eq!(SMALL_RESULTSET, -3);
        assert_eq!(PARAMETER_DESCRIPTION, -5);
    }
}
