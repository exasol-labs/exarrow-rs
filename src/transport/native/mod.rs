pub mod arrow_builder;
pub mod attributes;
pub mod constants;
pub mod encryption;
pub mod framing;
pub mod handshake;
pub mod result_parser;

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use crate::error::TransportError;

use super::deadline::SetupStep;
use super::lifecycle::{self, ConnectionLifecycle, ConnectionState, LifecycleSteps};
use super::messages::{ColumnInfo, ResultData, ResultPayload, ResultSetHandle, SessionInfo};
use super::protocol::{
    ConnectionParams, Credentials, PreparedStatementHandle, QueryResult, TransportProtocol,
};
use super::tls;

use self::attributes::{AttributeSet, AttributeValue};
use self::constants::{
    ATTR_AUTOCOMMIT, ATTR_CURRENT_SCHEMA, ATTR_DATABASE_NAME, ATTR_DATA_MESSAGE_SIZE,
    ATTR_PRODUCT_NAME, ATTR_PROTOCOL_VERSION, ATTR_PUBLIC_KEY, ATTR_QUERY_TIMEOUT,
    ATTR_RANDOM_PHRASE, ATTR_RELEASE_VERSION, ATTR_SESSIONID, ATTR_TIMEZONE, CMD_CLOSE_PREPARED,
    CMD_CLOSE_RESULTSET, CMD_CREATE_PREPARED, CMD_DISCONNECT, CMD_EXECUTE, CMD_EXECUTE_PREPARED,
    CMD_FETCH2, CMD_GET_ATTRIBUTES, CMD_SET_ATTRIBUTES, HEADER_SIZE, IS_UTF8, IS_VARCHAR,
    MAX_DATA_MESSAGE_SIZE, PROTOCOL_VERSION, SMALL_RESULTSET, T_BOOLEAN, T_CHAR, T_DATE, T_DECIMAL,
    T_DOUBLE, T_GEOMETRY, T_HASHTYPE, T_INTERVAL_DAY, T_INTERVAL_YEAR, T_TIMESTAMP,
    T_TIMESTAMP_LOCAL_TZ, T_TIMESTAMP_UTC,
};
use self::encryption::ChaCha20Encryptor;
use self::framing::{MessageHeader, SerialCounter};
use self::result_parser::{NativeColumnMeta, NativeResponse, NativeResponseEnvelope};

/// Abstraction over plain TCP or TLS-wrapped TCP stream.
enum NativeStream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl NativeStream {
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), TransportError> {
        match self {
            NativeStream::Plain(s) => {
                s.read_exact(buf)
                    .await
                    .map_err(|e| TransportError::ReceiveError(e.to_string()))?;
                Ok(())
            }
            NativeStream::Tls(s) => {
                s.read_exact(buf)
                    .await
                    .map_err(|e| TransportError::ReceiveError(e.to_string()))?;
                Ok(())
            }
        }
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<(), TransportError> {
        match self {
            NativeStream::Plain(s) => s
                .write_all(buf)
                .await
                .map_err(|e| TransportError::SendError(e.to_string())),
            NativeStream::Tls(s) => s
                .write_all(buf)
                .await
                .map_err(|e| TransportError::SendError(e.to_string())),
        }
    }

    async fn flush(&mut self) -> Result<(), TransportError> {
        match self {
            NativeStream::Plain(s) => s
                .flush()
                .await
                .map_err(|e| TransportError::SendError(e.to_string())),
            NativeStream::Tls(s) => s
                .flush()
                .await
                .map_err(|e| TransportError::SendError(e.to_string())),
        }
    }
}

/// The payload of CMD_EXECUTE_PREPARED holds one table of parameter rows.
const PARAMETER_TABLE_COUNT: i32 = 1;
const IS_TABLE: u8 = 1;

/// Wire layout of one parameter column, shared by every data message of a batch.
struct ParameterColumn {
    name: String,
    wire_type: u32,
    scale: i32,
}

/// A run of consecutive parameter rows and the span of `EncodedParameterRows::bytes` that holds them.
struct RowRange {
    rows: Range<usize>,
    bytes: Range<usize>,
}

/// Parameter rows encoded once, with the ranges that keep each data message within a size limit.
struct EncodedParameterRows {
    columns: Vec<ParameterColumn>,
    bytes: Vec<u8>,
    ranges: Vec<RowRange>,
}

/// Native binary TCP transport for Exasol.
///
/// Communicates using Exasol's native binary protocol over TCP (optionally TLS-wrapped),
/// with ChaCha20 encryption for message payloads after the handshake.
pub struct NativeTcpTransport {
    stream: Option<NativeStream>,
    lifecycle: ConnectionLifecycle,
    serial: SerialCounter,
    encryptor: ChaCha20Encryptor,
    session: Option<SessionInfo>,
    tls_active: bool,
    fetch_positions: HashMap<i32, i64>,
    result_columns: HashMap<i32, Arc<Vec<NativeColumnMeta>>>,
    recv_buf: Vec<u8>,
    current_schema: Option<String>,
}

impl NativeTcpTransport {
    pub fn new() -> Self {
        Self {
            stream: None,
            lifecycle: ConnectionLifecycle::new(),
            serial: SerialCounter::new(),
            encryptor: ChaCha20Encryptor::new(),
            session: None,
            tls_active: false,
            fetch_positions: HashMap::new(),
            result_columns: HashMap::new(),
            recv_buf: Vec::with_capacity(1 << 20),
            current_schema: None,
        }
    }

    fn stream_mut(&mut self) -> Result<&mut NativeStream, TransportError> {
        self.stream
            .as_mut()
            .ok_or_else(|| TransportError::ProtocolError("Not connected".into()))
    }

    /// Send raw bytes to the stream.
    async fn send_raw(&mut self, data: &[u8]) -> Result<(), TransportError> {
        let stream = self.stream_mut()?;
        stream.write_all(data).await?;
        stream.flush().await
    }

    /// Send a command message with attributes and optional extra data payload.
    async fn send_message(
        &mut self,
        command: u8,
        attrs: &AttributeSet,
        extra_data: Option<&[u8]>,
    ) -> Result<(), TransportError> {
        self.send_message_with_serial(command, attrs, extra_data, None)
            .await
    }

    async fn send_message_with_serial(
        &mut self,
        command: u8,
        attrs: &AttributeSet,
        extra_data: Option<&[u8]>,
        serial_override: Option<u32>,
    ) -> Result<(), TransportError> {
        let attr_bytes = attrs.serialize();
        let extra_len = extra_data.map_or(0, |d| d.len());
        let total_payload_len = attr_bytes.len() + extra_len;

        let header = MessageHeader {
            message_length: total_payload_len as u32,
            command,
            serial: serial_override.unwrap_or_else(|| self.serial.next()),
            num_attributes: attrs.num_attributes(),
            attribute_data_len: attr_bytes.len() as u32,
            num_result_parts: if extra_data.is_some() { 1 } else { 0 },
        };

        let header_bytes = header.serialize();

        let mut payload = Vec::with_capacity(total_payload_len);
        payload.extend_from_slice(&attr_bytes);
        if let Some(extra) = extra_data {
            payload.extend_from_slice(extra);
        }

        if self.encryptor.is_active() {
            self.encryptor.encrypt(&mut payload);
        }

        self.send_raw(&header_bytes).await?;
        self.send_raw(&payload).await
    }

    /// Receive a response message into the reusable `recv_buf`, returning the header.
    ///
    /// After this call, `self.recv_buf` contains the decrypted payload bytes.
    /// The buffer grows on demand but never shrinks, amortising allocation cost across fetches.
    /// Every response passes through here, so this also records attribute 22 when the
    /// response carries it.
    async fn receive_into_buf(&mut self) -> Result<MessageHeader, TransportError> {
        let mut hdr = [0u8; HEADER_SIZE];
        self.stream_mut()?.read_exact(&mut hdr).await?;
        let header = MessageHeader::parse(&hdr)?;
        self.recv_buf.clear();
        if header.message_length > 0 {
            let msg_len = header.message_length as usize;
            self.recv_buf.resize(msg_len, 0);
            // Split borrows: access stream and recv_buf through separate field paths
            // so the borrow checker sees them as disjoint.
            let stream = self
                .stream
                .as_mut()
                .ok_or_else(|| TransportError::ProtocolError("Not connected".into()))?;
            stream.read_exact(&mut self.recv_buf).await?;
            if self.encryptor.is_active() {
                self.encryptor.decrypt(&mut self.recv_buf);
            }
        }
        let attrs = Self::response_attributes(&header, &self.recv_buf)?;
        if let Some(schema) = current_schema_change(&attrs) {
            self.current_schema = schema;
        }
        Ok(header)
    }

    /// Receive a response message: header + payload (decrypted if needed).
    async fn receive_message(&mut self) -> Result<(MessageHeader, Vec<u8>), TransportError> {
        let header = self.receive_into_buf().await?;
        Ok((header, self.recv_buf.clone()))
    }

    /// Send a command and receive its response, handling StillExecuting retries.
    /// The response payload is stored in `self.recv_buf`; only the header is returned.
    /// This avoids cloning the payload — callers access `self.recv_buf` directly.
    async fn send_and_fill_buf(
        &mut self,
        command: u8,
        attrs: &AttributeSet,
        extra_data: Option<&[u8]>,
    ) -> Result<MessageHeader, TransportError> {
        let command_serial = self.serial.next();
        self.send_message_with_serial(command, attrs, extra_data, Some(command_serial))
            .await?;
        loop {
            let header = self.receive_into_buf().await?;
            let still_executing = {
                let result_data = Self::result_data_slice(&header, &self.recv_buf);
                matches!(
                    result_parser::parse_response(result_data)?.terminal,
                    NativeResponse::StillExecuting
                )
            };
            if !still_executing {
                return Ok(header);
            }
        }
    }

    /// Send a command and receive its response, handling StillExecuting retries.
    async fn send_and_receive(
        &mut self,
        command: u8,
        attrs: &AttributeSet,
        extra_data: Option<&[u8]>,
    ) -> Result<(MessageHeader, Vec<u8>), TransportError> {
        let header = self.send_and_fill_buf(command, attrs, extra_data).await?;
        Ok((header, self.recv_buf.clone()))
    }

    /// Extract the result-part data from a response, skipping past any attribute data.
    fn extract_result_data(header: &MessageHeader, payload: &[u8]) -> Vec<u8> {
        let attr_len = header.attribute_data_len as usize;
        if attr_len < payload.len() {
            payload[attr_len..].to_vec()
        } else {
            Vec::new()
        }
    }

    fn result_data_slice<'a>(header: &MessageHeader, payload: &'a [u8]) -> &'a [u8] {
        let skip = header.attribute_data_len as usize;
        if skip < payload.len() {
            &payload[skip..]
        } else {
            &[]
        }
    }

    /// Parse the attribute block at the start of a response payload.
    ///
    /// A header that declares no attributes or no attribute bytes carries no block.
    fn response_attributes(
        header: &MessageHeader,
        payload: &[u8],
    ) -> Result<AttributeSet, TransportError> {
        if header.num_attributes == 0 || header.attribute_data_len == 0 {
            return Ok(AttributeSet::new());
        }
        let attr_len = (header.attribute_data_len as usize).min(payload.len());
        attributes::parse_attributes(&payload[..attr_len], header.num_attributes)
    }

    /// Parse the result-part of a response payload and check for exceptions.
    fn check_response(
        header: &MessageHeader,
        payload: &[u8],
    ) -> Result<NativeResponseEnvelope, TransportError> {
        let result_data = Self::extract_result_data(header, payload);
        let response = result_parser::parse_response(&result_data)?;
        if let NativeResponse::Exception {
            ref message,
            ref sql_state,
        } = response.terminal
        {
            return Err(TransportError::ProtocolError(format!(
                "{} (SQL state: {})",
                message, sql_state
            )));
        }
        Ok(response)
    }

    /// Convert native column metadata to the shared ColumnInfo type.
    fn to_column_info(columns: &[NativeColumnMeta]) -> Vec<ColumnInfo> {
        columns
            .iter()
            .map(|c| ColumnInfo {
                name: c.name.clone(),
                data_type: arrow_builder::native_meta_to_data_type(c),
            })
            .collect()
    }

    /// Convert native result to QueryResult, building Arrow RecordBatch directly
    /// from column-major binary wire data without JSON intermediary.
    fn native_result_to_query_result(
        response: NativeResponse,
    ) -> Result<QueryResult, TransportError> {
        match response {
            NativeResponse::ResultSet {
                handle,
                columns,
                batch,
                total_rows,
                rows_received: _,
            } => {
                let col_infos = Self::to_column_info(&columns);

                let record_batch = batch.unwrap_or_else(|| {
                    arrow::record_batch::RecordBatch::new_empty(std::sync::Arc::new(
                        arrow::datatypes::Schema::empty(),
                    ))
                });

                let result_data = ResultData {
                    columns: col_infos,
                    data: ResultPayload::Arrow(record_batch),
                    total_rows,
                };

                let rs_handle = if handle == SMALL_RESULTSET {
                    None
                } else {
                    Some(ResultSetHandle::new(handle))
                };

                Ok(QueryResult::result_set(rs_handle, result_data))
            }
            NativeResponse::RowCount(count) => Ok(QueryResult::row_count(count)),
            NativeResponse::Empty => Ok(QueryResult::row_count(0)),
            other => Err(TransportError::ProtocolError(format!(
                "Unexpected native response where a query result was expected: {other:?}"
            ))),
        }
    }

    /// Build the raw data payload for CMD_EXECUTE_PREPARED.
    ///
    /// Wire format (matching the JDBC driver):
    /// ```text
    /// [handle:4 LE][num_tables:4 LE=1][is_table:1=1][num_columns:4 LE]
    /// [total_rows:8 LE][rows_in_msg:8 LE]
    /// For each column: [name_len:4 LE][name_bytes][type_id:4 LE][type-specific metadata]
    /// For each row, for each column: [null_marker:1] [value (type-specific)]
    /// ```
    fn build_execute_prepared_payload(
        handle: &PreparedStatementHandle,
        parameters: Option<&[Vec<serde_json::Value>]>,
    ) -> Result<Vec<u8>, TransportError> {
        let cols = parameters.unwrap_or(&[]);
        let num_rows = cols.first().map_or(0, Vec::len);
        let columns = Self::parameter_columns(handle, cols);

        let mut buf = Self::payload_prefix(handle, &columns, num_rows);
        Self::write_parameter_rows(&mut buf, cols, &columns, 0..num_rows)?;
        Ok(buf)
    }

    /// Encode the rows of a parameter set once and split them into consecutive row ranges.
    ///
    /// Rows join a range while the whole message of that range stays within `limit`:
    /// the message header, the empty attribute set, the payload prefix with the column
    /// metadata, and the row bytes. A row whose message alone exceeds `limit` forms a
    /// range of its own. Builds no payload: `build_range_payload` does, one range at a time.
    fn split_parameter_rows(
        handle: &PreparedStatementHandle,
        cols: &[Vec<serde_json::Value>],
        limit: usize,
    ) -> Result<EncodedParameterRows, TransportError> {
        let num_rows = cols.first().map_or(0, Vec::len);
        let columns = Self::parameter_columns(handle, cols);

        let message_overhead = HEADER_SIZE
            + Self::execute_prepared_attributes().serialize().len()
            + Self::payload_prefix(handle, &columns, 0).len();

        let mut bytes = Vec::new();
        let mut ranges = Vec::new();
        let (mut first_row, mut first_byte) = (0, 0);
        for row in 0..num_rows {
            let row_start = bytes.len();
            Self::write_parameter_rows(&mut bytes, cols, &columns, row..row + 1)?;
            if row > first_row && message_overhead + (bytes.len() - first_byte) > limit {
                ranges.push(RowRange {
                    rows: first_row..row,
                    bytes: first_byte..row_start,
                });
                (first_row, first_byte) = (row, row_start);
            }
        }
        if num_rows > 0 {
            ranges.push(RowRange {
                rows: first_row..num_rows,
                bytes: first_byte..bytes.len(),
            });
        }

        Ok(EncodedParameterRows {
            columns,
            bytes,
            ranges,
        })
    }

    /// Build the CMD_EXECUTE_PREPARED payload of one range of `encoded` rows.
    fn build_range_payload(
        handle: &PreparedStatementHandle,
        encoded: &EncodedParameterRows,
        range: &RowRange,
    ) -> Vec<u8> {
        let mut buf = Self::payload_prefix(handle, &encoded.columns, range.rows.len());
        buf.reserve(range.bytes.len());
        buf.extend_from_slice(&encoded.bytes[range.bytes.clone()]);
        buf
    }

    /// Derive each column's wire type and scale from the whole parameter set.
    ///
    /// A range must not infer its own types, because `infer_wire_type` reads the
    /// first value of a column and a later range could start with a different type.
    fn parameter_columns(
        handle: &PreparedStatementHandle,
        cols: &[Vec<serde_json::Value>],
    ) -> Vec<ParameterColumn> {
        cols.iter()
            .enumerate()
            .map(|(i, col_values)| {
                let (wire_type, name) = Self::infer_wire_type(handle, i, col_values);
                let scale = if wire_type == T_DECIMAL {
                    Self::decimal_metadata(handle, i).1
                } else {
                    0
                };
                ParameterColumn {
                    name,
                    wire_type,
                    scale,
                }
            })
            .collect()
    }

    /// The bytes that precede the row data: the statement header and the column headers.
    fn payload_prefix(
        handle: &PreparedStatementHandle,
        columns: &[ParameterColumn],
        num_rows: usize,
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&handle.handle.to_le_bytes());
        buf.extend_from_slice(&PARAMETER_TABLE_COUNT.to_le_bytes());
        buf.push(IS_TABLE);
        buf.extend_from_slice(&(columns.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(num_rows as i64).to_le_bytes()); // total_rows
        buf.extend_from_slice(&(num_rows as i64).to_le_bytes()); // rows_in_msg

        for (i, column) in columns.iter().enumerate() {
            let name_bytes = column.name.as_bytes();
            buf.extend_from_slice(&(name_bytes.len() as i32).to_le_bytes());
            buf.extend_from_slice(name_bytes);
            buf.extend_from_slice(&(column.wire_type as i32).to_le_bytes());
            Self::write_column_metadata(&mut buf, handle, i, column.wire_type);
        }
        buf
    }

    /// Append the type-specific metadata a column header carries.
    /// BOOLEAN, DOUBLE and the other fixed-width types have none.
    fn write_column_metadata(
        buf: &mut Vec<u8>,
        handle: &PreparedStatementHandle,
        col_idx: usize,
        wire_type: u32,
    ) {
        match wire_type {
            T_CHAR => {
                buf.push(IS_VARCHAR | IS_UTF8); // vc_flag: is_varchar + utf8
                buf.extend_from_slice(&2_000_000i32.to_le_bytes()); // max_len
                buf.extend_from_slice(&(2_000_000i32 * 4).to_le_bytes()); // octet_len
            }
            T_DECIMAL => {
                let (prec, scale) = Self::decimal_metadata(handle, col_idx);
                buf.extend_from_slice(&prec.to_le_bytes());
                buf.extend_from_slice(&scale.to_le_bytes());
            }
            _ => {}
        }
    }

    /// Append the parameter data of `rows` in row-major order: col0_row0, col1_row0, col0_row1, ...
    ///
    /// Exasol's native prepared-statement protocol requires this interleaving.
    /// Column-major encoding causes the server to drop the connection for num_rows > 1.
    fn write_parameter_rows(
        buf: &mut Vec<u8>,
        cols: &[Vec<serde_json::Value>],
        columns: &[ParameterColumn],
        rows: Range<usize>,
    ) -> Result<(), TransportError> {
        for row_idx in rows {
            for (col_values, column) in cols.iter().zip(columns) {
                Self::write_param_value(buf, column.wire_type, &col_values[row_idx], column.scale)?;
            }
        }
        Ok(())
    }

    /// Determine wire type and column name from handle metadata or by inference.
    fn infer_wire_type(
        handle: &PreparedStatementHandle,
        col_idx: usize,
        col_values: &[serde_json::Value],
    ) -> (u32, String) {
        if col_idx < handle.parameter_types.len() {
            let dt = &handle.parameter_types[col_idx];
            let wire = Self::data_type_to_wire_type(dt);
            let name = handle
                .parameter_names
                .get(col_idx)
                .and_then(|n| n.clone())
                .unwrap_or_else(|| format!("param{}", col_idx));
            (wire, name)
        } else {
            let wire = col_values
                .first()
                .map(Self::json_value_to_wire_type)
                .unwrap_or(T_CHAR);
            (wire, format!("param{}", col_idx))
        }
    }

    /// Get decimal precision and scale from handle metadata.
    fn decimal_metadata(handle: &PreparedStatementHandle, col_idx: usize) -> (i32, i32) {
        if col_idx < handle.parameter_types.len() {
            let dt = &handle.parameter_types[col_idx];
            (dt.precision.unwrap_or(18), dt.scale.unwrap_or(0))
        } else {
            (18, 0)
        }
    }

    /// Map DataType to native wire type ID.
    fn data_type_to_wire_type(dt: &super::messages::DataType) -> u32 {
        match dt.type_name.as_str() {
            "DECIMAL" => T_DECIMAL,
            "DOUBLE" => T_DOUBLE,
            "BOOLEAN" => T_BOOLEAN,
            "VARCHAR" | "CHAR" => T_CHAR,
            "DATE" => T_DATE,
            "TIMESTAMP" => T_TIMESTAMP,
            "TIMESTAMP WITH LOCAL TIME ZONE" => T_TIMESTAMP_UTC,
            "GEOMETRY" => T_GEOMETRY,
            "HASHTYPE" => T_HASHTYPE,
            "INTERVAL YEAR TO MONTH" => T_INTERVAL_YEAR,
            "INTERVAL DAY TO SECOND" => T_INTERVAL_DAY,
            _ => T_CHAR, // fallback
        }
    }

    /// Infer wire type from a JSON value.
    fn json_value_to_wire_type(value: &serde_json::Value) -> u32 {
        match value {
            serde_json::Value::Null => T_CHAR,
            serde_json::Value::Bool(_) => T_BOOLEAN,
            serde_json::Value::Number(n) => {
                if n.is_f64() && !n.is_i64() && !n.is_u64() {
                    T_DOUBLE
                } else {
                    T_DECIMAL
                }
            }
            serde_json::Value::String(_) => T_CHAR,
            _ => T_CHAR,
        }
    }

    /// Write a single parameter value in native binary format.
    ///
    /// `scale` is the Exasol DECIMAL scale; it is only used for `T_DECIMAL`
    /// values, where the wire format expects an already-scaled `i64` integer
    /// (e.g. `1.23` with scale `2` is sent as `123`). It is ignored for all
    /// other types.
    fn write_param_value(
        buf: &mut Vec<u8>,
        wire_type: u32,
        value: &serde_json::Value,
        scale: i32,
    ) -> Result<(), TransportError> {
        if value.is_null() {
            buf.push(0u8); // null marker
            return Ok(());
        }
        buf.push(1u8); // not-null marker

        match wire_type {
            T_BOOLEAN => {
                let b = value.as_bool().unwrap_or(false);
                buf.push(if b { 1u8 } else { 0u8 });
            }
            T_DOUBLE => {
                let d = value.as_f64().unwrap_or(0.0);
                buf.extend_from_slice(&d.to_le_bytes());
            }
            T_DECIMAL => {
                let scaled = Self::scale_decimal_value(value, scale);
                buf.extend_from_slice(&scaled.to_le_bytes());
            }
            T_DATE => {
                // Date as packed integer: (year<<16)+(month<<8)+day
                let s = value.as_str().unwrap_or("2000-01-01");
                let packed = Self::parse_date_to_packed(s);
                buf.extend_from_slice(&packed.to_le_bytes());
            }
            T_TIMESTAMP | T_TIMESTAMP_LOCAL_TZ | T_TIMESTAMP_UTC => {
                // Timestamp: [year:2 LE][month:1][day:1][hour:1][min:1][sec:1][nanos:4 LE]
                let s = value.as_str().unwrap_or("2000-01-01 00:00:00");
                Self::write_timestamp_bytes(buf, s);
            }
            _ => {
                // String-like types (CHAR, VARCHAR, etc.)
                let s = match value {
                    serde_json::Value::String(s) => s.as_str(),
                    _ => {
                        let formatted = value.to_string();
                        let bytes = formatted.as_bytes();
                        buf.extend_from_slice(&(bytes.len() as i32).to_le_bytes());
                        buf.extend_from_slice(bytes);
                        return Ok(());
                    }
                };
                let bytes = s.as_bytes();
                buf.extend_from_slice(&(bytes.len() as i32).to_le_bytes());
                buf.extend_from_slice(bytes);
            }
        }
        Ok(())
    }

    /// Convert a JSON parameter value into the scaled integer wire
    /// representation required by Exasol for DECIMAL parameters.
    ///
    /// Exasol's native protocol transmits DECIMAL values as already-scaled
    /// integers: a decimal `1.23` with `scale = 2` is sent as `123`. The result
    /// is saturated to `i64::MIN..=i64::MAX` because the current wire encoding
    /// uses 8 bytes for bound DECIMAL parameters.
    fn scale_decimal_value(value: &serde_json::Value, scale: i32) -> i64 {
        let multiplier = 10f64.powi(scale);

        if let Some(i) = value.as_i64() {
            if scale <= 0 {
                if scale == 0 {
                    return i;
                }
                let divisor = 10f64.powi(-scale);
                return ((i as f64) / divisor).round() as i64;
            }
            let pow10 = 10i128.checked_pow(scale as u32);
            if let Some(p) = pow10 {
                let scaled = (i as i128).saturating_mul(p);
                return scaled.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
            }
            return ((i as f64) * multiplier).round() as i64;
        }

        if let Some(f) = value.as_f64() {
            let scaled = (f * multiplier).round();
            if scaled.is_nan() {
                return 0;
            }
            return scaled.clamp(i64::MIN as f64, i64::MAX as f64) as i64;
        }

        0
    }

    /// Parse a date string "YYYY-MM-DD" into packed int format.
    fn parse_date_to_packed(s: &str) -> i32 {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() == 3 {
            let year: i32 = parts[0].parse().unwrap_or(2000);
            let month: i32 = parts[1].parse().unwrap_or(1);
            let day: i32 = parts[2].parse().unwrap_or(1);
            (year << 16) + (month << 8) + day
        } else {
            (2000 << 16) + (1 << 8) + 1
        }
    }

    /// Write timestamp bytes: [year:2 LE][month:1][day:1][hour:1][min:1][sec:1][nanos:4 LE]
    fn write_timestamp_bytes(buf: &mut Vec<u8>, s: &str) {
        // Parse "YYYY-MM-DD HH:MM:SS" or "YYYY-MM-DD HH:MM:SS.ffffff"
        let date_part = &s[..10.min(s.len())];
        let time_part = if s.len() > 11 { &s[11..] } else { "00:00:00" };

        let parts: Vec<&str> = date_part.split('-').collect();
        let year: i16 = parts.first().and_then(|p| p.parse().ok()).unwrap_or(2000);
        let month: u8 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(1);
        let day: u8 = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(1);

        let time_parts: Vec<&str> = time_part.split(':').collect();
        let hour: u8 = time_parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minute: u8 = time_parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
        let sec_str = time_parts.get(2).unwrap_or(&"0");
        let sec_parts: Vec<&str> = sec_str.split('.').collect();
        let second: u8 = sec_parts[0].parse().unwrap_or(0);
        let nanos: i32 = if sec_parts.len() > 1 {
            let frac = sec_parts[1];
            let padded = format!("{:0<9}", frac);
            padded[..9].parse().unwrap_or(0)
        } else {
            0
        };

        buf.extend_from_slice(&year.to_le_bytes());
        buf.push(month);
        buf.push(day);
        buf.push(hour);
        buf.push(minute);
        buf.push(second);
        buf.extend_from_slice(&nanos.to_le_bytes());
    }

    /// Convert a response to QueryResult, caching column metadata and the fetch
    /// start position for result sets with handles.
    ///
    /// The start position is the number of rows the execute response already
    /// delivered, so the first `CMD_FETCH2` does not repeat them.
    fn convert_and_cache_result(
        &mut self,
        response: NativeResponse,
    ) -> Result<QueryResult, TransportError> {
        if let NativeResponse::ResultSet {
            ref handle,
            ref columns,
            total_rows: _,
            rows_received,
            ..
        } = response
        {
            // Only cache column metadata for handles the server has NOT already closed.
            // The server auto-closes a handle when all rows are returned in the initial
            // response; caching metadata for such handles causes spurious CMD_FETCH2 calls.
            if *handle != SMALL_RESULTSET {
                self.result_columns
                    .insert(*handle, Arc::new(columns.clone()));
                self.fetch_positions.insert(*handle, rows_received);
            }
        }
        Self::native_result_to_query_result(response)
    }

    /// The server's maximum data message size, or the documented default before login.
    fn max_data_message_size(&self) -> i64 {
        self.session
            .as_ref()
            .map(|s| s.max_data_message_size)
            .unwrap_or(MAX_DATA_MESSAGE_SIZE as i64)
    }

    /// Send CMD_SET_ATTRIBUTES and fail with the server's message when it rejects a value.
    async fn set_attributes(&mut self, attrs: &AttributeSet) -> Result<(), TransportError> {
        self.lifecycle.require(
            ConnectionState::Authenticated,
            "Must authenticate before setting attributes",
        )?;

        let header = self
            .send_and_fill_buf(CMD_SET_ATTRIBUTES, attrs, None)
            .await?;
        Self::check_response(&header, &self.recv_buf)?;
        Ok(())
    }

    /// Every CMD_EXECUTE_PREPARED message carries these attributes, and `split_parameter_rows` counts their size.
    fn execute_prepared_attributes() -> AttributeSet {
        AttributeSet::new()
    }

    /// Send one CMD_EXECUTE_PREPARED message and convert its answer.
    async fn run_execute_prepared(&mut self, data: &[u8]) -> Result<QueryResult, TransportError> {
        let attrs = Self::execute_prepared_attributes();
        let (header, payload) = self
            .send_and_receive(CMD_EXECUTE_PREPARED, &attrs, Some(data))
            .await?;
        let response = Self::check_response(&header, &payload)?.terminal;
        self.convert_and_cache_result(response)
    }

    /// Run one execution per range of `encoded`, sum the row counts, and stop at a result set.
    async fn execute_prepared_ranges(
        &mut self,
        handle: &PreparedStatementHandle,
        encoded: &EncodedParameterRows,
    ) -> Result<QueryResult, TransportError> {
        let mut affected_rows = 0;
        for range in &encoded.ranges {
            let data = Self::build_range_payload(handle, encoded, range);
            let result = self.run_execute_prepared(&data).await?;
            if let Some(result_set) = Self::add_range_result(&mut affected_rows, result) {
                return Ok(result_set);
            }
        }
        Ok(QueryResult::row_count(affected_rows))
    }

    /// Add a row count to the total, or hand back a result set that ends the batch.
    fn add_range_result(affected_rows: &mut i64, result: QueryResult) -> Option<QueryResult> {
        match result {
            QueryResult::RowCount { count } => {
                *affected_rows += count;
                None
            }
            result_set => Some(result_set),
        }
    }
}

impl Default for NativeTcpTransport {
    fn default() -> Self {
        Self::new()
    }
}

/// Read an attribute the server may encode either as raw bytes or as text.
///
/// The handshake attributes that carry key material (public key, random phrase)
/// arrive as `T_binary` from current servers but as `T_char` from older ones,
/// so both encodings resolve to the same byte string here.
fn attribute_bytes(attrs: &AttributeSet, id: u16) -> Option<Vec<u8>> {
    match attrs.get(id)? {
        AttributeValue::Binary(bytes) => Some(bytes.clone()),
        AttributeValue::String(text) => Some(text.as_bytes().to_vec()),
        _ => None,
    }
}

/// Read a text attribute, ignoring any other encoding.
fn attribute_text(attrs: &AttributeSet, id: u16) -> Option<String> {
    match attrs.get(id)? {
        AttributeValue::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// Read an integer attribute, widening the 32-bit encoding to 64 bits.
fn attribute_integer(attrs: &AttributeSet, id: u16) -> Option<i64> {
    match attrs.get(id)? {
        AttributeValue::Int64(value) => Some(*value),
        AttributeValue::Int32(value) => Some(*value as i64),
        _ => None,
    }
}

/// The change a response's attributes make to the recorded current schema.
///
/// `None` leaves the recorded value unchanged: a response to any command other
/// than CMD_GET_ATTRIBUTES carries attribute 22 only when the statement changed
/// it. `Some(None)` clears it, because an empty attribute 22 means no schema.
fn current_schema_change(attrs: &AttributeSet) -> Option<Option<String>> {
    attribute_text(attrs, ATTR_CURRENT_SCHEMA).map(|name| Some(name).filter(|n| !n.is_empty()))
}

/// The current schema a CMD_GET_ATTRIBUTES response reports.
///
/// That response lists every session attribute, so there an absent attribute 22
/// means the session has no current schema.
fn current_schema_from_get_attributes(attrs: &AttributeSet) -> Option<String> {
    current_schema_change(attrs).flatten()
}

/// Fail when a response payload carries a server exception, describing it with `context`.
fn reject_exception(context: &str, result_data: &[u8]) -> Result<(), TransportError> {
    if result_data.is_empty() {
        return Ok(());
    }
    let response = result_parser::parse_response(result_data)?;
    if let NativeResponse::Exception { message, sql_state } = response.terminal {
        return Err(TransportError::ProtocolError(format!(
            "{}: {} (SQL state: {})",
            context, message, sql_state
        )));
    }
    Ok(())
}

#[async_trait]
impl LifecycleSteps for NativeTcpTransport {
    fn lifecycle_mut(&mut self) -> &mut ConnectionLifecycle {
        &mut self.lifecycle
    }

    async fn login_exchange(
        &mut self,
        credentials: &Credentials,
    ) -> Result<SessionInfo, TransportError> {
        // Phase 1: Send login packet
        let login_packet = handshake::build_login_packet(&credentials.username);
        self.send_raw(&login_packet).await?;

        // Phase 1 response: Server responds with a standard 21-byte header + attribute payload
        let (login_header, login_payload) = self.receive_message().await?;

        // Parse attributes from the login response payload
        let server_attrs = super::native::attributes::parse_attributes(
            &login_payload,
            login_header.num_attributes,
        )?;

        // Extract public key (binary: [exponent:128 BE][modulus:128 BE])
        let public_key = attribute_bytes(&server_attrs, ATTR_PUBLIC_KEY).ok_or_else(|| {
            TransportError::ProtocolError("Server did not send public key".into())
        })?;

        // Extract random phrase for RSA interleaving
        let random_phrase =
            attribute_bytes(&server_attrs, ATTR_RANDOM_PHRASE).ok_or_else(|| {
                TransportError::ProtocolError("Server did not send random phrase".into())
            })?;

        // Extract session info from server attributes
        let session_id = attribute_integer(&server_attrs, ATTR_SESSIONID)
            .map(|id| id.to_string())
            .unwrap_or_else(|| "0".to_string());

        let protocol_version = attribute_integer(&server_attrs, ATTR_PROTOCOL_VERSION)
            .map(|version| version as i32)
            .unwrap_or(PROTOCOL_VERSION as i32);

        let release_version =
            attribute_text(&server_attrs, ATTR_RELEASE_VERSION).unwrap_or_default();

        let database_name = attribute_text(&server_attrs, ATTR_DATABASE_NAME).unwrap_or_default();

        let product_name = attribute_text(&server_attrs, ATTR_PRODUCT_NAME).unwrap_or_default();

        let max_data_msg_size = attribute_integer(&server_attrs, ATTR_DATA_MESSAGE_SIZE)
            .unwrap_or(MAX_DATA_MESSAGE_SIZE as i64);

        let time_zone = attribute_text(&server_attrs, ATTR_TIMEZONE);

        // Phase 2: Send password + ChaCha20 keys
        let use_chacha20 = !self.tls_active;
        let auth = handshake::build_auth_message(
            &credentials.password,
            &public_key,
            &random_phrase,
            &self.serial,
            use_chacha20,
        )?;
        self.send_raw(&auth.wire_bytes).await?;

        // Phase 2 response: Read the server's response to CMD_SET_ATTRIBUTES
        let (header, payload) = self.receive_message().await?;

        // Check for exception in the result part (skip attribute data)
        let result_data = Self::extract_result_data(&header, &payload);
        reject_exception("Authentication failed", &result_data)?;

        // Activate ChaCha20 encryption for all subsequent messages (skip over TLS)
        if use_chacha20 && !auth.send_key.is_empty() {
            self.encryptor.set_keys(&auth.send_key, &auth.recv_key);
        }

        // Phase 3: CMD_GET_ATTRIBUTES to retrieve session info
        let empty_attrs = AttributeSet::new();
        self.send_message(CMD_GET_ATTRIBUTES, &empty_attrs, None)
            .await?;
        let (ga_header, ga_payload) = self.receive_message().await?;

        // Parse session attributes from the response
        let ga_result_data = Self::extract_result_data(&ga_header, &ga_payload);
        reject_exception("GET_ATTRIBUTES failed", &ga_result_data)?;

        let ga_attrs = Self::response_attributes(&ga_header, &ga_payload)?;
        self.current_schema = current_schema_from_get_attributes(&ga_attrs);

        let session_id = attribute_integer(&ga_attrs, ATTR_SESSIONID)
            .map(|id| id.to_string())
            .unwrap_or(session_id);

        let release_version =
            attribute_text(&ga_attrs, ATTR_RELEASE_VERSION).unwrap_or(release_version);

        let database_name = attribute_text(&ga_attrs, ATTR_DATABASE_NAME).unwrap_or(database_name);

        let product_name = attribute_text(&ga_attrs, ATTR_PRODUCT_NAME).unwrap_or(product_name);

        let time_zone = attribute_text(&ga_attrs, ATTR_TIMEZONE).or(time_zone);

        let session_info = SessionInfo {
            session_id,
            protocol_version,
            release_version,
            database_name,
            product_name,
            max_data_message_size: max_data_msg_size,
            time_zone,
        };

        self.session = Some(session_info.clone());
        Ok(session_info)
    }

    async fn send_disconnect(&mut self) {
        if self.lifecycle.state() == ConnectionState::Authenticated {
            let empty_attrs = AttributeSet::new();
            let _ = self
                .send_and_receive(CMD_DISCONNECT, &empty_attrs, None)
                .await;
        }
    }

    fn release_connection(&mut self) {
        self.stream = None;
        self.session = None;
        self.current_schema = None;
    }
}

#[async_trait]
impl TransportProtocol for NativeTcpTransport {
    async fn connect(&mut self, params: &ConnectionParams) -> Result<(), TransportError> {
        let deadline = self.lifecycle.begin_connect(params.timeout_ms)?;
        let addr = format!("{}:{}", params.host, params.port);

        let tcp_stream = deadline
            .run(SetupStep::TcpConnect, async {
                TcpStream::connect(&addr)
                    .await
                    .map_err(|e| TransportError::IoError(e.to_string()))
            })
            .await?;

        tcp_stream
            .set_nodelay(true)
            .map_err(|e| TransportError::IoError(e.to_string()))?;

        if params.use_tls {
            let config = tls::client_config(
                params.certificate_fingerprint.as_deref(),
                params.validate_server_certificate,
            );
            let tls_stream = deadline
                .run(
                    SetupStep::TlsHandshake,
                    tls::client_handshake(tcp_stream, &params.host, config),
                )
                .await?;

            self.stream = Some(NativeStream::Tls(Box::new(tls_stream)));
            self.tls_active = true;
        } else {
            self.stream = Some(NativeStream::Plain(tcp_stream));
        }

        self.lifecycle.connected(deadline);
        Ok(())
    }

    async fn authenticate(
        &mut self,
        credentials: &Credentials,
    ) -> Result<SessionInfo, TransportError> {
        lifecycle::authenticate_within_deadline(self, credentials).await
    }

    async fn execute_query(&mut self, sql: &str) -> Result<QueryResult, TransportError> {
        self.lifecycle.require(
            ConnectionState::Authenticated,
            "Must authenticate before executing queries",
        )?;

        let attrs = AttributeSet::new();
        let header = self
            .send_and_fill_buf(CMD_EXECUTE, &attrs, Some(sql.as_bytes()))
            .await?;
        let response = {
            let result_data = Self::result_data_slice(&header, &self.recv_buf);
            let r = result_parser::parse_response(result_data)?;
            if let NativeResponse::Exception {
                ref message,
                ref sql_state,
            } = r.terminal
            {
                return Err(TransportError::ProtocolError(format!(
                    "{} (SQL state: {})",
                    message, sql_state
                )));
            }
            r.terminal
        };
        self.convert_and_cache_result(response)
    }

    async fn fetch_results(
        &mut self,
        handle: ResultSetHandle,
    ) -> Result<ResultData, TransportError> {
        self.lifecycle.require(
            ConnectionState::Authenticated,
            "Must authenticate before fetching results",
        )?;

        let handle_id = handle.as_i32();
        let start_position = *self.fetch_positions.get(&handle_id).unwrap_or(&0);
        let fetch_size_bytes = self.max_data_message_size();

        // CMD_FETCH2 payload: [handle:4 LE] [start_position:8 LE] [fetch_size_bytes:8 LE]
        let mut data = Vec::with_capacity(20);
        data.extend_from_slice(&handle_id.to_le_bytes());
        data.extend_from_slice(&start_position.to_le_bytes());
        data.extend_from_slice(&fetch_size_bytes.to_le_bytes());

        // Get cached columns before the zero-copy borrow of recv_buf
        let cached_columns = self
            .result_columns
            .get(&handle_id)
            .map(Arc::clone)
            .ok_or_else(|| {
                TransportError::ProtocolError("No cached column metadata for fetch".into())
            })?;

        let attrs = AttributeSet::new();
        let header = self
            .send_and_fill_buf(CMD_FETCH2, &attrs, Some(&data))
            .await?;

        let response = {
            let payload = &self.recv_buf;
            Self::check_response(&header, payload)?.terminal
        };
        match response {
            NativeResponse::MoreRows(data) => {
                let (rows_received, batch) =
                    result_parser::parse_fetch_to_record_batch(&data, &cached_columns)?;
                self.fetch_positions
                    .insert(handle_id, start_position + rows_received);
                let col_infos = Self::to_column_info(&cached_columns);
                Ok(ResultData {
                    columns: col_infos,
                    data: ResultPayload::Arrow(batch),
                    total_rows: rows_received,
                })
            }
            NativeResponse::ResultSet {
                batch,
                rows_received,
                total_rows,
                columns,
                ..
            } => {
                self.fetch_positions
                    .insert(handle_id, start_position + rows_received);
                let col_infos = Self::to_column_info(&columns);
                let record_batch = batch.unwrap_or_else(|| {
                    arrow::record_batch::RecordBatch::new_empty(std::sync::Arc::new(
                        arrow::datatypes::Schema::empty(),
                    ))
                });
                Ok(ResultData {
                    columns: col_infos,
                    data: ResultPayload::Arrow(record_batch),
                    total_rows,
                })
            }
            NativeResponse::Empty => Ok(ResultData {
                columns: vec![],
                data: ResultPayload::Arrow(arrow::record_batch::RecordBatch::new_empty(
                    std::sync::Arc::new(arrow::datatypes::Schema::empty()),
                )),
                total_rows: 0,
            }),
            other => Err(TransportError::ProtocolError(format!(
                "Expected a result set or more-rows response from CMD_FETCH2, got {other:?}"
            ))),
        }
    }

    async fn close_result_set(&mut self, handle: ResultSetHandle) -> Result<(), TransportError> {
        self.lifecycle.require(
            ConnectionState::Authenticated,
            "Must authenticate before closing result sets",
        )?;

        self.fetch_positions.remove(&handle.as_i32());
        self.result_columns.remove(&handle.as_i32());

        // Send handle as data payload
        let data = handle.as_i32().to_le_bytes();
        let attrs = AttributeSet::new();
        let (header, payload) = self
            .send_and_receive(CMD_CLOSE_RESULTSET, &attrs, Some(&data))
            .await?;

        // Check for exception
        if !payload.is_empty() {
            let _ = Self::check_response(&header, &payload)?;
        }
        Ok(())
    }

    async fn create_prepared_statement(
        &mut self,
        sql: &str,
    ) -> Result<PreparedStatementHandle, TransportError> {
        self.lifecycle.require(
            ConnectionState::Authenticated,
            "Must authenticate before creating prepared statements",
        )?;

        let attrs = AttributeSet::new();
        let sql_bytes = sql.as_bytes();

        let (header, payload) = self
            .send_and_receive(CMD_CREATE_PREPARED, &attrs, Some(sql_bytes))
            .await?;
        let response = Self::check_response(&header, &payload)?.terminal;

        match response {
            NativeResponse::PreparedStatement {
                handle: stmt_handle,
                parameters,
                result_columns,
            } => {
                let param_types: Vec<_> = parameters
                    .iter()
                    .map(arrow_builder::native_meta_to_data_type)
                    .collect();
                let param_names: Vec<_> = parameters
                    .iter()
                    .map(|c| {
                        if c.name.is_empty() {
                            None
                        } else {
                            Some(c.name.clone())
                        }
                    })
                    .collect();
                Ok(PreparedStatementHandle::new(
                    stmt_handle,
                    parameters.len() as i32,
                    param_types,
                    param_names,
                )
                .with_result_columns(Self::to_column_info(&result_columns)))
            }
            NativeResponse::Empty => Ok(PreparedStatementHandle::new(0, 0, vec![], vec![])),
            _ => Err(TransportError::ProtocolError(
                "Unexpected response from CREATE PREPARED".into(),
            )),
        }
    }

    /// Executes a prepared statement.
    ///
    /// A statement that returns an affected-row count runs as consecutive executions when its
    /// parameter values exceed the server's maximum data message size, and returns the summed
    /// row count. Exasol drops the connection on a larger message.
    async fn execute_prepared_statement(
        &mut self,
        handle: &PreparedStatementHandle,
        parameters: Option<Vec<Vec<serde_json::Value>>>,
    ) -> Result<QueryResult, TransportError> {
        self.lifecycle.require(
            ConnectionState::Authenticated,
            "Must authenticate before executing prepared statements",
        )?;

        if let Some(cols) = parameters
            .as_deref()
            .filter(|_| handle.result_columns.is_empty())
        {
            let limit = usize::try_from(self.max_data_message_size())
                .unwrap_or(MAX_DATA_MESSAGE_SIZE as usize);
            let encoded = Self::split_parameter_rows(handle, cols, limit)?;
            if !encoded.ranges.is_empty() {
                return self.execute_prepared_ranges(handle, &encoded).await;
            }
        }

        let data = Self::build_execute_prepared_payload(handle, parameters.as_deref())?;
        self.run_execute_prepared(&data).await
    }

    async fn close_prepared_statement(
        &mut self,
        handle: &PreparedStatementHandle,
    ) -> Result<(), TransportError> {
        self.lifecycle.require(
            ConnectionState::Authenticated,
            "Must authenticate before closing prepared statements",
        )?;

        // Send handle as data payload
        let data = handle.handle.to_le_bytes();
        let attrs = AttributeSet::new();
        let (header, payload) = self
            .send_and_receive(CMD_CLOSE_PREPARED, &attrs, Some(&data))
            .await?;

        if !payload.is_empty() {
            let _ = Self::check_response(&header, &payload)?;
        }
        Ok(())
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        lifecycle::close_gracefully(self).await
    }

    fn terminate(&mut self) {
        lifecycle::terminate(self);
    }

    fn is_connected(&self) -> bool {
        self.lifecycle.is_open()
    }

    async fn set_autocommit(&mut self, enabled: bool) -> Result<(), TransportError> {
        let mut attrs = AttributeSet::new();
        attrs.add(ATTR_AUTOCOMMIT, AttributeValue::Bool(enabled));
        self.set_attributes(&attrs).await
    }

    async fn set_query_timeout(&mut self, timeout_secs: u64) -> Result<(), TransportError> {
        let timeout_secs_i32 = i32::try_from(timeout_secs).map_err(|_| {
            TransportError::ProtocolError(format!(
                "query_timeout of {timeout_secs}s exceeds the native protocol's i32 range"
            ))
        })?;

        let mut attrs = AttributeSet::new();
        attrs.add(ATTR_QUERY_TIMEOUT, AttributeValue::Int32(timeout_secs_i32));
        self.set_attributes(&attrs).await
    }

    async fn set_current_schema(&mut self, schema: &str) -> Result<(), TransportError> {
        let mut attrs = AttributeSet::new();
        attrs.add(
            ATTR_CURRENT_SCHEMA,
            AttributeValue::String(schema.to_owned()),
        );
        self.set_attributes(&attrs).await?;
        self.refresh_current_schema().await?;
        Ok(())
    }

    async fn refresh_current_schema(&mut self) -> Result<Option<String>, TransportError> {
        self.lifecycle.require(
            ConnectionState::Authenticated,
            "Must authenticate before reading attributes",
        )?;

        let header = self
            .send_and_fill_buf(CMD_GET_ATTRIBUTES, &AttributeSet::new(), None)
            .await?;
        Self::check_response(&header, &self.recv_buf)?;
        let attrs = Self::response_attributes(&header, &self.recv_buf)?;
        self.current_schema = current_schema_from_get_attributes(&attrs);
        Ok(self.current_schema.clone())
    }

    fn current_schema(&self) -> Option<String> {
        self.current_schema.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::lifecycle::{ConnectionLifecycle, ConnectionState};
    use crate::transport::test_support::{
        assert_names_the_termination, finish_within, silent_server_params, test_credentials,
        SilentServer, DISCONNECT_BOUND, LOOPBACK_TEST_BOUND, SILENT_SERVER_CONNECTION_TIMEOUT,
    };
    use std::time::Instant;

    #[test]
    fn transport_new_is_disconnected() {
        let transport = NativeTcpTransport::new();
        assert!(!transport.is_connected());
        assert_eq!(transport.lifecycle.state(), ConnectionState::Disconnected);
    }

    #[test]
    fn transport_default_is_disconnected() {
        let transport = NativeTcpTransport::default();
        assert!(!transport.is_connected());
    }

    #[tokio::test]
    async fn connect_requires_disconnected_state() {
        let mut transport = NativeTcpTransport::new();
        transport.lifecycle = ConnectionLifecycle::in_state(ConnectionState::Connected);

        let params = ConnectionParams::new("localhost".to_string(), 8563);
        let result = transport.connect(&params).await;
        assert!(result.is_err());
    }

    /// Scenario: Server that never answers the TLS handshake
    #[tokio::test]
    async fn connect_fails_at_the_tls_handshake_when_the_server_never_answers() {
        finish_within(LOOPBACK_TEST_BOUND, async {
            let mut server = SilentServer::accepting().await;
            let mut transport = NativeTcpTransport::new();
            let started = Instant::now();

            let error = transport
                .connect(&silent_server_params(&server))
                .await
                .expect_err("a server that never answers must fail the connection");

            assert!(started.elapsed() >= SILENT_SERVER_CONNECTION_TIMEOUT);
            assert!(
                error
                    .to_string()
                    .contains("Connection timeout after 300ms (TLS handshake)"),
                "{error}"
            );
            finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;
        })
        .await;
    }

    /// Scenario: Server that never answers the login
    #[tokio::test]
    async fn authenticate_fails_at_login_when_the_server_goes_silent_after_tls() {
        finish_within(LOOPBACK_TEST_BOUND, async {
            let mut server = SilentServer::after_tls().await;
            let mut transport = NativeTcpTransport::new();
            let started = Instant::now();

            transport
                .connect(&silent_server_params(&server))
                .await
                .expect("the TLS handshake completes");
            let error = transport
                .authenticate(&test_credentials())
                .await
                .expect_err("a server that never answers the login must fail it");

            assert!(started.elapsed() >= SILENT_SERVER_CONNECTION_TIMEOUT);
            assert!(
                error
                    .to_string()
                    .contains("Connection timeout after 300ms (login)"),
                "{error}"
            );
            assert!(!transport.is_connected());
            finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await;
        })
        .await;
    }

    /// Scenario: Terminate a connection whose in-flight response is no longer trusted
    /// Scenario: Operations after an export timeout name the termination
    #[tokio::test]
    async fn operations_after_terminate_report_the_terminated_transport() {
        let mut transport = NativeTcpTransport::new();
        transport.lifecycle = ConnectionLifecycle::in_state(ConnectionState::Authenticated);

        transport.terminate();
        let error = transport
            .execute_query("SELECT 1")
            .await
            .expect_err("a terminated transport refuses queries");
        transport
            .close()
            .await
            .expect("closing a terminated transport succeeds");
        let error_after_close = transport
            .execute_query("SELECT 1")
            .await
            .expect_err("a closed terminated transport still refuses queries");

        assert_names_the_termination(&error);
        assert_names_the_termination(&error_after_close);
        assert!(!transport.is_connected());
    }

    #[tokio::test]
    async fn authenticate_requires_connected_state() {
        let mut transport = NativeTcpTransport::new();
        let creds = Credentials::new("sys".to_string(), "exasol".to_string());
        let result = transport.authenticate(&creds).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_requires_authenticated_state() {
        let mut transport = NativeTcpTransport::new();
        let result = transport.execute_query("SELECT 1").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn close_is_idempotent() {
        let mut transport = NativeTcpTransport::new();
        assert!(transport.close().await.is_ok());
        transport.lifecycle = ConnectionLifecycle::in_state(ConnectionState::Closed);
        assert!(transport.close().await.is_ok());
    }

    /// Scenario: Terminate a connection whose in-flight response is no longer trusted
    #[test]
    fn terminate_drops_the_session_and_reports_the_transport_disconnected() {
        let mut transport = NativeTcpTransport::new();
        transport.lifecycle = ConnectionLifecycle::in_state(ConnectionState::Authenticated);
        transport.session = Some(SessionInfo {
            session_id: "12345".to_string(),
            protocol_version: 3,
            release_version: "7.1.0".to_string(),
            database_name: "test_db".to_string(),
            product_name: "EXASolution".to_string(),
            max_data_message_size: 1024 * 1024,
            time_zone: Some("UTC".to_string()),
        });

        transport.terminate();

        assert!(!transport.is_connected());
        assert!(transport.session.is_none());
    }

    #[test]
    fn scale_decimal_value_scales_positive_float_by_power_of_ten() {
        let v = serde_json::json!(1.23);
        assert_eq!(NativeTcpTransport::scale_decimal_value(&v, 2), 123);
    }

    #[test]
    fn scale_decimal_value_rounds_half_up_for_floats() {
        let v = serde_json::json!(1.235);
        assert_eq!(NativeTcpTransport::scale_decimal_value(&v, 2), 124);
    }

    #[test]
    fn scale_decimal_value_passes_integer_unchanged_when_scale_zero() {
        let v = serde_json::json!(42i64);
        assert_eq!(NativeTcpTransport::scale_decimal_value(&v, 0), 42);
    }

    #[test]
    fn scale_decimal_value_scales_integer_exactly_without_float_rounding() {
        // 1 with scale=18 should be 10^18 exactly.
        let v = serde_json::json!(1i64);
        assert_eq!(
            NativeTcpTransport::scale_decimal_value(&v, 18),
            1_000_000_000_000_000_000i64
        );
    }

    #[test]
    fn scale_decimal_value_saturates_on_integer_overflow() {
        // 10^19 overflows i64, the helper must clamp instead of panic.
        let v = serde_json::json!(1i64);
        let out = NativeTcpTransport::scale_decimal_value(&v, 19);
        assert_eq!(out, i64::MAX);
    }

    #[test]
    fn scale_decimal_value_handles_negative_integer_values() {
        let v = serde_json::json!(-1i64);
        assert_eq!(NativeTcpTransport::scale_decimal_value(&v, 3), -1000);
    }

    #[test]
    fn scale_decimal_value_returns_zero_for_non_numeric_input() {
        let v = serde_json::json!("oops");
        assert_eq!(NativeTcpTransport::scale_decimal_value(&v, 2), 0);
    }

    #[test]
    fn scale_decimal_value_round_trips_negative_float() {
        let v = serde_json::json!(-1.23);
        assert_eq!(NativeTcpTransport::scale_decimal_value(&v, 2), -123);
    }

    // --- Reading server handshake attributes ---

    fn attribute_set(values: &[(u16, AttributeValue)]) -> AttributeSet {
        let mut attrs = AttributeSet::new();
        for (id, value) in values {
            attrs.add(*id, value.clone());
        }
        attrs
    }

    #[test]
    fn attribute_bytes_accepts_both_binary_and_text_encodings() {
        let attrs = attribute_set(&[
            (ATTR_PUBLIC_KEY, AttributeValue::Binary(vec![1, 2, 3])),
            (
                ATTR_RANDOM_PHRASE,
                AttributeValue::String("phrase".to_owned()),
            ),
        ]);

        assert_eq!(
            attribute_bytes(&attrs, ATTR_PUBLIC_KEY),
            Some(vec![1, 2, 3])
        );
        assert_eq!(
            attribute_bytes(&attrs, ATTR_RANDOM_PHRASE),
            Some(b"phrase".to_vec())
        );
    }

    #[test]
    fn attribute_bytes_rejects_numeric_and_absent_attributes() {
        let attrs = attribute_set(&[(ATTR_PUBLIC_KEY, AttributeValue::Int32(7))]);

        assert_eq!(attribute_bytes(&attrs, ATTR_PUBLIC_KEY), None);
        assert_eq!(attribute_bytes(&attrs, ATTR_RANDOM_PHRASE), None);
    }

    #[test]
    fn attribute_text_reads_only_text_attributes() {
        let attrs = attribute_set(&[
            (
                ATTR_RELEASE_VERSION,
                AttributeValue::String("8.34.0".to_owned()),
            ),
            (ATTR_DATABASE_NAME, AttributeValue::Int64(4)),
        ]);

        assert_eq!(
            attribute_text(&attrs, ATTR_RELEASE_VERSION),
            Some("8.34.0".to_owned())
        );
        assert_eq!(attribute_text(&attrs, ATTR_DATABASE_NAME), None);
        assert_eq!(attribute_text(&attrs, ATTR_PRODUCT_NAME), None);
    }

    #[test]
    fn attribute_integer_widens_int32_and_reads_int64() {
        let attrs = attribute_set(&[
            (ATTR_PROTOCOL_VERSION, AttributeValue::Int32(21)),
            (ATTR_SESSIONID, AttributeValue::Int64(1_700_000_000_000)),
            (ATTR_TIMEZONE, AttributeValue::String("UTC".to_owned())),
        ]);

        assert_eq!(attribute_integer(&attrs, ATTR_PROTOCOL_VERSION), Some(21));
        assert_eq!(
            attribute_integer(&attrs, ATTR_SESSIONID),
            Some(1_700_000_000_000)
        );
        assert_eq!(attribute_integer(&attrs, ATTR_TIMEZONE), None);
        assert_eq!(attribute_integer(&attrs, ATTR_DATA_MESSAGE_SIZE), None);
    }

    // --- Rejecting server exceptions ---

    #[test]
    fn reject_exception_accepts_an_empty_payload() {
        assert!(reject_exception("Authentication failed", &[]).is_ok());
    }

    #[test]
    fn reject_exception_accepts_a_payload_without_an_exception() {
        let payload = 0i32.to_le_bytes();
        assert!(reject_exception("Authentication failed", &payload).is_ok());
    }

    // --- Splitting attribute data from result data ---

    fn header_with_attribute_len(attribute_data_len: u32) -> MessageHeader {
        MessageHeader::new(CMD_EXECUTE, 1, 0, attribute_data_len, 1)
    }

    #[test]
    fn result_data_starts_after_the_attribute_block() {
        let header = header_with_attribute_len(2);
        let payload = [0xAA, 0xBB, 0x01, 0x02];

        assert_eq!(
            NativeTcpTransport::extract_result_data(&header, &payload),
            vec![0x01, 0x02]
        );
        assert_eq!(
            NativeTcpTransport::result_data_slice(&header, &payload),
            &[0x01, 0x02]
        );
    }

    #[test]
    fn result_data_is_empty_when_attributes_fill_the_payload() {
        let header = header_with_attribute_len(4);
        let payload = [0xAA, 0xBB, 0xCC, 0xDD];

        assert!(NativeTcpTransport::extract_result_data(&header, &payload).is_empty());
        assert!(NativeTcpTransport::result_data_slice(&header, &payload).is_empty());
    }

    #[test]
    fn result_data_is_empty_when_the_attribute_length_overruns_the_payload() {
        let header = header_with_attribute_len(99);
        let payload = [0xAA];

        assert!(NativeTcpTransport::extract_result_data(&header, &payload).is_empty());
        assert!(NativeTcpTransport::result_data_slice(&header, &payload).is_empty());
    }

    fn response_with_attributes(attrs: &AttributeSet) -> (MessageHeader, Vec<u8>) {
        let mut payload = attrs.serialize();
        let header = MessageHeader::new(
            CMD_EXECUTE,
            1,
            attrs.num_attributes(),
            payload.len() as u32,
            1,
        );
        payload.extend_from_slice(&0i32.to_le_bytes());
        (header, payload)
    }

    fn reported_change(attrs: &AttributeSet) -> Option<Option<String>> {
        let (header, payload) = response_with_attributes(attrs);
        let parsed = NativeTcpTransport::response_attributes(&header, &payload).unwrap();
        current_schema_change(&parsed)
    }

    /// Scenario: Track the current schema attribute from responses
    #[test]
    fn current_schema_change_reads_attribute_22_from_a_response() {
        let attrs = attribute_set(&[
            (ATTR_AUTOCOMMIT, AttributeValue::Bool(true)),
            (
                ATTR_CURRENT_SCHEMA,
                AttributeValue::String("ZZ_MixedCase".to_owned()),
            ),
        ]);

        assert_eq!(
            reported_change(&attrs),
            Some(Some("ZZ_MixedCase".to_owned()))
        );
    }

    /// Scenario: Track the current schema attribute from responses
    #[test]
    fn current_schema_change_reports_an_empty_attribute_22_as_no_schema() {
        let attrs = attribute_set(&[(ATTR_CURRENT_SCHEMA, AttributeValue::String(String::new()))]);

        assert_eq!(reported_change(&attrs), Some(None));
    }

    /// Scenario: Track the current schema attribute from responses
    #[test]
    fn response_without_attribute_22_leaves_the_current_schema_unchanged() {
        let other_attribute = attribute_set(&[(ATTR_AUTOCOMMIT, AttributeValue::Bool(false))]);

        assert_eq!(reported_change(&other_attribute), None);
        assert_eq!(reported_change(&AttributeSet::new()), None);
    }

    /// Scenario: Track the current schema attribute from responses
    #[test]
    fn get_attributes_response_without_attribute_22_means_no_current_schema() {
        let without_schema = attribute_set(&[(ATTR_AUTOCOMMIT, AttributeValue::Bool(true))]);
        let with_schema = attribute_set(&[(
            ATTR_CURRENT_SCHEMA,
            AttributeValue::String("SYS".to_owned()),
        )]);

        assert_eq!(current_schema_from_get_attributes(&without_schema), None);
        assert_eq!(
            current_schema_from_get_attributes(&with_schema),
            Some("SYS".to_owned())
        );
    }

    #[test]
    fn response_attributes_are_empty_when_the_header_declares_no_attribute_block() {
        let header = MessageHeader::new(CMD_EXECUTE, 1, 2, 0, 1);

        let attrs = NativeTcpTransport::response_attributes(&header, &[0, 0, 0, 0]).unwrap();

        assert_eq!(attrs.num_attributes(), 0);
    }

    #[test]
    fn response_attributes_report_a_truncated_attribute_block() {
        let header = MessageHeader::new(CMD_EXECUTE, 1, 1, 6, 0);
        let mut payload = ATTR_CURRENT_SCHEMA.to_le_bytes().to_vec();
        payload.extend_from_slice(&10u32.to_le_bytes());

        assert!(NativeTcpTransport::response_attributes(&header, &payload).is_err());
    }

    #[test]
    fn check_response_returns_the_envelope_for_a_row_count() {
        let mut payload = vec![0xAA];
        payload.extend_from_slice(&1i32.to_le_bytes());
        payload.push(constants::R_ROW_COUNT as u8);
        payload.extend_from_slice(&12i64.to_le_bytes());

        let envelope =
            NativeTcpTransport::check_response(&header_with_attribute_len(1), &payload).unwrap();

        assert!(matches!(envelope.terminal, NativeResponse::RowCount(12)));
    }

    #[test]
    fn check_response_turns_a_server_exception_into_an_error() {
        let message = "syntax error";
        let mut payload = 1i32.to_le_bytes().to_vec();
        payload.push(constants::R_EXCEPTION as u8);
        payload.extend_from_slice(&(message.len() as i32).to_le_bytes());
        payload.extend_from_slice(message.as_bytes());
        payload.extend_from_slice(b"42000");

        let err = NativeTcpTransport::check_response(&header_with_attribute_len(0), &payload)
            .unwrap_err();

        match err {
            TransportError::ProtocolError(msg) => {
                assert_eq!(msg, "syntax error (SQL state: 42000)")
            }
            other => panic!("expected ProtocolError, got {other:?}"),
        }
    }

    // --- Converting native responses to query results ---

    fn column_meta(name: &str, type_id: u32) -> NativeColumnMeta {
        NativeColumnMeta {
            name: name.to_string(),
            type_id,
            precision: None,
            scale: None,
            is_varchar: false,
            max_len: None,
        }
    }

    #[test]
    fn column_info_carries_the_mapped_data_type() {
        let columns = vec![column_meta("ID", T_DECIMAL), column_meta("FLAG", T_BOOLEAN)];

        let infos = NativeTcpTransport::to_column_info(&columns);

        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].name, "ID");
        assert_eq!(infos[0].data_type.type_name, "DECIMAL");
        assert_eq!(infos[1].name, "FLAG");
        assert_eq!(infos[1].data_type.type_name, "BOOLEAN");
    }

    #[test]
    fn small_result_set_needs_no_fetch_handle() {
        let response = NativeResponse::ResultSet {
            handle: SMALL_RESULTSET,
            columns: vec![column_meta("ID", T_DECIMAL)],
            batch: None,
            total_rows: 3,
            rows_received: 3,
        };

        match NativeTcpTransport::native_result_to_query_result(response).unwrap() {
            QueryResult::ResultSet { handle, data } => {
                assert!(handle.is_none());
                assert_eq!(data.total_rows, 3);
                assert_eq!(data.columns.len(), 1);
            }
            other => panic!("expected ResultSet, got {other:?}"),
        }
    }

    #[test]
    fn large_result_set_keeps_its_fetch_handle() {
        let response = NativeResponse::ResultSet {
            handle: 42,
            columns: Vec::new(),
            batch: None,
            total_rows: 1_000,
            rows_received: 100,
        };

        match NativeTcpTransport::native_result_to_query_result(response).unwrap() {
            QueryResult::ResultSet { handle, .. } => {
                assert_eq!(handle.map(|h| h.as_i32()), Some(42))
            }
            other => panic!("expected ResultSet, got {other:?}"),
        }
    }

    /// Scenario: Large result set (multi-fetch)
    #[test]
    fn inline_rows_seed_the_fetch_position_of_a_large_result_set() {
        let mut transport = NativeTcpTransport::new();
        let response = |handle, rows_received| NativeResponse::ResultSet {
            handle,
            columns: vec![column_meta("ID", T_DECIMAL)],
            batch: None,
            total_rows: 70,
            rows_received,
        };

        transport
            .convert_and_cache_result(response(42, 67))
            .unwrap();
        transport
            .convert_and_cache_result(response(SMALL_RESULTSET, 70))
            .unwrap();

        assert_eq!(transport.fetch_positions.get(&42), Some(&67));
        assert_eq!(transport.fetch_positions.get(&SMALL_RESULTSET), None);
    }

    #[test]
    fn row_count_and_empty_responses_become_row_counts() {
        assert!(matches!(
            NativeTcpTransport::native_result_to_query_result(NativeResponse::RowCount(7)).unwrap(),
            QueryResult::RowCount { count: 7 }
        ));
        assert!(matches!(
            NativeTcpTransport::native_result_to_query_result(NativeResponse::Empty).unwrap(),
            QueryResult::RowCount { count: 0 }
        ));
    }

    #[test]
    fn a_still_executing_response_is_not_a_query_result() {
        let err = NativeTcpTransport::native_result_to_query_result(NativeResponse::StillExecuting)
            .unwrap_err();

        match err {
            TransportError::ProtocolError(msg) => {
                assert!(msg.contains("Unexpected native response"), "{msg}")
            }
            other => panic!("expected ProtocolError, got {other:?}"),
        }
    }

    #[test]
    fn a_prepared_statement_response_is_not_a_query_result() {
        let response = NativeResponse::PreparedStatement {
            handle: 9,
            parameters: vec![column_meta("P1", T_DECIMAL)],
            result_columns: vec![column_meta("ID", T_DECIMAL)],
        };

        let err = NativeTcpTransport::native_result_to_query_result(response).unwrap_err();

        match err {
            TransportError::ProtocolError(msg) => {
                assert!(msg.contains("PreparedStatement"), "{msg}")
            }
            other => panic!("expected ProtocolError, got {other:?}"),
        }
    }

    // --- Mapping declared and inferred parameter types ---

    fn data_type(type_name: &str) -> super::super::messages::DataType {
        super::super::messages::DataType {
            type_name: type_name.to_string(),
            precision: None,
            scale: None,
            size: None,
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        }
    }

    #[test]
    fn declared_parameter_types_map_to_their_wire_types() {
        let cases = [
            ("DECIMAL", T_DECIMAL),
            ("DOUBLE", T_DOUBLE),
            ("BOOLEAN", T_BOOLEAN),
            ("VARCHAR", T_CHAR),
            ("CHAR", T_CHAR),
            ("DATE", T_DATE),
            ("TIMESTAMP", T_TIMESTAMP),
            ("TIMESTAMP WITH LOCAL TIME ZONE", T_TIMESTAMP_UTC),
            ("GEOMETRY", T_GEOMETRY),
            ("HASHTYPE", T_HASHTYPE),
            ("INTERVAL YEAR TO MONTH", T_INTERVAL_YEAR),
            ("INTERVAL DAY TO SECOND", T_INTERVAL_DAY),
            ("SOMETHING ELSE", T_CHAR),
        ];

        for (type_name, expected) in cases {
            assert_eq!(
                NativeTcpTransport::data_type_to_wire_type(&data_type(type_name)),
                expected,
                "{type_name}"
            );
        }
    }

    #[test]
    fn json_values_infer_their_wire_types() {
        let cases = [
            (serde_json::Value::Null, T_CHAR),
            (serde_json::json!(true), T_BOOLEAN),
            (serde_json::json!(7), T_DECIMAL),
            (serde_json::json!(1.5), T_DOUBLE),
            (serde_json::json!("text"), T_CHAR),
            (serde_json::json!([1, 2]), T_CHAR),
        ];

        for (value, expected) in cases {
            assert_eq!(
                NativeTcpTransport::json_value_to_wire_type(&value),
                expected,
                "{value}"
            );
        }
    }

    #[test]
    fn declared_parameter_metadata_supplies_the_wire_type_and_name() {
        let handle = PreparedStatementHandle::new(
            1,
            2,
            vec![data_type("BOOLEAN"), data_type("DOUBLE")],
            vec![Some("FLAG".to_string()), None],
        );

        assert_eq!(
            NativeTcpTransport::infer_wire_type(&handle, 0, &[]),
            (T_BOOLEAN, "FLAG".to_string())
        );
        assert_eq!(
            NativeTcpTransport::infer_wire_type(&handle, 1, &[]),
            (T_DOUBLE, "param1".to_string())
        );
    }

    #[test]
    fn parameters_beyond_the_declared_metadata_are_inferred_from_their_values() {
        let handle = PreparedStatementHandle::new(1, 0, Vec::new(), Vec::new());

        assert_eq!(
            NativeTcpTransport::infer_wire_type(&handle, 0, &[serde_json::json!(true)]),
            (T_BOOLEAN, "param0".to_string())
        );
        assert_eq!(
            NativeTcpTransport::infer_wire_type(&handle, 1, &[]),
            (T_CHAR, "param1".to_string())
        );
    }

    #[test]
    fn decimal_metadata_falls_back_to_eighteen_zero() {
        let declared = super::super::messages::DataType::decimal(12, 3);
        let handle = PreparedStatementHandle::new(1, 1, vec![declared], vec![None]);

        assert_eq!(NativeTcpTransport::decimal_metadata(&handle, 0), (12, 3));
        assert_eq!(NativeTcpTransport::decimal_metadata(&handle, 1), (18, 0));
    }

    // --- Encoding parameter values ---

    fn encoded_param(wire_type: u32, value: serde_json::Value, scale: i32) -> Vec<u8> {
        let mut buf = Vec::new();
        NativeTcpTransport::write_param_value(&mut buf, wire_type, &value, scale).unwrap();
        buf
    }

    #[test]
    fn a_null_parameter_is_a_lone_null_marker() {
        assert_eq!(
            encoded_param(T_DECIMAL, serde_json::Value::Null, 0),
            vec![0u8]
        );
    }

    #[test]
    fn boolean_parameters_are_encoded_as_one_byte() {
        assert_eq!(
            encoded_param(T_BOOLEAN, serde_json::json!(true), 0),
            vec![1u8, 1u8]
        );
        assert_eq!(
            encoded_param(T_BOOLEAN, serde_json::json!(false), 0),
            vec![1u8, 0u8]
        );
        assert_eq!(
            encoded_param(T_BOOLEAN, serde_json::json!("not a bool"), 0),
            vec![1u8, 0u8]
        );
    }

    #[test]
    fn double_parameters_are_encoded_as_little_endian_f64() {
        let mut expected = vec![1u8];
        expected.extend_from_slice(&1.5f64.to_le_bytes());
        assert_eq!(encoded_param(T_DOUBLE, serde_json::json!(1.5), 0), expected);

        let mut zero = vec![1u8];
        zero.extend_from_slice(&0.0f64.to_le_bytes());
        assert_eq!(encoded_param(T_DOUBLE, serde_json::json!("nope"), 0), zero);
    }

    #[test]
    fn decimal_parameters_are_encoded_as_a_scaled_i64() {
        let mut expected = vec![1u8];
        expected.extend_from_slice(&123i64.to_le_bytes());
        assert_eq!(
            encoded_param(T_DECIMAL, serde_json::json!(1.23), 2),
            expected
        );
    }

    #[test]
    fn date_parameters_are_encoded_as_a_packed_i32() {
        let mut expected = vec![1u8];
        expected.extend_from_slice(&((2024 << 16) + (3 << 8) + 9i32).to_le_bytes());
        assert_eq!(
            encoded_param(T_DATE, serde_json::json!("2024-03-09"), 0),
            expected
        );
    }

    #[test]
    fn string_parameters_are_length_prefixed() {
        let mut expected = vec![1u8];
        expected.extend_from_slice(&5i32.to_le_bytes());
        expected.extend_from_slice(b"hello");
        assert_eq!(
            encoded_param(T_CHAR, serde_json::json!("hello"), 0),
            expected
        );
    }

    #[test]
    fn non_string_values_on_a_string_column_are_encoded_as_their_json_text() {
        let mut expected = vec![1u8];
        expected.extend_from_slice(&2i32.to_le_bytes());
        expected.extend_from_slice(b"42");
        assert_eq!(encoded_param(T_CHAR, serde_json::json!(42), 0), expected);
    }

    #[test]
    fn packed_dates_fall_back_field_by_field() {
        let default = (2000 << 16) + (1 << 8) + 1;

        // Wrong number of dash-separated fields.
        assert_eq!(NativeTcpTransport::parse_date_to_packed("2024-01"), default);
        // Three fields, none of them numeric.
        assert_eq!(
            NativeTcpTransport::parse_date_to_packed("not-a-date"),
            default
        );
        // Only the unparsable field falls back.
        assert_eq!(
            NativeTcpTransport::parse_date_to_packed("2024-xx-05"),
            (2024 << 16) + (1 << 8) + 5
        );
    }

    #[test]
    fn timestamps_encode_their_fractional_seconds_as_nanoseconds() {
        let mut buf = Vec::new();
        NativeTcpTransport::write_timestamp_bytes(&mut buf, "2024-03-09 14:25:36.123456");

        let mut expected = 2024i16.to_le_bytes().to_vec();
        expected.extend_from_slice(&[3, 9, 14, 25, 36]);
        expected.extend_from_slice(&123_456_000i32.to_le_bytes());
        assert_eq!(buf, expected);
    }

    #[test]
    fn timestamps_without_a_time_part_default_to_midnight() {
        let mut buf = Vec::new();
        NativeTcpTransport::write_timestamp_bytes(&mut buf, "2024-03-09");

        let mut expected = 2024i16.to_le_bytes().to_vec();
        expected.extend_from_slice(&[3, 9, 0, 0, 0]);
        expected.extend_from_slice(&0i32.to_le_bytes());
        assert_eq!(buf, expected);
    }

    #[test]
    fn unparsable_timestamp_fields_fall_back_to_their_defaults() {
        let mut buf = Vec::new();
        NativeTcpTransport::write_timestamp_bytes(&mut buf, "bad-value! xx:yy:zz");

        let mut expected = 2000i16.to_le_bytes().to_vec();
        expected.extend_from_slice(&[1, 1, 0, 0, 0]);
        expected.extend_from_slice(&0i32.to_le_bytes());
        assert_eq!(buf, expected);
    }

    // --- CMD_EXECUTE_PREPARED payload ---

    #[test]
    fn prepared_payload_without_parameters_declares_no_columns() {
        let handle = PreparedStatementHandle::new(5, 0, Vec::new(), Vec::new());

        let payload = NativeTcpTransport::build_execute_prepared_payload(&handle, None).unwrap();

        let mut expected = 5i32.to_le_bytes().to_vec();
        expected.extend_from_slice(&1i32.to_le_bytes());
        expected.push(1u8);
        expected.extend_from_slice(&0i32.to_le_bytes());
        expected.extend_from_slice(&0i64.to_le_bytes());
        expected.extend_from_slice(&0i64.to_le_bytes());
        assert_eq!(payload, expected);
    }

    #[test]
    fn prepared_payload_with_an_empty_parameter_list_declares_no_columns() {
        let handle = PreparedStatementHandle::new(5, 0, Vec::new(), Vec::new());

        let payload =
            NativeTcpTransport::build_execute_prepared_payload(&handle, Some(&[])).unwrap();

        assert_eq!(
            payload,
            NativeTcpTransport::build_execute_prepared_payload(&handle, None).unwrap()
        );
    }

    #[test]
    fn prepared_payload_interleaves_parameter_values_row_by_row() {
        let handle = PreparedStatementHandle::new(
            9,
            2,
            vec![data_type("BOOLEAN"), data_type("VARCHAR")],
            vec![Some("FLAG".to_string()), Some("NAME".to_string())],
        );
        let parameters = vec![
            vec![serde_json::json!(true), serde_json::json!(false)],
            vec![serde_json::json!("a"), serde_json::json!("b")],
        ];

        let payload =
            NativeTcpTransport::build_execute_prepared_payload(&handle, Some(&parameters)).unwrap();

        let mut expected = 9i32.to_le_bytes().to_vec();
        expected.extend_from_slice(&1i32.to_le_bytes());
        expected.push(1u8);
        expected.extend_from_slice(&2i32.to_le_bytes()); // num_columns
        expected.extend_from_slice(&2i64.to_le_bytes()); // total_rows
        expected.extend_from_slice(&2i64.to_le_bytes()); // rows_in_msg
                                                         // BOOLEAN column header, no type metadata
        expected.extend_from_slice(&4i32.to_le_bytes());
        expected.extend_from_slice(b"FLAG");
        expected.extend_from_slice(&(T_BOOLEAN as i32).to_le_bytes());
        // VARCHAR column header carries vc_flag, max_len and octet_len
        expected.extend_from_slice(&4i32.to_le_bytes());
        expected.extend_from_slice(b"NAME");
        expected.extend_from_slice(&(T_CHAR as i32).to_le_bytes());
        // Literal, not IS_VARCHAR | IS_UTF8: building the expectation from the
        // same constants the production write uses would pass for any values.
        expected.push(0x11u8);
        expected.extend_from_slice(&2_000_000i32.to_le_bytes());
        expected.extend_from_slice(&(2_000_000i32 * 4).to_le_bytes());
        // Row 0 then row 1, each holding both columns
        expected.extend_from_slice(&[1, 1]);
        expected.extend_from_slice(&[1]);
        expected.extend_from_slice(&1i32.to_le_bytes());
        expected.extend_from_slice(b"a");
        expected.extend_from_slice(&[1, 0]);
        expected.extend_from_slice(&[1]);
        expected.extend_from_slice(&1i32.to_le_bytes());
        expected.extend_from_slice(b"b");

        assert_eq!(payload, expected);
    }

    #[test]
    fn prepared_payload_writes_decimal_precision_and_scales_its_values() {
        let handle = PreparedStatementHandle::new(
            1,
            1,
            vec![super::super::messages::DataType::decimal(12, 2)],
            vec![Some("AMOUNT".to_string())],
        );
        let parameters = vec![vec![serde_json::json!(1.23)]];

        let payload =
            NativeTcpTransport::build_execute_prepared_payload(&handle, Some(&parameters)).unwrap();

        let mut expected = 1i32.to_le_bytes().to_vec();
        expected.extend_from_slice(&1i32.to_le_bytes());
        expected.push(1u8);
        expected.extend_from_slice(&1i32.to_le_bytes());
        expected.extend_from_slice(&1i64.to_le_bytes());
        expected.extend_from_slice(&1i64.to_le_bytes());
        expected.extend_from_slice(&6i32.to_le_bytes());
        expected.extend_from_slice(b"AMOUNT");
        expected.extend_from_slice(&(T_DECIMAL as i32).to_le_bytes());
        expected.extend_from_slice(&12i32.to_le_bytes()); // precision
        expected.extend_from_slice(&2i32.to_le_bytes()); // scale
        expected.push(1u8);
        expected.extend_from_slice(&123i64.to_le_bytes());

        assert_eq!(payload, expected);
    }

    // --- Splitting a parameter set into data messages ---

    const TOTAL_ROWS_BYTES: Range<usize> = 13..21;
    const ROWS_IN_MSG_BYTES: Range<usize> = 21..29;
    const FIXED_PREFIX_LEN: usize = 29;
    const DECIMAL_VALUE_LEN: usize = 1 + 8;

    /// Bytes of one string value: null marker, length, characters.
    const fn string_value_len(chars: usize) -> usize {
        1 + 4 + chars
    }

    fn varchar_handle() -> PreparedStatementHandle {
        PreparedStatementHandle::new(
            9,
            1,
            vec![data_type("VARCHAR")],
            vec![Some("NAME".to_string())],
        )
    }

    fn one_string_column(values: &[String]) -> Vec<Vec<serde_json::Value>> {
        vec![values.iter().map(|v| serde_json::json!(v)).collect()]
    }

    fn message_size(payload: &[u8]) -> usize {
        HEADER_SIZE
            + NativeTcpTransport::execute_prepared_attributes()
                .serialize()
                .len()
            + payload.len()
    }

    fn range_payloads(
        handle: &PreparedStatementHandle,
        encoded: &EncodedParameterRows,
    ) -> Vec<Vec<u8>> {
        encoded
            .ranges
            .iter()
            .map(|range| NativeTcpTransport::build_range_payload(handle, encoded, range))
            .collect()
    }

    /// Scenario: Batch update larger than one data message over the native protocol
    #[test]
    fn prepared_payload_ranges_keep_each_message_within_the_limit() {
        let handle = varchar_handle();
        let parameters = one_string_column(&vec!["x".repeat(100); 10]);
        let single =
            NativeTcpTransport::build_execute_prepared_payload(&handle, Some(&parameters)).unwrap();
        let prefix_len = single.len() - 10 * string_value_len(100);
        let limit = message_size(&single) - 7 * string_value_len(100);

        let encoded =
            NativeTcpTransport::split_parameter_rows(&handle, &parameters, limit).unwrap();
        let payloads = range_payloads(&handle, &encoded);

        let row_ranges: Vec<_> = encoded.ranges.iter().map(|r| r.rows.clone()).collect();
        assert_eq!(row_ranges, vec![0..3, 3..6, 6..9, 9..10]);
        let mut joined_rows = Vec::new();
        for (payload, range) in payloads.iter().zip(&encoded.ranges) {
            assert!(message_size(payload) <= limit);
            let declared = (range.rows.len() as i64).to_le_bytes();
            assert_eq!(payload[TOTAL_ROWS_BYTES], declared, "total_rows");
            assert_eq!(payload[ROWS_IN_MSG_BYTES], declared, "rows_in_msg");
            joined_rows.extend_from_slice(&payload[prefix_len..]);
        }
        assert_eq!(joined_rows, single[prefix_len..]);
    }

    /// Scenario: Batch update larger than one data message over the native protocol
    #[test]
    fn prepared_payload_without_rows_forms_no_range() {
        let encoded = NativeTcpTransport::split_parameter_rows(
            &varchar_handle(),
            &one_string_column(&[]),
            usize::MAX,
        )
        .unwrap();

        assert!(encoded.ranges.is_empty());
    }

    /// Scenario: Batch update larger than one data message over the native protocol
    #[test]
    fn prepared_payload_that_fits_forms_one_range() {
        let handle = varchar_handle();
        let parameters = one_string_column(&vec!["x".repeat(100); 10]);
        let single =
            NativeTcpTransport::build_execute_prepared_payload(&handle, Some(&parameters)).unwrap();

        let encoded =
            NativeTcpTransport::split_parameter_rows(&handle, &parameters, message_size(&single))
                .unwrap();
        let payloads = range_payloads(&handle, &encoded);

        assert_eq!(payloads, vec![single]);
    }

    /// Scenario: Batch update larger than one data message over the native protocol
    #[test]
    fn prepared_payload_row_above_the_limit_forms_its_own_range() {
        let handle = varchar_handle();
        let values = ["x".repeat(10), "y".repeat(1000), "z".repeat(10)];
        let parameters = one_string_column(&values);
        let single =
            NativeTcpTransport::build_execute_prepared_payload(&handle, Some(&parameters)).unwrap();
        let prefix_len =
            single.len() - (string_value_len(10) + string_value_len(1000) + string_value_len(10));
        let limit = message_size(&single[..prefix_len]) + 2 * string_value_len(10);

        let encoded =
            NativeTcpTransport::split_parameter_rows(&handle, &parameters, limit).unwrap();

        let row_ranges: Vec<_> = encoded.ranges.iter().map(|r| r.rows.clone()).collect();
        assert_eq!(row_ranges, vec![0..1, 1..2, 2..3]);
    }

    /// Scenario: Batch update larger than one data message over the native protocol
    #[test]
    fn prepared_payload_ranges_reuse_the_wire_types_of_the_whole_batch() {
        let handle = PreparedStatementHandle::new(1, 1, Vec::new(), Vec::new());
        // The first value infers DECIMAL for the column. The first value of the
        // second range would infer DOUBLE if each range inferred its own type.
        let parameters = vec![vec![
            serde_json::json!(1),
            serde_json::json!(2),
            serde_json::json!(1.5),
            serde_json::json!(2.5),
        ]];
        let single =
            NativeTcpTransport::build_execute_prepared_payload(&handle, Some(&parameters)).unwrap();
        let prefix_len = single.len() - 4 * DECIMAL_VALUE_LEN;
        let limit = message_size(&single[..prefix_len]) + 2 * DECIMAL_VALUE_LEN;

        let encoded =
            NativeTcpTransport::split_parameter_rows(&handle, &parameters, limit).unwrap();
        let payloads = range_payloads(&handle, &encoded);

        assert_eq!(payloads.len(), 2);
        for payload in &payloads {
            assert_eq!(
                payload[FIXED_PREFIX_LEN..prefix_len],
                single[FIXED_PREFIX_LEN..prefix_len]
            );
        }
        assert_eq!(
            payloads[1][prefix_len..],
            single[prefix_len + 2 * DECIMAL_VALUE_LEN..]
        );
    }

    /// Scenario: Batch update larger than one data message over the native protocol
    #[test]
    fn range_results_add_up_row_counts() {
        let mut total = 0;

        assert!(
            NativeTcpTransport::add_range_result(&mut total, QueryResult::row_count(10)).is_none()
        );
        assert!(
            NativeTcpTransport::add_range_result(&mut total, QueryResult::row_count(20)).is_none()
        );

        assert_eq!(total, 30);
    }

    #[test]
    fn a_result_set_ends_the_range_batch() {
        let mut total = 5;
        let result_set = QueryResult::ResultSet {
            handle: None,
            data: ResultData {
                columns: Vec::new(),
                data: ResultPayload::Json(Vec::new()),
                total_rows: 0,
            },
        };

        let stop = NativeTcpTransport::add_range_result(&mut total, result_set);

        assert!(matches!(stop, Some(QueryResult::ResultSet { .. })));
        assert_eq!(total, 5);
    }

    #[test]
    fn max_data_message_size_defaults_before_login() {
        assert_eq!(
            NativeTcpTransport::new().max_data_message_size(),
            MAX_DATA_MESSAGE_SIZE as i64
        );
    }

    #[test]
    fn reject_exception_reports_the_context_message_and_sql_state() {
        let message = "invalid credentials";
        let mut payload = 1i32.to_le_bytes().to_vec();
        payload.push(constants::R_EXCEPTION as u8);
        payload.extend_from_slice(&(message.len() as i32).to_le_bytes());
        payload.extend_from_slice(message.as_bytes());
        payload.extend_from_slice(b"08004");

        let err = reject_exception("Authentication failed", &payload).unwrap_err();

        match err {
            TransportError::ProtocolError(msg) => assert_eq!(
                msg,
                "Authentication failed: invalid credentials (SQL state: 08004)"
            ),
            other => panic!("expected ProtocolError, got {other:?}"),
        }
    }
}
