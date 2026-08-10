use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryBuilder, BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder,
    Int32Builder, Int64Builder, StringBuilder, TimestampMicrosecondBuilder,
};
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema, TimeUnit};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};

use crate::error::TransportError;

use super::constants::{
    IS_VARCHAR, PARAMETER_DESCRIPTION, R_COLUMN_COUNT, R_EMPTY, R_EXCEPTION, R_HANDLE, R_MORE_ROWS,
    R_RESULT_SET, R_ROW_COUNT, R_STILL_EXECUTING, R_WARNING, T_BIGDECIMAL, T_BINARY, T_BOOLEAN,
    T_CHAR, T_DATE, T_DECIMAL, T_DOUBLE, T_GEOMETRY, T_HASHTYPE, T_INTEGER, T_INTERVAL_DAY,
    T_INTERVAL_YEAR, T_REAL, T_SMALLDECIMAL, T_SMALLINT, T_TIMESTAMP, T_TIMESTAMP_LOCAL_TZ,
    T_TIMESTAMP_UTC,
};

/// One decoded column: the Arrow field describing it and the array holding its values.
type BuiltColumn = (Field, ArrayRef);

/// Parsed column metadata from a native protocol result set.
#[derive(Debug, Clone)]
pub struct NativeColumnMeta {
    pub name: String,
    pub type_id: u32,
    pub precision: Option<i32>,
    pub scale: Option<i32>,
    pub is_varchar: bool,
    pub max_len: Option<i32>,
}

/// Parsed response from the native protocol.
#[derive(Debug)]
pub enum NativeResponse {
    ResultSet {
        handle: i32,
        columns: Vec<NativeColumnMeta>,
        batch: Option<arrow::record_batch::RecordBatch>,
        total_rows: i64,
        rows_received: i64,
    },
    /// Reply to `CMD_CREATE_PREPARED`: two independent column descriptions and no
    /// row data. This is a shape of its own because `ResultSet` holds a single
    /// column list, which forced one of the two descriptions to be discarded.
    PreparedStatement {
        handle: i32,
        parameters: Vec<NativeColumnMeta>,
        result_columns: Vec<NativeColumnMeta>,
    },
    RowCount(i64),
    Empty,
    StillExecuting,
    MoreRows(Vec<u8>),
    Exception {
        message: String,
        sql_state: String,
    },
}

/// Non-fatal warning returned alongside a terminal result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeWarning {
    pub message: String,
    pub sql_state: String,
}

/// Parsed native response envelope containing warnings and one terminal result.
#[derive(Debug)]
pub struct NativeResponseEnvelope {
    pub warnings: Vec<NativeWarning>,
    pub terminal: NativeResponse,
}

/// Parse a response payload (after the 21-byte header) into a structured response.
pub fn parse_response(data: &[u8]) -> Result<NativeResponseEnvelope, TransportError> {
    if data.is_empty() {
        return Ok(NativeResponseEnvelope {
            warnings: Vec::new(),
            terminal: NativeResponse::Empty,
        });
    }

    if data.len() == 4 && i32::from_le_bytes([data[0], data[1], data[2], data[3]]) == 0 {
        return Ok(NativeResponseEnvelope {
            warnings: Vec::new(),
            terminal: NativeResponse::Empty,
        });
    }

    if let Some(envelope) = try_parse_counted_envelope(data)? {
        return Ok(envelope);
    }

    parse_legacy_response(data)
}

fn try_parse_counted_envelope(
    data: &[u8],
) -> Result<Option<NativeResponseEnvelope>, TransportError> {
    if data.len() < 5 {
        return Ok(None);
    }

    let mut probe_offset = 0;
    let result_count = read_i32(data, &mut probe_offset)?;
    if !(1..=64).contains(&result_count) {
        return Ok(None);
    }

    let first_part_type = data[probe_offset] as i8;
    if !is_known_response_type(first_part_type) {
        return Ok(None);
    }

    let mut offset = 0;
    let result_count = read_i32(data, &mut offset)?;

    let mut warnings = Vec::new();
    let mut terminal = None;

    for part_idx in 0..(result_count as usize) {
        let result_type = read_u8(data, &mut offset)? as i8;
        match result_type {
            R_WARNING => warnings.push(parse_warning(data, &mut offset)?),
            R_COLUMN_COUNT => {
                let _ = read_i32(data, &mut offset)?;
            }
            R_RESULT_SET => {
                let response = parse_result_set_at(data, &mut offset)?;
                assign_terminal_result(&mut terminal, response)?;
            }
            R_HANDLE => {
                let handle_envelope = parse_handle_only_at(data, &mut offset)?;
                warnings.extend(handle_envelope.warnings);
                assign_terminal_result(&mut terminal, handle_envelope.terminal)?;
            }
            R_ROW_COUNT => {
                let response = parse_row_count_at(data, &mut offset)?;
                assign_terminal_result(&mut terminal, response)?;
            }
            R_EXCEPTION => {
                let response = parse_exception_at(data, &mut offset)?;
                assign_terminal_result(&mut terminal, response)?;
            }
            R_EMPTY => {
                assign_terminal_result(&mut terminal, NativeResponse::Empty)?;
            }
            R_STILL_EXECUTING => {
                assign_terminal_result(&mut terminal, NativeResponse::StillExecuting)?;
            }
            R_MORE_ROWS => {
                if part_idx + 1 != result_count as usize {
                    return Err(TransportError::ProtocolError(
                        "MoreRows must be the final native response part".into(),
                    ));
                }
                let response = NativeResponse::MoreRows(data[offset..].to_vec());
                assign_terminal_result(&mut terminal, response)?;
                offset = data.len();
            }
            _ => {
                let preview_end = data.len().min(64);
                return Err(TransportError::ProtocolError(format!(
                    "Unknown response type: {} at offset {} (data {:02x?})",
                    result_type,
                    offset.saturating_sub(1),
                    &data[..preview_end]
                )));
            }
        }
    }

    if offset != data.len() {
        return Ok(None);
    }

    Ok(Some(NativeResponseEnvelope {
        warnings,
        terminal: terminal.unwrap_or(NativeResponse::Empty),
    }))
}

fn parse_legacy_response(data: &[u8]) -> Result<NativeResponseEnvelope, TransportError> {
    let mut offset = 0;
    let response = parse_legacy_response_at(data, &mut offset)?;
    if offset != data.len() {
        return Err(TransportError::ProtocolError(format!(
            "Trailing bytes after legacy response: remaining {:02x?}",
            &data[offset..]
        )));
    }
    Ok(response)
}

fn parse_legacy_response_at(
    data: &[u8],
    offset: &mut usize,
) -> Result<NativeResponseEnvelope, TransportError> {
    let result_type = read_u8(data, offset)? as i8;
    parse_legacy_response_body(data, offset, result_type)
}

fn parse_legacy_response_body(
    data: &[u8],
    offset: &mut usize,
    result_type: i8,
) -> Result<NativeResponseEnvelope, TransportError> {
    match result_type {
        R_RESULT_SET => Ok(NativeResponseEnvelope {
            warnings: Vec::new(),
            terminal: parse_result_set_at(data, offset)?,
        }),
        R_HANDLE => parse_handle_only_at(data, offset),
        R_ROW_COUNT => Ok(NativeResponseEnvelope {
            warnings: Vec::new(),
            terminal: parse_row_count_at(data, offset)?,
        }),
        R_EXCEPTION => Ok(NativeResponseEnvelope {
            warnings: Vec::new(),
            terminal: parse_exception_at(data, offset)?,
        }),
        R_WARNING => Ok(NativeResponseEnvelope {
            warnings: { vec![parse_warning(data, offset)?] },
            terminal: NativeResponse::Empty,
        }),
        R_COLUMN_COUNT => {
            let _ = read_i32(data, offset)?;
            Ok(NativeResponseEnvelope {
                warnings: Vec::new(),
                terminal: NativeResponse::Empty,
            })
        }
        R_EMPTY => Ok(NativeResponseEnvelope {
            warnings: Vec::new(),
            terminal: NativeResponse::Empty,
        }),
        R_STILL_EXECUTING => Ok(NativeResponseEnvelope {
            warnings: Vec::new(),
            terminal: {
                consume_status_message(data, offset)?;
                NativeResponse::StillExecuting
            },
        }),
        R_MORE_ROWS => {
            let remaining = data[*offset..].to_vec();
            *offset = data.len();
            Ok(NativeResponseEnvelope {
                warnings: Vec::new(),
                terminal: NativeResponse::MoreRows(remaining),
            })
        }
        _ => Err(TransportError::ProtocolError(format!(
            "Unknown response type: {}",
            result_type
        ))),
    }
}

fn is_known_response_type(result_type: i8) -> bool {
    matches!(
        result_type,
        R_RESULT_SET
            | R_HANDLE
            | R_ROW_COUNT
            | R_COLUMN_COUNT
            | R_WARNING
            | R_STILL_EXECUTING
            | R_MORE_ROWS
            | R_EXCEPTION
            | R_EMPTY
    )
}

fn consume_status_message(data: &[u8], offset: &mut usize) -> Result<(), TransportError> {
    if *offset >= data.len() {
        return Ok(());
    }
    let msg_len = read_i32(data, offset)? as usize;
    if *offset + msg_len > data.len() {
        return Err(TransportError::ProtocolError(
            "StillExecuting status message truncated".into(),
        ));
    }
    *offset += msg_len;
    Ok(())
}

fn assign_terminal_result(
    slot: &mut Option<NativeResponse>,
    response: NativeResponse,
) -> Result<(), TransportError> {
    if slot.is_some() {
        return Err(TransportError::ProtocolError(
            "Multiple terminal results in native response envelope".into(),
        ));
    }
    *slot = Some(response);
    Ok(())
}

/// Parse a handle-only response (R_HANDLE = 2), sent in reply to CREATE PREPARED.
///
/// Format: [statement_handle:4 LE] [sub_result...]
///
/// A result-set sub-result whose handle is `PARAMETER_DESCRIPTION` describes the
/// parameters; any other describes the statement's result-set columns. An
/// exception sub-result is reported as an error rather than as a handle with
/// empty metadata, which the caller would otherwise bind against as though the
/// server had confirmed it.
fn parse_handle_only_at(
    data: &[u8],
    offset: &mut usize,
) -> Result<NativeResponseEnvelope, TransportError> {
    let statement_handle = read_i32(data, offset)?;

    let mut warnings = Vec::new();
    let mut parameters = Vec::new();
    let mut result_columns = Vec::new();
    while *offset < data.len() {
        let sub_response = parse_legacy_response_at(data, offset)?;
        warnings.extend(sub_response.warnings);
        match sub_response.terminal {
            NativeResponse::ResultSet {
                handle: sub_handle,
                columns,
                ..
            } => {
                if sub_handle == PARAMETER_DESCRIPTION {
                    parameters = columns;
                } else {
                    result_columns = columns;
                }
            }
            NativeResponse::Exception { message, sql_state } => {
                return Err(TransportError::ProtocolError(format!(
                    "Exasol reported an exception in a prepared-statement reply sub-result: {message} (SQLSTATE {sql_state})"
                )));
            }
            _ => {}
        }
    }

    Ok(NativeResponseEnvelope {
        warnings,
        terminal: NativeResponse::PreparedStatement {
            handle: statement_handle,
            parameters,
            result_columns,
        },
    })
}

#[cfg(test)]
fn parse_result_set(data: &[u8]) -> Result<NativeResponse, TransportError> {
    parse_result_set_at_end(data)
}

#[cfg(test)]
fn parse_result_set_at_end(data: &[u8]) -> Result<NativeResponse, TransportError> {
    let mut offset = 0;
    let response = parse_result_set_at(data, &mut offset)?;
    if offset != data.len() {
        return Err(TransportError::ProtocolError(format!(
            "Trailing bytes after result-set response: remaining {:02x?}",
            &data[offset..]
        )));
    }
    Ok(response)
}

fn parse_result_set_at(data: &[u8], offset: &mut usize) -> Result<NativeResponse, TransportError> {
    let handle = read_i32(data, offset)?;
    let num_columns = read_i32(data, offset)? as usize;
    let total_rows = read_i64(data, offset)?;
    let rows_received = read_i64(data, offset)?;
    if rows_received < 0 {
        return Err(TransportError::ProtocolError(format!(
            "Invalid row count from server: {}",
            rows_received
        )));
    }

    let mut columns = Vec::with_capacity(num_columns);
    for _ in 0..num_columns {
        columns.push(parse_column_meta(data, offset)?);
    }

    let batch = if num_columns == 0 {
        None
    } else {
        Some(build_batch_from_wire(
            data,
            offset,
            &columns,
            rows_received as usize,
        )?)
    };

    Ok(NativeResponse::ResultSet {
        handle,
        columns,
        batch,
        total_rows,
        rows_received,
    })
}

fn parse_column_meta(data: &[u8], offset: &mut usize) -> Result<NativeColumnMeta, TransportError> {
    let name_len = read_i32(data, offset)? as usize;
    if *offset + name_len > data.len() {
        return Err(TransportError::ProtocolError(
            "Column name truncated".into(),
        ));
    }
    let name = std::str::from_utf8(&data[*offset..*offset + name_len])
        .map_err(|e| TransportError::ProtocolError(format!("Invalid column name UTF-8: {}", e)))?
        .to_owned();
    *offset += name_len;

    let type_id = read_i32(data, offset)? as u32;

    let mut precision = None;
    let mut scale = None;
    let mut is_varchar = false;
    let mut max_len = None;

    match type_id {
        T_DECIMAL | T_SMALLDECIMAL | T_BIGDECIMAL => {
            precision = Some(read_i32(data, offset)?);
            scale = Some(read_i32(data, offset)?);
        }
        T_CHAR | T_GEOMETRY | T_HASHTYPE => {
            // All string-like types: vcFlag(1) + maxLen(4) + octetLen(4)
            if *offset >= data.len() {
                return Err(TransportError::ProtocolError(
                    "CHAR metadata truncated".into(),
                ));
            }
            let vc_flag = data[*offset];
            *offset += 1;
            is_varchar = (vc_flag & IS_VARCHAR) != 0;
            max_len = Some(read_i32(data, offset)?);
            // octet_len
            let _octet_len = read_i32(data, offset)?;
        }
        T_INTERVAL_YEAR => {
            // vcFlag(1) + maxLen(4) + octetLen(4)
            if *offset >= data.len() {
                return Err(TransportError::ProtocolError(
                    "INTERVAL metadata truncated".into(),
                ));
            }
            let vc_flag = data[*offset];
            *offset += 1;
            is_varchar = (vc_flag & IS_VARCHAR) != 0;
            max_len = Some(read_i32(data, offset)?);
            let _octet_len = read_i32(data, offset)?;
            // Protocol v19+: datetimeIntervalPrecision(4)
            let _interval_precision = read_i32(data, offset)?;
        }
        T_INTERVAL_DAY => {
            // vcFlag(1) + maxLen(4) + octetLen(4)
            if *offset >= data.len() {
                return Err(TransportError::ProtocolError(
                    "INTERVAL metadata truncated".into(),
                ));
            }
            let vc_flag = data[*offset];
            *offset += 1;
            is_varchar = (vc_flag & IS_VARCHAR) != 0;
            max_len = Some(read_i32(data, offset)?);
            let _octet_len = read_i32(data, offset)?;
            // Protocol v19+: datetimeIntervalPrecision(4) + datetimeIntervalFraction(4)
            let _interval_precision = read_i32(data, offset)?;
            let _interval_fraction = read_i32(data, offset)?;
        }
        T_TIMESTAMP | T_TIMESTAMP_LOCAL_TZ | T_TIMESTAMP_UTC => {
            // Protocol v19+: precision(4)
            let _ts_precision = read_i32(data, offset)?;
        }
        _ => {}
    }

    Ok(NativeColumnMeta {
        name,
        type_id,
        precision,
        scale,
        is_varchar,
        max_len,
    })
}

fn parse_row_count_at(data: &[u8], offset: &mut usize) -> Result<NativeResponse, TransportError> {
    let count = read_i64(data, offset)?;
    Ok(NativeResponse::RowCount(count))
}

fn parse_exception_at(data: &[u8], offset: &mut usize) -> Result<NativeResponse, TransportError> {
    let warning = parse_message_part(data, offset)?;
    Ok(NativeResponse::Exception {
        message: warning.message,
        sql_state: warning.sql_state,
    })
}

fn parse_warning(data: &[u8], offset: &mut usize) -> Result<NativeWarning, TransportError> {
    parse_message_part(data, offset)
}

fn parse_message_part(data: &[u8], offset: &mut usize) -> Result<NativeWarning, TransportError> {
    let msg_len = read_i32(data, offset)? as usize;
    if *offset + msg_len > data.len() {
        return Err(TransportError::ProtocolError(
            "Exception message truncated".into(),
        ));
    }
    let message = std::str::from_utf8(&data[*offset..*offset + msg_len])
        .map_err(|e| TransportError::ProtocolError(format!("Invalid exception UTF-8: {}", e)))?
        .to_owned();
    *offset += msg_len;

    let sql_state = if *offset + 5 <= data.len() {
        std::str::from_utf8(&data[*offset..*offset + 5])
            .unwrap_or("?????")
            .to_owned()
    } else {
        "?????".to_owned()
    };
    if *offset + 5 <= data.len() {
        *offset += 5;
    }

    Ok(NativeWarning { message, sql_state })
}

/// Build an Arrow `RecordBatch` directly from native wire bytes in a single pass,
/// appending values straight into typed Arrow builders. DATE → Date32,
/// TIMESTAMP → TimestampMicrosecond; no intermediate string allocations.
fn build_batch_from_wire(
    data: &[u8],
    offset: &mut usize,
    column_metas: &[NativeColumnMeta],
    num_rows: usize,
) -> Result<RecordBatch, TransportError> {
    if column_metas.is_empty() {
        return RecordBatch::try_new_with_options(
            Arc::new(Schema::empty()),
            vec![],
            &RecordBatchOptions::new().with_row_count(Some(num_rows)),
        )
        .map_err(|e| {
            TransportError::ProtocolError(format!("Failed to create empty RecordBatch: {}", e))
        });
    }

    let mut fields = Vec::with_capacity(column_metas.len());
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(column_metas.len());

    for meta in column_metas {
        let (field, array) = fill_column_builder(data, offset, meta, num_rows)?;
        fields.push(field);
        arrays.push(array);
    }

    RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
        .map_err(|e| TransportError::ProtocolError(format!("Failed to create RecordBatch: {}", e)))
}

/// Extract Arrow Decimal128 precision and scale from column metadata, clamping
/// to the valid Arrow range (precision 1..=38, scale in i8) and defaulting
/// precision when absent.
fn decimal_precision_scale(meta: &NativeColumnMeta, default_precision: i32) -> (u8, i8) {
    let raw_precision = meta.precision.unwrap_or(default_precision);
    let precision = raw_precision.clamp(1, 38) as u8;
    let raw_scale = meta.scale.unwrap_or(0);
    let scale = raw_scale.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    (precision, scale)
}

/// Parse one column worth of wire bytes directly into an Arrow array.
///
/// Types with no binary wire representation of their own — CHAR, GEOMETRY, HASHTYPE,
/// both INTERVAL kinds, and any type id this driver does not recognise — decode as
/// length-prefixed UTF-8 strings.
fn fill_column_builder(
    data: &[u8],
    offset: &mut usize,
    meta: &NativeColumnMeta,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    match meta.type_id {
        T_DOUBLE => fill_double_column(data, offset, &meta.name, num_rows),
        T_REAL => fill_real_column(data, offset, &meta.name, num_rows),
        T_INTEGER => fill_integer_column(data, offset, &meta.name, num_rows),
        T_SMALLINT => fill_smallint_column(data, offset, &meta.name, num_rows),
        T_BOOLEAN => fill_boolean_column(data, offset, &meta.name, num_rows),
        T_BINARY => fill_binary_column(data, offset, &meta.name, num_rows),
        T_SMALLDECIMAL => fill_smalldecimal_column(data, offset, meta, num_rows),
        T_DECIMAL => fill_decimal_column(data, offset, meta, num_rows),
        T_BIGDECIMAL => fill_bigdecimal_column(data, offset, meta, num_rows),
        T_DATE => fill_date_column(data, offset, &meta.name, num_rows),
        T_TIMESTAMP | T_TIMESTAMP_LOCAL_TZ | T_TIMESTAMP_UTC => {
            fill_timestamp_column(data, offset, meta, num_rows)
        }
        _ => fill_string_builder(data, offset, &meta.name, num_rows),
    }
}

fn fill_double_column(
    data: &[u8],
    offset: &mut usize,
    name: &str,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let mut builder = Float64Builder::with_capacity(num_rows);
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else {
            builder.append_value(read_f64(data, offset)?);
        }
    }
    Ok((
        Field::new(name, ArrowDataType::Float64, true),
        Arc::new(builder.finish()),
    ))
}

fn fill_real_column(
    data: &[u8],
    offset: &mut usize,
    name: &str,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let mut builder = Float64Builder::with_capacity(num_rows);
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else {
            builder.append_value(read_f32(data, offset)? as f64);
        }
    }
    Ok((
        Field::new(name, ArrowDataType::Float64, true),
        Arc::new(builder.finish()),
    ))
}

fn fill_integer_column(
    data: &[u8],
    offset: &mut usize,
    name: &str,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let mut builder = Int64Builder::with_capacity(num_rows);
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else {
            builder.append_value(read_i64(data, offset)?);
        }
    }
    Ok((
        Field::new(name, ArrowDataType::Int64, true),
        Arc::new(builder.finish()),
    ))
}

fn fill_smallint_column(
    data: &[u8],
    offset: &mut usize,
    name: &str,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let mut builder = Int32Builder::with_capacity(num_rows);
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else {
            builder.append_value(read_i32(data, offset)?);
        }
    }
    Ok((
        Field::new(name, ArrowDataType::Int32, true),
        Arc::new(builder.finish()),
    ))
}

fn fill_boolean_column(
    data: &[u8],
    offset: &mut usize,
    name: &str,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let mut builder = BooleanBuilder::with_capacity(num_rows);
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else {
            builder.append_value(read_u8(data, offset)? != 0);
        }
    }
    Ok((
        Field::new(name, ArrowDataType::Boolean, true),
        Arc::new(builder.finish()),
    ))
}

fn fill_binary_column(
    data: &[u8],
    offset: &mut usize,
    name: &str,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let mut builder = BinaryBuilder::with_capacity(num_rows, num_rows * 16);
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else {
            builder.append_value(read_length_prefixed(data, offset, "Binary data truncated")?);
        }
    }
    Ok((
        Field::new(name, ArrowDataType::Binary, true),
        Arc::new(builder.finish()),
    ))
}

fn fill_smalldecimal_column(
    data: &[u8],
    offset: &mut usize,
    meta: &NativeColumnMeta,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let data_type = decimal_data_type(meta, 9);
    let mut builder = Decimal128Builder::with_capacity(num_rows).with_data_type(data_type.clone());
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else {
            builder.append_value(read_i32(data, offset)? as i128);
        }
    }
    Ok((
        Field::new(&meta.name, data_type, true),
        Arc::new(builder.finish()),
    ))
}

fn fill_decimal_column(
    data: &[u8],
    offset: &mut usize,
    meta: &NativeColumnMeta,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let data_type = decimal_data_type(meta, 18);
    let mut builder = Decimal128Builder::with_capacity(num_rows).with_data_type(data_type.clone());
    let use_i32 = meta.precision.unwrap_or(18) <= 9;
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else if use_i32 {
            builder.append_value(read_i32(data, offset)? as i128);
        } else {
            builder.append_value(read_i64(data, offset)? as i128);
        }
    }
    Ok((
        Field::new(&meta.name, data_type, true),
        Arc::new(builder.finish()),
    ))
}

fn fill_bigdecimal_column(
    data: &[u8],
    offset: &mut usize,
    meta: &NativeColumnMeta,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let data_type = decimal_data_type(meta, 36);
    let mut builder = Decimal128Builder::with_capacity(num_rows).with_data_type(data_type.clone());
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else {
            builder.append_value(read_i128(data, offset)?);
        }
    }
    Ok((
        Field::new(&meta.name, data_type, true),
        Arc::new(builder.finish()),
    ))
}

fn decimal_data_type(meta: &NativeColumnMeta, default_precision: i32) -> ArrowDataType {
    let (precision, scale) = decimal_precision_scale(meta, default_precision);
    ArrowDataType::Decimal128(precision, scale)
}

fn fill_date_column(
    data: &[u8],
    offset: &mut usize,
    name: &str,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let mut builder = Date32Builder::with_capacity(num_rows);
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else {
            builder.append_value(read_packed_date_days(data, offset)?);
        }
    }
    Ok((
        Field::new(name, ArrowDataType::Date32, true),
        Arc::new(builder.finish()),
    ))
}

fn fill_timestamp_column(
    data: &[u8],
    offset: &mut usize,
    meta: &NativeColumnMeta,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let mut builder = TimestampMicrosecondBuilder::with_capacity(num_rows);
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else {
            builder.append_value(read_timestamp_micros(data, offset)?);
        }
    }

    let finished = builder.finish();
    if meta.type_id == T_TIMESTAMP_UTC {
        return Ok((
            Field::new(
                &meta.name,
                ArrowDataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                true,
            ),
            Arc::new(finished.with_timezone("UTC")),
        ));
    }
    Ok((
        Field::new(
            &meta.name,
            ArrowDataType::Timestamp(TimeUnit::Microsecond, None),
            true,
        ),
        Arc::new(finished),
    ))
}

/// Decode the packed `[year:16][month:8][day:8]` DATE value into Arrow Date32 days.
fn read_packed_date_days(data: &[u8], offset: &mut usize) -> Result<i32, TransportError> {
    let packed = read_i32(data, offset)?;
    let year = packed >> 16;
    let month = ((packed >> 8) & 0xFF) as u32;
    let day = (packed & 0xFF) as u32;
    Ok(crate::types::conversion::ymd_to_days(year, month, day))
}

fn read_timestamp_micros(data: &[u8], offset: &mut usize) -> Result<i64, TransportError> {
    let year = read_i16(data, offset)? as i32;
    let month = read_u8(data, offset)? as u32;
    let day = read_u8(data, offset)? as u32;
    let hour = read_u8(data, offset)? as u64;
    let minute = read_u8(data, offset)? as u64;
    let second = read_u8(data, offset)? as u64;
    let nanos = read_i32(data, offset)?;
    Ok(crate::types::conversion::ymd_hms_nanos_to_micros(
        year, month, day, hour, minute, second, nanos,
    ))
}

/// Read a length-prefixed value, reporting `truncated_message` when the declared
/// length runs past the end of the buffer.
fn read_length_prefixed<'a>(
    data: &'a [u8],
    offset: &mut usize,
    truncated_message: &str,
) -> Result<&'a [u8], TransportError> {
    let len = read_i32(data, offset)? as usize;
    if *offset + len > data.len() {
        return Err(TransportError::ProtocolError(truncated_message.to_owned()));
    }
    let value = &data[*offset..*offset + len];
    *offset += len;
    Ok(value)
}

/// Specialised length-prefixed string path that appends directly to a
/// `StringBuilder` without allocating an owned `String` per value.
fn fill_string_builder(
    data: &[u8],
    offset: &mut usize,
    name: &str,
    num_rows: usize,
) -> Result<BuiltColumn, TransportError> {
    let mut builder = StringBuilder::with_capacity(num_rows, num_rows * 16);
    for _ in 0..num_rows {
        if read_u8(data, offset)? == 0 {
            builder.append_null();
        } else {
            let bytes = read_length_prefixed(data, offset, "String data truncated")?;
            let text = std::str::from_utf8(bytes).map_err(|e| {
                TransportError::ProtocolError(format!("Invalid UTF-8 in string: {}", e))
            })?;
            builder.append_value(text);
        }
    }
    Ok((
        Field::new(name, ArrowDataType::Utf8, true),
        Arc::new(builder.finish()),
    ))
}

pub fn parse_fetch_to_record_batch(
    data: &[u8],
    columns: &[NativeColumnMeta],
) -> Result<(i64, arrow::record_batch::RecordBatch), TransportError> {
    let mut offset = 0;
    let rows_received = read_i64(data, &mut offset)?;
    if rows_received < 0 {
        return Err(TransportError::ProtocolError(format!(
            "Invalid row count from server: {}",
            rows_received
        )));
    }
    let batch = build_batch_from_wire(data, &mut offset, columns, rows_received as usize)?;
    Ok((rows_received, batch))
}

// --- Primitive readers ---

fn read_u8(data: &[u8], offset: &mut usize) -> Result<u8, TransportError> {
    if *offset >= data.len() {
        return Err(TransportError::ProtocolError("Data truncated (u8)".into()));
    }
    let v = data[*offset];
    *offset += 1;
    Ok(v)
}

fn read_i16(data: &[u8], offset: &mut usize) -> Result<i16, TransportError> {
    if *offset + 2 > data.len() {
        return Err(TransportError::ProtocolError("Data truncated (i16)".into()));
    }
    let v = i16::from_le_bytes([data[*offset], data[*offset + 1]]);
    *offset += 2;
    Ok(v)
}

fn read_i32(data: &[u8], offset: &mut usize) -> Result<i32, TransportError> {
    if *offset + 4 > data.len() {
        return Err(TransportError::ProtocolError("Data truncated (i32)".into()));
    }
    let v = i32::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset += 4;
    Ok(v)
}

fn read_i64(data: &[u8], offset: &mut usize) -> Result<i64, TransportError> {
    if *offset + 8 > data.len() {
        return Err(TransportError::ProtocolError("Data truncated (i64)".into()));
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
    Ok(v)
}

fn read_f64(data: &[u8], offset: &mut usize) -> Result<f64, TransportError> {
    if *offset + 8 > data.len() {
        return Err(TransportError::ProtocolError("Data truncated (f64)".into()));
    }
    let v = f64::from_le_bytes([
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
    Ok(v)
}

fn read_f32(data: &[u8], offset: &mut usize) -> Result<f32, TransportError> {
    if *offset + 4 > data.len() {
        return Err(TransportError::ProtocolError("Data truncated (f32)".into()));
    }
    let v = f32::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset += 4;
    Ok(v)
}

fn read_i128(data: &[u8], offset: &mut usize) -> Result<i128, TransportError> {
    if *offset + 16 > data.len() {
        return Err(TransportError::ProtocolError(
            "Data truncated (i128)".into(),
        ));
    }
    let v = i128::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
        data[*offset + 4],
        data[*offset + 5],
        data[*offset + 6],
        data[*offset + 7],
        data[*offset + 8],
        data[*offset + 9],
        data[*offset + 10],
        data[*offset + 11],
        data[*offset + 12],
        data[*offset + 13],
        data[*offset + 14],
        data[*offset + 15],
    ]);
    *offset += 16;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::super::arrow_builder::native_meta_to_data_type;
    use super::super::constants::{IS_UTF8, SMALL_RESULTSET};
    use super::*;

    #[test]
    fn parse_empty_response() {
        let resp = parse_response(&[]).unwrap();
        assert!(matches!(resp.terminal, NativeResponse::Empty));
    }

    #[test]
    fn parse_zero_result_count_response() {
        let resp = parse_response(&0i32.to_le_bytes()).unwrap();
        assert!(matches!(resp.terminal, NativeResponse::Empty));
    }

    #[test]
    fn parse_result_count_with_still_executing() {
        let mut data = Vec::new();
        data.extend_from_slice(&1i32.to_le_bytes());
        data.push(R_STILL_EXECUTING as u8);

        let resp = parse_response(&data).unwrap();
        assert!(matches!(resp.terminal, NativeResponse::StillExecuting));
    }

    #[test]
    fn parse_result_count_with_row_count_response() {
        let mut data = Vec::new();
        data.extend_from_slice(&1i32.to_le_bytes());
        data.push(R_ROW_COUNT as u8);
        data.extend_from_slice(&42i64.to_le_bytes());
        let resp = parse_response(&data).unwrap();
        match resp.terminal {
            NativeResponse::RowCount(c) => assert_eq!(c, 42),
            _ => panic!("Expected RowCount"),
        }
    }

    #[test]
    fn parse_result_count_with_exception_response() {
        let msg = "test error";
        let state = "42000";
        let mut data = Vec::new();
        data.extend_from_slice(&1i32.to_le_bytes());
        data.push(R_EXCEPTION as u8);
        data.extend_from_slice(&(msg.len() as i32).to_le_bytes());
        data.extend_from_slice(msg.as_bytes());
        data.extend_from_slice(state.as_bytes());

        let resp = parse_response(&data).unwrap();
        match resp.terminal {
            NativeResponse::Exception { message, sql_state } => {
                assert_eq!(message, "test error");
                assert_eq!(sql_state, "42000");
            }
            _ => panic!("Expected Exception"),
        }
    }

    /// Helper: write column metadata bytes for a given type.
    fn write_col_meta(buf: &mut Vec<u8>, name: &str, type_id: u32, extra: &[u8]) {
        buf.extend_from_slice(&(name.len() as i32).to_le_bytes());
        buf.extend_from_slice(name.as_bytes());
        buf.extend_from_slice(&(type_id as i32).to_le_bytes());
        buf.extend_from_slice(extra);
    }

    fn envelope(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&(parts.len() as i32).to_le_bytes());
        for part in parts {
            data.extend_from_slice(part);
        }
        data
    }

    fn warning_part(message: &str, sql_state: &str) -> Vec<u8> {
        let mut data = Vec::new();
        data.push(R_WARNING as u8);
        data.extend_from_slice(&(message.len() as i32).to_le_bytes());
        data.extend_from_slice(message.as_bytes());
        data.extend_from_slice(sql_state.as_bytes());
        data
    }

    fn row_count_part(count: i64) -> Vec<u8> {
        let mut data = Vec::new();
        data.push(R_ROW_COUNT as u8);
        data.extend_from_slice(&count.to_le_bytes());
        data
    }

    fn empty_part() -> Vec<u8> {
        vec![R_EMPTY as u8]
    }

    fn column_count_part(count: i32) -> Vec<u8> {
        let mut data = Vec::new();
        data.push(R_COLUMN_COUNT as u8);
        data.extend_from_slice(&count.to_le_bytes());
        data
    }

    fn result_set_part(body: Vec<u8>) -> Vec<u8> {
        let mut data = Vec::with_capacity(body.len() + 1);
        data.push(R_RESULT_SET as u8);
        data.extend_from_slice(&body);
        data
    }

    /// Helper: build a minimal result set payload (after the result-type byte).
    /// Contains only column metadata (0 rows) so no column data section.
    fn build_result_set_header(columns: &[(&str, u32, Vec<u8>)]) -> Vec<u8> {
        build_result_set_header_with_handle(SMALL_RESULTSET, columns)
    }

    fn build_result_set_header_with_handle(
        handle: i32,
        columns: &[(&str, u32, Vec<u8>)],
    ) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&handle.to_le_bytes());
        data.extend_from_slice(&(columns.len() as i32).to_le_bytes());
        // total_rows
        data.extend_from_slice(&0i64.to_le_bytes());
        // rows_received
        data.extend_from_slice(&0i64.to_le_bytes());
        for (name, type_id, extra) in columns {
            write_col_meta(&mut data, name, *type_id, extra);
        }
        data
    }

    fn exception_part(message: &str, sql_state: &str) -> Vec<u8> {
        let mut data = vec![R_EXCEPTION as u8];
        data.extend_from_slice(&(message.len() as i32).to_le_bytes());
        data.extend_from_slice(message.as_bytes());
        data.extend_from_slice(sql_state.as_bytes());
        data
    }

    fn handle_part(statement_handle: i32, sub_results: &[Vec<u8>]) -> Vec<u8> {
        let mut data = vec![R_HANDLE as u8];
        data.extend_from_slice(&statement_handle.to_le_bytes());
        for sub in sub_results {
            data.extend_from_slice(sub);
        }
        data
    }

    fn more_rows_part(payload: &[u8]) -> Vec<u8> {
        let mut data = vec![R_MORE_ROWS as u8];
        data.extend_from_slice(payload);
        data
    }

    /// Metadata bytes a CHAR column carries: vcFlag(1) + maxLen(4) + octetLen(4).
    fn varchar_meta_bytes(max_len: i32) -> Vec<u8> {
        let mut extra = vec![IS_VARCHAR | IS_UTF8];
        extra.extend_from_slice(&max_len.to_le_bytes());
        extra.extend_from_slice(&(max_len * 4).to_le_bytes());
        extra
    }

    fn protocol_message(error: TransportError) -> String {
        match error {
            TransportError::ProtocolError(msg) => msg,
            other => panic!("expected ProtocolError, got {other:?}"),
        }
    }

    fn parse_error(data: &[u8]) -> String {
        protocol_message(parse_response(data).unwrap_err())
    }

    fn legacy_error(data: &[u8]) -> String {
        protocol_message(parse_legacy_response(data).unwrap_err())
    }

    #[test]
    fn parse_result_count_with_empty_response() {
        let resp = parse_response(&envelope(&[empty_part()])).unwrap();
        assert!(matches!(resp.terminal, NativeResponse::Empty));
    }

    #[test]
    fn parse_warning_then_row_count_response() {
        let resp = parse_response(&envelope(&[
            warning_part("careful", "01000"),
            row_count_part(7),
        ]))
        .unwrap();

        assert_eq!(resp.warnings.len(), 1);
        assert_eq!(resp.warnings[0].message, "careful");
        assert_eq!(resp.warnings[0].sql_state, "01000");
        assert!(matches!(resp.terminal, NativeResponse::RowCount(7)));
    }

    #[test]
    fn parse_warning_then_result_set_response() {
        let resp = parse_response(&envelope(&[
            warning_part("heads up", "01000"),
            result_set_part(build_result_set_header(&[("ID", T_INTEGER, Vec::new())])),
        ]))
        .unwrap();

        assert_eq!(resp.warnings.len(), 1);
        match resp.terminal {
            NativeResponse::ResultSet { columns, .. } => {
                assert_eq!(columns.len(), 1);
                assert_eq!(columns[0].name, "ID");
            }
            _ => panic!("Expected ResultSet"),
        }
    }

    #[test]
    fn parse_column_count_then_empty_response() {
        let resp = parse_response(&envelope(&[column_count_part(3), empty_part()])).unwrap();
        assert!(matches!(resp.terminal, NativeResponse::Empty));
    }

    #[test]
    fn parse_column_meta_timestamp_reads_precision() {
        // TIMESTAMP metadata in protocol v19+: type_id(4) + precision(4)
        let mut ts_extra = Vec::new();
        ts_extra.extend_from_slice(&3i32.to_le_bytes()); // precision = 3

        let columns = vec![("created_at", T_TIMESTAMP, ts_extra)];
        let data = build_result_set_header(&columns);
        let result = parse_result_set(&data).unwrap();

        match result {
            NativeResponse::ResultSet { columns, .. } => {
                assert_eq!(columns.len(), 1);
                assert_eq!(columns[0].name, "created_at");
                assert_eq!(columns[0].type_id, T_TIMESTAMP);
            }
            _ => panic!("Expected ResultSet"),
        }
    }

    #[test]
    fn parse_column_meta_timestamp_utc_reads_precision() {
        let mut ts_extra = Vec::new();
        ts_extra.extend_from_slice(&6i32.to_le_bytes()); // precision = 6

        let columns = vec![("ts_utc", T_TIMESTAMP_UTC, ts_extra)];
        let data = build_result_set_header(&columns);
        let result = parse_result_set(&data).unwrap();

        match result {
            NativeResponse::ResultSet { columns, .. } => {
                assert_eq!(columns.len(), 1);
                assert_eq!(columns[0].name, "ts_utc");
                assert_eq!(columns[0].type_id, T_TIMESTAMP_UTC);
            }
            _ => panic!("Expected ResultSet"),
        }
    }

    #[test]
    fn parse_column_meta_interval_year_reads_char_and_precision() {
        // INTERVAL YEAR TO MONTH: vcFlag(1) + maxLen(4) + octetLen(4) + intervalPrecision(4)
        let mut extra = Vec::new();
        extra.push(0u8); // vcFlag
        extra.extend_from_slice(&13i32.to_le_bytes()); // maxLen
        extra.extend_from_slice(&13i32.to_le_bytes()); // octetLen
        extra.extend_from_slice(&2i32.to_le_bytes()); // intervalPrecision

        let columns = vec![("interval_col", T_INTERVAL_YEAR, extra)];
        let data = build_result_set_header(&columns);
        let result = parse_result_set(&data).unwrap();

        match result {
            NativeResponse::ResultSet { columns, .. } => {
                assert_eq!(columns.len(), 1);
                assert_eq!(columns[0].name, "interval_col");
                assert_eq!(columns[0].type_id, T_INTERVAL_YEAR);
            }
            _ => panic!("Expected ResultSet"),
        }
    }

    #[test]
    fn parse_column_meta_interval_day_reads_char_and_two_precisions() {
        // INTERVAL DAY TO SECOND: vcFlag(1) + maxLen(4) + octetLen(4)
        //   + intervalPrecision(4) + intervalFraction(4)
        let mut extra = Vec::new();
        extra.push(0u8); // vcFlag
        extra.extend_from_slice(&29i32.to_le_bytes()); // maxLen
        extra.extend_from_slice(&29i32.to_le_bytes()); // octetLen
        extra.extend_from_slice(&2i32.to_le_bytes()); // intervalPrecision
        extra.extend_from_slice(&3i32.to_le_bytes()); // intervalFraction

        let columns = vec![("interval_day_col", T_INTERVAL_DAY, extra)];
        let data = build_result_set_header(&columns);
        let result = parse_result_set(&data).unwrap();

        match result {
            NativeResponse::ResultSet { columns, .. } => {
                assert_eq!(columns.len(), 1);
                assert_eq!(columns[0].name, "interval_day_col");
                assert_eq!(columns[0].type_id, T_INTERVAL_DAY);
            }
            _ => panic!("Expected ResultSet"),
        }
    }

    #[test]
    fn parse_column_meta_geometry_reads_char_metadata() {
        // GEOMETRY: vcFlag(1) + maxLen(4) + octetLen(4)
        let mut extra = Vec::new();
        extra.push(0u8); // vcFlag (no varchar, no UTF-8)
        extra.extend_from_slice(&100i32.to_le_bytes()); // maxLen
        extra.extend_from_slice(&100i32.to_le_bytes()); // octetLen

        let columns = vec![("geom_col", T_GEOMETRY, extra)];
        let data = build_result_set_header(&columns);
        let result = parse_result_set(&data).unwrap();

        match result {
            NativeResponse::ResultSet { columns, .. } => {
                assert_eq!(columns.len(), 1);
                assert_eq!(columns[0].name, "geom_col");
                assert_eq!(columns[0].type_id, T_GEOMETRY);
            }
            _ => panic!("Expected ResultSet"),
        }
    }

    #[test]
    fn varchar_flag_bit_discriminates_varchar_from_char() {
        // Literal vcFlag bytes, not the constants the parser reads them with:
        // building the input from IS_VARCHAR would pass for any mask value.
        let reported_type = |vc_flag: u8| {
            let mut extra = vec![vc_flag];
            extra.extend_from_slice(&50i32.to_le_bytes()); // maxLen
            extra.extend_from_slice(&200i32.to_le_bytes()); // octetLen
            let mut data = Vec::new();
            write_col_meta(&mut data, "NAME", T_CHAR, &extra);
            let mut offset = 0;
            native_meta_to_data_type(&parse_column_meta(&data, &mut offset).unwrap())
        };

        let varchar = reported_type(0x11);
        assert_eq!(varchar.type_name, "VARCHAR");
        assert_eq!(varchar.size, Some(50));

        let char_utf8 = reported_type(0x10);
        assert_eq!(char_utf8.type_name, "CHAR");
        assert_eq!(char_utf8.size, Some(50));

        assert_ne!(reported_type(0x00).type_name, "VARCHAR");
    }

    #[test]
    fn parse_multi_column_benchmark_table_metadata() {
        // Simulate the benchmark table:
        // id BIGINT, name VARCHAR(100), email VARCHAR(200), age INTEGER,
        // salary DECIMAL(12,2), created_at TIMESTAMP, is_active BOOLEAN,
        // description VARCHAR(1000)

        let mut columns = Vec::new();

        // BIGINT -> DECIMAL(18,0) on the wire
        let mut decimal_extra = Vec::new();
        decimal_extra.extend_from_slice(&18i32.to_le_bytes()); // precision
        decimal_extra.extend_from_slice(&0i32.to_le_bytes()); // scale
        columns.push(("ID", T_DECIMAL, decimal_extra));

        // VARCHAR(100)
        let mut vc_extra = Vec::new();
        vc_extra.push(IS_VARCHAR | IS_UTF8); // vcFlag
        vc_extra.extend_from_slice(&100i32.to_le_bytes()); // maxLen
        vc_extra.extend_from_slice(&400i32.to_le_bytes()); // octetLen (UTF-8 = 4x)
        columns.push(("NAME", T_CHAR, vc_extra));

        // VARCHAR(200)
        let mut vc_extra2 = Vec::new();
        vc_extra2.push(IS_VARCHAR | IS_UTF8);
        vc_extra2.extend_from_slice(&200i32.to_le_bytes());
        vc_extra2.extend_from_slice(&800i32.to_le_bytes());
        columns.push(("EMAIL", T_CHAR, vc_extra2));

        // INTEGER -> DECIMAL(18,0) on the wire
        let mut int_extra = Vec::new();
        int_extra.extend_from_slice(&18i32.to_le_bytes());
        int_extra.extend_from_slice(&0i32.to_le_bytes());
        columns.push(("AGE", T_DECIMAL, int_extra));

        // DECIMAL(12,2)
        let mut dec_extra = Vec::new();
        dec_extra.extend_from_slice(&12i32.to_le_bytes());
        dec_extra.extend_from_slice(&2i32.to_le_bytes());
        columns.push(("SALARY", T_DECIMAL, dec_extra));

        // TIMESTAMP with precision field (protocol v19+)
        let mut ts_extra = Vec::new();
        ts_extra.extend_from_slice(&3i32.to_le_bytes()); // precision
        columns.push(("CREATED_AT", T_TIMESTAMP, ts_extra));

        // BOOLEAN (no extra metadata)
        columns.push(("IS_ACTIVE", T_BOOLEAN, Vec::new()));

        // VARCHAR(1000)
        let mut vc_extra3 = Vec::new();
        vc_extra3.push(IS_VARCHAR | IS_UTF8);
        vc_extra3.extend_from_slice(&1000i32.to_le_bytes());
        vc_extra3.extend_from_slice(&4000i32.to_le_bytes());
        columns.push(("DESCRIPTION", T_CHAR, vc_extra3));

        let data = build_result_set_header(&columns);
        let result = parse_result_set(&data).unwrap();

        match result {
            NativeResponse::ResultSet {
                columns: cols,
                total_rows,
                rows_received,
                ..
            } => {
                assert_eq!(cols.len(), 8);
                assert_eq!(cols[0].name, "ID");
                assert_eq!(cols[0].type_id, T_DECIMAL);
                assert_eq!(cols[1].name, "NAME");
                assert_eq!(cols[1].type_id, T_CHAR);
                assert!(cols[1].is_varchar);
                assert_eq!(cols[2].name, "EMAIL");
                assert_eq!(cols[3].name, "AGE");
                assert_eq!(cols[4].name, "SALARY");
                assert_eq!(cols[4].precision, Some(12));
                assert_eq!(cols[4].scale, Some(2));
                assert_eq!(cols[5].name, "CREATED_AT");
                assert_eq!(cols[5].type_id, T_TIMESTAMP);
                assert_eq!(cols[6].name, "IS_ACTIVE");
                assert_eq!(cols[6].type_id, T_BOOLEAN);
                assert_eq!(cols[7].name, "DESCRIPTION");
                assert_eq!(cols[7].type_id, T_CHAR);
                assert!(cols[7].is_varchar);
                assert_eq!(total_rows, 0);
                assert_eq!(rows_received, 0);
            }
            _ => panic!("Expected ResultSet"),
        }
    }

    fn meta(
        name: &str,
        type_id: u32,
        precision: Option<i32>,
        scale: Option<i32>,
    ) -> NativeColumnMeta {
        NativeColumnMeta {
            name: name.to_string(),
            type_id,
            precision,
            scale,
            is_varchar: false,
            max_len: None,
        }
    }

    #[test]
    fn single_pass_date_column_decodes_to_date32() {
        use arrow::array::{Array, Date32Array};
        let columns = vec![meta("d", T_DATE, None, None)];
        let mut data = Vec::new();
        // Row 0: 2024-01-02 → packed i32
        let packed = (2024i32 << 16) | (1 << 8) | 2;
        data.push(1u8);
        data.extend_from_slice(&packed.to_le_bytes());
        // Row 1: null
        data.push(0u8);

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 2).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 1);

        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap();
        assert!(arr.is_null(1));
        let expected = crate::types::conversion::ymd_to_days(2024, 1, 2);
        assert_eq!(arr.value(0), expected);
        assert_eq!(offset, data.len());
    }

    #[test]
    fn single_pass_timestamp_column_decodes_to_microseconds() {
        use arrow::array::{Array, TimestampMicrosecondArray};

        let columns = vec![meta("ts", T_TIMESTAMP, None, None)];
        let mut data = Vec::new();
        // Row 0: 1970-01-01 00:00:00.123
        data.push(1u8);
        data.extend_from_slice(&1970i16.to_le_bytes());
        data.push(1u8);
        data.push(1u8);
        data.push(0u8);
        data.push(0u8);
        data.push(0u8);
        data.extend_from_slice(&123_000_000i32.to_le_bytes()); // 123 ms → 123_000 micros
                                                               // Row 1: null
        data.push(0u8);

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 2).unwrap();
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap();
        assert_eq!(arr.value(0), 123_000);
        assert!(arr.is_null(1));
    }

    #[test]
    fn single_pass_timestamp_utc_has_utc_timezone() {
        use arrow::array::Array;
        use arrow::datatypes::{DataType, TimeUnit};

        let columns = vec![meta("ts", T_TIMESTAMP_UTC, None, None)];
        let mut data = Vec::new();
        data.push(1u8);
        data.extend_from_slice(&1970i16.to_le_bytes());
        data.push(1u8);
        data.push(1u8);
        data.push(0u8);
        data.push(0u8);
        data.push(0u8);
        data.extend_from_slice(&0i32.to_le_bytes());

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 1).unwrap();
        let arr = batch.column(0);
        assert_eq!(
            arr.data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
    }

    #[test]
    fn single_pass_string_column_decodes_utf8_without_owning_string() {
        use arrow::array::{Array, StringArray};

        let columns = vec![meta("s", T_CHAR, None, None)];
        let mut data = Vec::new();
        // Row 0: "hello"
        data.push(1u8);
        data.extend_from_slice(&5i32.to_le_bytes());
        data.extend_from_slice(b"hello");
        // Row 1: null
        data.push(0u8);

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 2).unwrap();
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(arr.value(0), "hello");
        assert!(arr.is_null(1));
    }

    #[test]
    fn single_pass_all_primitive_types_offsets_align() {
        // DOUBLE + BOOLEAN + INTEGER with 2 rows each, mixed nulls.
        let columns = vec![
            meta("d", T_DOUBLE, None, None),
            meta("b", T_BOOLEAN, None, None),
            meta("i", T_INTEGER, None, None),
        ];
        let mut data = Vec::new();
        // DOUBLE column
        data.push(1u8);
        data.extend_from_slice(&1.5f64.to_le_bytes());
        data.push(0u8); // null

        // BOOLEAN column
        data.push(1u8);
        data.push(1u8); // true
        data.push(0u8); // null

        // INTEGER column
        data.push(1u8);
        data.extend_from_slice(&42i64.to_le_bytes());
        data.push(0u8); // null

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 2).unwrap();
        assert_eq!(offset, data.len(), "all bytes should be consumed");
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);

        use arrow::array::{Array, BooleanArray, Float64Array, Int64Array};
        let f = batch
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let b = batch
            .column(1)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        let i = batch
            .column(2)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();

        assert_eq!(f.value(0), 1.5);
        assert!(f.is_null(1));
        assert!(b.value(0));
        assert!(b.is_null(1));
        assert_eq!(i.value(0), 42);
        assert!(i.is_null(1));
    }

    #[test]
    fn single_pass_empty_rows_produces_typed_empty_batch() {
        let columns = vec![
            meta("i", T_INTEGER, None, None),
            meta("s", T_CHAR, None, None),
        ];
        let data = Vec::new();
        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 0).unwrap();
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.num_columns(), 2);
        use arrow::datatypes::DataType;
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Int64);
        assert_eq!(batch.schema().field(1).data_type(), &DataType::Utf8);
    }

    #[test]
    fn single_pass_binary_column_copies_into_builder() {
        use arrow::array::{Array, BinaryArray};
        let columns = vec![meta("bin", T_BINARY, None, None)];
        let mut data = Vec::new();
        data.push(1u8);
        data.extend_from_slice(&3i32.to_le_bytes());
        data.extend_from_slice(&[0xDE, 0xAD, 0xBE]);
        data.push(0u8);

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 2).unwrap();
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(arr.value(0), &[0xDE, 0xAD, 0xBE][..]);
        assert!(arr.is_null(1));
    }

    #[test]
    fn parse_fetch_to_record_batch_reads_row_count_prefix() {
        use arrow::array::{Array, Int64Array};
        let columns = vec![meta("i", T_INTEGER, None, None)];
        let mut data = Vec::new();
        // rows_received prefix
        data.extend_from_slice(&2i64.to_le_bytes());
        // Row 0: 100
        data.push(1u8);
        data.extend_from_slice(&100i64.to_le_bytes());
        // Row 1: null
        data.push(0u8);

        let (rows, batch) = parse_fetch_to_record_batch(&data, &columns).unwrap();
        assert_eq!(rows, 2);
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(arr.value(0), 100);
        assert!(arr.is_null(1));
    }

    #[test]
    fn single_pass_decimal_scale_zero_maps_to_decimal128() {
        use arrow::array::Decimal128Array;
        use arrow::datatypes::DataType;

        let columns = vec![meta("big", T_DECIMAL, Some(18), Some(0))];
        let mut data = Vec::new();
        data.push(1u8);
        data.extend_from_slice(&12345i64.to_le_bytes());

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 1).unwrap();
        assert_eq!(
            batch.schema().field(0).data_type(),
            &DataType::Decimal128(18, 0)
        );
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        assert_eq!(arr.value(0), 12345i128);
    }

    #[test]
    fn single_pass_decimal_with_scale_maps_to_decimal128_preserving_precision() {
        use arrow::array::Decimal128Array;
        use arrow::datatypes::DataType;

        let columns = vec![meta("s", T_DECIMAL, Some(12), Some(2))];
        let mut data = Vec::new();
        // precision <= 9 would be i32; precision 12 uses i64
        data.push(1u8);
        data.extend_from_slice(&12345i64.to_le_bytes());

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 1).unwrap();
        assert_eq!(
            batch.schema().field(0).data_type(),
            &DataType::Decimal128(12, 2)
        );
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        assert_eq!(arr.value(0), 12345i128);
        assert_eq!(arr.value_as_string(0), "123.45");
    }

    #[test]
    fn single_pass_smalldecimal_uses_i32_wire_and_decimal128() {
        use arrow::array::{Array, Decimal128Array};
        use arrow::datatypes::DataType;

        let columns = vec![meta("s", T_SMALLDECIMAL, Some(5), Some(2))];
        let mut data = Vec::new();
        data.push(1u8);
        data.extend_from_slice(&12345i32.to_le_bytes());
        data.push(0u8); // null

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 2).unwrap();
        assert_eq!(
            batch.schema().field(0).data_type(),
            &DataType::Decimal128(5, 2)
        );
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        assert_eq!(arr.value(0), 12345i128);
        assert!(arr.is_null(1));
        assert_eq!(offset, data.len());
    }

    #[test]
    fn single_pass_decimal_precision_le_9_uses_i32_wire() {
        use arrow::array::Decimal128Array;
        use arrow::datatypes::DataType;

        let columns = vec![meta("d", T_DECIMAL, Some(9), Some(3))];
        let mut data = Vec::new();
        data.push(1u8);
        data.extend_from_slice(&1_234_567i32.to_le_bytes());

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 1).unwrap();
        assert_eq!(
            batch.schema().field(0).data_type(),
            &DataType::Decimal128(9, 3)
        );
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        assert_eq!(arr.value(0), 1_234_567i128);
        assert_eq!(offset, data.len());
    }

    #[test]
    fn single_pass_real_column_widens_f32_to_f64() {
        use arrow::array::{Array, Float64Array};
        use arrow::datatypes::DataType;

        let columns = vec![meta("r", T_REAL, None, None)];
        let mut data = Vec::new();
        data.push(1u8);
        data.extend_from_slice(&1.5f32.to_le_bytes());
        data.push(0u8);

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 2).unwrap();
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Float64);
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert_eq!(arr.value(0), 1.5);
        assert!(arr.is_null(1));
        assert_eq!(offset, data.len());
    }

    #[test]
    fn single_pass_smallint_column_decodes_to_int32() {
        use arrow::array::{Array, Int32Array};
        use arrow::datatypes::DataType;

        let columns = vec![meta("si", T_SMALLINT, None, None)];
        let mut data = Vec::new();
        data.push(1u8);
        data.extend_from_slice(&(-7i32).to_le_bytes());
        data.push(0u8);

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 2).unwrap();
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Int32);
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(arr.value(0), -7);
        assert!(arr.is_null(1));
        assert_eq!(offset, data.len());
    }

    #[test]
    fn single_pass_timestamp_with_local_time_zone_has_no_arrow_timezone() {
        use arrow::datatypes::{DataType, TimeUnit};

        let columns = vec![meta("ts", T_TIMESTAMP_LOCAL_TZ, None, None)];
        let mut data = Vec::new();
        data.push(1u8);
        data.extend_from_slice(&1970i16.to_le_bytes());
        data.push(1u8);
        data.push(1u8);
        data.push(0u8);
        data.push(0u8);
        data.push(0u8);
        data.extend_from_slice(&0i32.to_le_bytes());

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 1).unwrap();
        assert_eq!(
            batch.schema().field(0).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, None)
        );
    }

    #[test]
    fn single_pass_string_like_types_all_decode_as_utf8() {
        use arrow::array::StringArray;
        use arrow::datatypes::DataType;

        const UNKNOWN_TYPE_ID: u32 = 9_999;
        let string_like = [
            T_CHAR,
            T_GEOMETRY,
            T_HASHTYPE,
            T_INTERVAL_YEAR,
            T_INTERVAL_DAY,
            UNKNOWN_TYPE_ID,
        ];

        for type_id in string_like {
            let columns = vec![meta("s", type_id, None, None)];
            let mut data = Vec::new();
            data.push(1u8);
            data.extend_from_slice(&2i32.to_le_bytes());
            data.extend_from_slice(b"ok");

            let mut offset = 0;
            let batch = build_batch_from_wire(&data, &mut offset, &columns, 1).unwrap();
            assert_eq!(
                batch.schema().field(0).data_type(),
                &DataType::Utf8,
                "type_id {type_id}"
            );
            let arr = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            assert_eq!(arr.value(0), "ok", "type_id {type_id}");
            assert_eq!(offset, data.len(), "type_id {type_id}");
        }
    }

    #[test]
    fn single_pass_bigdecimal_preserves_i128() {
        use arrow::array::Decimal128Array;
        use arrow::datatypes::DataType;

        // i128 value larger than i64::MAX would overflow under the old path.
        let big: i128 = (i64::MAX as i128) + 1;
        let columns = vec![meta("bd", T_BIGDECIMAL, Some(36), Some(0))];
        let mut data = Vec::new();
        data.push(1u8);
        data.extend_from_slice(&big.to_le_bytes());

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 1).unwrap();
        assert_eq!(
            batch.schema().field(0).data_type(),
            &DataType::Decimal128(36, 0)
        );
        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        assert_eq!(arr.value(0), big);
        assert_eq!(offset, data.len());
    }

    // --- Legacy (un-counted) framing ---

    #[test]
    fn legacy_result_set_carries_its_column_metadata() {
        let data = result_set_part(build_result_set_header(&[("ID", T_INTEGER, Vec::new())]));

        let resp = parse_legacy_response(&data).unwrap();

        assert!(resp.warnings.is_empty());
        match resp.terminal {
            NativeResponse::ResultSet {
                handle, columns, ..
            } => {
                assert_eq!(handle, SMALL_RESULTSET);
                assert_eq!(columns.len(), 1);
                assert_eq!(columns[0].name, "ID");
            }
            other => panic!("Expected ResultSet, got {other:?}"),
        }
    }

    #[test]
    fn legacy_handle_without_sub_result_reports_no_columns() {
        let resp = parse_legacy_response(&handle_part(77, &[])).unwrap();

        match resp.terminal {
            NativeResponse::PreparedStatement {
                handle,
                parameters,
                result_columns,
            } => {
                assert_eq!(handle, 77);
                assert!(parameters.is_empty());
                assert!(result_columns.is_empty());
            }
            other => panic!("Expected PreparedStatement, got {other:?}"),
        }
    }

    #[test]
    fn legacy_row_count_returns_the_affected_row_count() {
        let resp = parse_legacy_response(&row_count_part(9)).unwrap();
        assert!(matches!(resp.terminal, NativeResponse::RowCount(9)));
    }

    #[test]
    fn legacy_exception_returns_message_and_sql_state() {
        let resp = parse_legacy_response(&exception_part("boom", "42000")).unwrap();

        match resp.terminal {
            NativeResponse::Exception { message, sql_state } => {
                assert_eq!(message, "boom");
                assert_eq!(sql_state, "42000");
            }
            other => panic!("Expected Exception, got {other:?}"),
        }
    }

    #[test]
    fn legacy_warning_is_collected_and_leaves_an_empty_terminal() {
        let resp = parse_legacy_response(&warning_part("careful", "01000")).unwrap();

        assert_eq!(
            resp.warnings,
            vec![NativeWarning {
                message: "careful".to_owned(),
                sql_state: "01000".to_owned(),
            }]
        );
        assert!(matches!(resp.terminal, NativeResponse::Empty));
    }

    #[test]
    fn legacy_column_count_is_consumed_and_yields_an_empty_terminal() {
        let resp = parse_legacy_response(&column_count_part(4)).unwrap();

        assert!(resp.warnings.is_empty());
        assert!(matches!(resp.terminal, NativeResponse::Empty));
    }

    #[test]
    fn legacy_empty_response_yields_an_empty_terminal() {
        let resp = parse_legacy_response(&empty_part()).unwrap();
        assert!(matches!(resp.terminal, NativeResponse::Empty));
    }

    #[test]
    fn legacy_still_executing_without_status_message_is_accepted() {
        let resp = parse_legacy_response(&[R_STILL_EXECUTING as u8]).unwrap();
        assert!(matches!(resp.terminal, NativeResponse::StillExecuting));
    }

    #[test]
    fn legacy_still_executing_consumes_its_status_message() {
        let mut data = vec![R_STILL_EXECUTING as u8];
        data.extend_from_slice(&7i32.to_le_bytes());
        data.extend_from_slice(b"running");

        let resp = parse_legacy_response(&data).unwrap();
        assert!(matches!(resp.terminal, NativeResponse::StillExecuting));
    }

    #[test]
    fn legacy_still_executing_with_a_short_status_message_is_rejected() {
        let mut data = vec![R_STILL_EXECUTING as u8];
        data.extend_from_slice(&10i32.to_le_bytes());
        data.extend_from_slice(b"ab");

        assert_eq!(
            legacy_error(&data),
            "StillExecuting status message truncated"
        );
    }

    #[test]
    fn legacy_more_rows_takes_the_rest_of_the_buffer() {
        let resp = parse_legacy_response(&more_rows_part(&[0xAA, 0xBB, 0xCC])).unwrap();

        match resp.terminal {
            NativeResponse::MoreRows(payload) => assert_eq!(payload, vec![0xAA, 0xBB, 0xCC]),
            other => panic!("Expected MoreRows, got {other:?}"),
        }
    }

    #[test]
    fn legacy_unknown_response_type_is_rejected() {
        assert_eq!(legacy_error(&[0x7F]), "Unknown response type: 127");
    }

    #[test]
    fn legacy_response_with_unconsumed_bytes_is_rejected() {
        let mut data = empty_part();
        data.push(0xAA);

        assert!(
            legacy_error(&data).starts_with("Trailing bytes after legacy response:"),
            "unexpected message"
        );
    }

    // --- Handle-only responses and their sub-results ---

    #[test]
    fn handle_sub_result_with_small_resultset_handle_describes_result_columns() {
        let sub = result_set_part(build_result_set_header_with_handle(
            SMALL_RESULTSET,
            &[("C1", T_INTEGER, Vec::new())],
        ));

        let resp = parse_legacy_response(&handle_part(5, &[sub])).unwrap();

        match resp.terminal {
            NativeResponse::PreparedStatement {
                handle,
                parameters,
                result_columns,
            } => {
                assert_eq!(handle, 5);
                assert!(parameters.is_empty());
                assert_eq!(result_columns.len(), 1);
                assert_eq!(result_columns[0].name, "C1");
            }
            other => panic!("Expected PreparedStatement, got {other:?}"),
        }
    }

    #[test]
    fn handle_sub_result_with_parameter_description_handle_describes_parameters() {
        let sub = result_set_part(build_result_set_header_with_handle(
            PARAMETER_DESCRIPTION,
            &[("P1", T_CHAR, varchar_meta_bytes(100))],
        ));

        let resp = parse_legacy_response(&handle_part(5, &[sub])).unwrap();

        match resp.terminal {
            NativeResponse::PreparedStatement {
                handle,
                parameters,
                result_columns,
            } => {
                assert_eq!(handle, 5);
                assert_eq!(parameters.len(), 1);
                assert_eq!(parameters[0].name, "P1");
                assert!(parameters[0].is_varchar);
                assert!(result_columns.is_empty());
            }
            other => panic!("Expected PreparedStatement, got {other:?}"),
        }
    }

    #[test]
    fn handle_with_both_sub_results_keeps_parameters_and_result_columns() {
        let first_result_set = result_set_part(build_result_set_header_with_handle(
            SMALL_RESULTSET,
            &[("C1", T_INTEGER, Vec::new())],
        ));
        let parameter_description = result_set_part(build_result_set_header_with_handle(
            PARAMETER_DESCRIPTION,
            &[("P1", T_INTEGER, Vec::new())],
        ));
        let last_result_set = result_set_part(build_result_set_header_with_handle(
            7,
            &[("C2", T_INTEGER, Vec::new())],
        ));

        let resp = parse_legacy_response(&handle_part(
            5,
            &[first_result_set, parameter_description, last_result_set],
        ))
        .unwrap();

        match resp.terminal {
            NativeResponse::PreparedStatement {
                handle,
                parameters,
                result_columns,
            } => {
                assert_eq!(handle, 5);
                assert_eq!(parameters.len(), 1);
                assert_eq!(parameters[0].name, "P1");
                assert_eq!(result_columns.len(), 1);
                assert_eq!(result_columns[0].name, "C2");
            }
            other => panic!("Expected PreparedStatement, got {other:?}"),
        }
    }

    #[test]
    fn handle_sub_result_that_is_not_a_result_set_leaves_no_columns() {
        let resp = parse_legacy_response(&handle_part(5, &[empty_part()])).unwrap();

        match resp.terminal {
            NativeResponse::PreparedStatement {
                handle,
                parameters,
                result_columns,
            } => {
                assert_eq!(handle, 5);
                assert!(parameters.is_empty());
                assert!(result_columns.is_empty());
            }
            other => panic!("Expected PreparedStatement, got {other:?}"),
        }
    }

    #[test]
    fn handle_sub_result_carrying_an_exception_is_rejected() {
        let data = handle_part(5, &[exception_part("prepare refused", "42000")]);

        let msg = legacy_error(&data);

        assert!(msg.contains("prepare refused"), "{msg}");
        assert!(msg.contains("42000"), "{msg}");
    }

    #[test]
    fn handle_sub_result_warning_is_propagated() {
        let data = handle_part(5, &[warning_part("truncated", "01004")]);

        let resp = parse_legacy_response(&data).unwrap();

        assert_eq!(resp.warnings.len(), 1);
        assert_eq!(resp.warnings[0].message, "truncated");
        assert_eq!(resp.warnings[0].sql_state, "01004");
        match resp.terminal {
            NativeResponse::PreparedStatement {
                handle,
                parameters,
                result_columns,
            } => {
                assert_eq!(handle, 5);
                assert!(parameters.is_empty());
                assert!(result_columns.is_empty());
            }
            other => panic!("Expected PreparedStatement, got {other:?}"),
        }
    }

    #[test]
    fn counted_envelope_handle_part_propagates_a_sub_result_warning() {
        let data = envelope(&[handle_part(31, &[warning_part("careful", "01000")])]);

        let resp = parse_response(&data).unwrap();

        assert_eq!(resp.warnings.len(), 1);
        assert_eq!(resp.warnings[0].message, "careful");
        assert!(matches!(
            resp.terminal,
            NativeResponse::PreparedStatement { handle: 31, .. }
        ));
    }

    // --- Counted envelope framing ---

    #[test]
    fn more_rows_as_the_final_envelope_part_is_accepted() {
        let data = envelope(&[column_count_part(2), more_rows_part(&[1, 2, 3])]);

        let resp = parse_response(&data).unwrap();

        match resp.terminal {
            NativeResponse::MoreRows(payload) => assert_eq!(payload, vec![1, 2, 3]),
            other => panic!("Expected MoreRows, got {other:?}"),
        }
    }

    #[test]
    fn more_rows_before_the_final_envelope_part_is_rejected() {
        let data = envelope(&[more_rows_part(&[1, 2, 3]), empty_part()]);

        assert_eq!(
            parse_error(&data),
            "MoreRows must be the final native response part"
        );
    }

    #[test]
    fn two_terminal_results_in_one_envelope_are_rejected() {
        let data = envelope(&[row_count_part(1), row_count_part(2)]);

        assert_eq!(
            parse_error(&data),
            "Multiple terminal results in native response envelope"
        );
    }

    #[test]
    fn unknown_response_type_inside_an_envelope_names_its_offset() {
        let data = envelope(&[column_count_part(1), vec![0x7F]]);

        let msg = parse_error(&data);
        assert!(
            msg.starts_with("Unknown response type: 127 at offset 9"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn envelope_with_an_exception_part_returns_the_exception() {
        let data = envelope(&[
            warning_part("careful", "01000"),
            exception_part("bad", "42000"),
        ]);

        let resp = parse_response(&data).unwrap();

        assert_eq!(resp.warnings.len(), 1);
        match resp.terminal {
            NativeResponse::Exception { message, sql_state } => {
                assert_eq!(message, "bad");
                assert_eq!(sql_state, "42000");
            }
            other => panic!("Expected Exception, got {other:?}"),
        }
    }

    #[test]
    fn envelope_with_a_handle_part_returns_the_statement_handle() {
        let data = envelope(&[handle_part(31, &[])]);

        let resp = parse_response(&data).unwrap();

        match resp.terminal {
            NativeResponse::PreparedStatement {
                handle,
                parameters,
                result_columns,
            } => {
                assert_eq!(handle, 31);
                assert!(parameters.is_empty());
                assert!(result_columns.is_empty());
            }
            other => panic!("Expected PreparedStatement, got {other:?}"),
        }
    }

    // --- Falling back from the counted envelope to legacy framing ---

    /// A payload under five bytes cannot hold a result count, so it is read as legacy.
    #[test]
    fn payload_too_short_for_a_result_count_falls_back_to_legacy() {
        let resp = parse_response(&[R_STILL_EXECUTING as u8]).unwrap();
        assert!(matches!(resp.terminal, NativeResponse::StillExecuting));
    }

    /// A legacy row count whose first four bytes read as an out-of-range result
    /// count (10752) is not mistaken for a counted envelope.
    #[test]
    fn out_of_range_result_count_falls_back_to_legacy() {
        let data = row_count_part(42);
        assert_eq!(
            i32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            10_752
        );

        let resp = parse_response(&data).unwrap();
        assert!(matches!(resp.terminal, NativeResponse::RowCount(42)));
    }

    /// A legacy result set whose fifth byte is not a known part type is not
    /// mistaken for a counted envelope holding one part.
    #[test]
    fn unknown_first_part_type_falls_back_to_legacy() {
        let handle = i32::from_le_bytes([0x00, 0x00, 0x00, 0x63]);
        let data = result_set_part(build_result_set_header_with_handle(handle, &[]));
        assert_eq!(i32::from_le_bytes([data[0], data[1], data[2], data[3]]), 1);
        assert!(!is_known_response_type(data[4] as i8));

        let resp = parse_response(&data).unwrap();

        match resp.terminal {
            NativeResponse::ResultSet {
                handle: parsed,
                batch,
                ..
            } => {
                assert_eq!(parsed, handle);
                assert!(batch.is_none());
            }
            other => panic!("Expected ResultSet, got {other:?}"),
        }
    }

    /// A counted parse that leaves bytes unconsumed is discarded, and the same
    /// payload is re-read as a legacy MoreRows response.
    #[test]
    fn counted_envelope_leaving_trailing_bytes_falls_back_to_legacy() {
        let mut data = 6i32.to_le_bytes().to_vec();
        for _ in 0..6 {
            data.extend_from_slice(&column_count_part(0));
        }
        data.push(0xAA);

        let resp = parse_response(&data).unwrap();

        match resp.terminal {
            NativeResponse::MoreRows(payload) => assert_eq!(payload, data[1..].to_vec()),
            other => panic!("Expected MoreRows, got {other:?}"),
        }
    }

    #[test]
    fn known_response_types_are_exactly_the_nine_protocol_parts() {
        for result_type in [
            R_ROW_COUNT,
            R_RESULT_SET,
            R_HANDLE,
            R_COLUMN_COUNT,
            R_WARNING,
            R_STILL_EXECUTING,
            R_MORE_ROWS,
            R_EXCEPTION,
            R_EMPTY,
        ] {
            assert!(
                is_known_response_type(result_type),
                "{result_type} should be known"
            );
        }
        for result_type in [7i8, 99, -3, -128] {
            assert!(
                !is_known_response_type(result_type),
                "{result_type} should be unknown"
            );
        }
    }

    // --- Message parts ---

    #[test]
    fn message_part_shorter_than_its_length_prefix_is_rejected() {
        let mut data = vec![R_EXCEPTION as u8];
        data.extend_from_slice(&10i32.to_le_bytes());
        data.extend_from_slice(b"ab");

        assert_eq!(legacy_error(&data), "Exception message truncated");
    }

    #[test]
    fn non_utf8_message_part_is_rejected() {
        let mut data = vec![R_EXCEPTION as u8];
        data.extend_from_slice(&2i32.to_le_bytes());
        data.extend_from_slice(&[0xFF, 0xFE]);
        data.extend_from_slice(b"42000");

        assert!(
            legacy_error(&data).starts_with("Invalid exception UTF-8: "),
            "unexpected message"
        );
    }

    #[test]
    fn message_part_without_room_for_a_sql_state_uses_the_unknown_state() {
        let mut data = 2i32.to_le_bytes().to_vec();
        data.extend_from_slice(b"ab");
        data.extend_from_slice(b"123");

        let mut offset = 0;
        let warning = parse_message_part(&data, &mut offset).unwrap();

        assert_eq!(warning.message, "ab");
        assert_eq!(warning.sql_state, "?????");
        assert_eq!(offset, 6);
    }

    #[test]
    fn message_part_with_a_non_utf8_sql_state_uses_the_unknown_state() {
        let mut data = 2i32.to_le_bytes().to_vec();
        data.extend_from_slice(b"ab");
        data.extend_from_slice(&[0xFFu8; 5]);

        let mut offset = 0;
        let warning = parse_message_part(&data, &mut offset).unwrap();

        assert_eq!(warning.sql_state, "?????");
        assert_eq!(offset, data.len());
    }

    // --- Column metadata errors ---

    #[test]
    fn column_name_longer_than_the_buffer_is_rejected() {
        let mut data = SMALL_RESULTSET.to_le_bytes().to_vec();
        data.extend_from_slice(&1i32.to_le_bytes());
        data.extend_from_slice(&0i64.to_le_bytes());
        data.extend_from_slice(&0i64.to_le_bytes());
        data.extend_from_slice(&50i32.to_le_bytes());
        data.extend_from_slice(b"ab");

        assert_eq!(
            protocol_message(parse_result_set(&data).unwrap_err()),
            "Column name truncated"
        );
    }

    #[test]
    fn non_utf8_column_name_is_rejected() {
        let mut data = SMALL_RESULTSET.to_le_bytes().to_vec();
        data.extend_from_slice(&1i32.to_le_bytes());
        data.extend_from_slice(&0i64.to_le_bytes());
        data.extend_from_slice(&0i64.to_le_bytes());
        data.extend_from_slice(&2i32.to_le_bytes());
        data.extend_from_slice(&[0xFF, 0xFE]);
        data.extend_from_slice(&T_INTEGER.to_le_bytes());

        assert!(
            protocol_message(parse_result_set(&data).unwrap_err())
                .starts_with("Invalid column name UTF-8: "),
            "unexpected message"
        );
    }

    #[test]
    fn string_like_column_metadata_without_its_varchar_flag_is_rejected() {
        for (type_id, expected) in [
            (T_CHAR, "CHAR metadata truncated"),
            (T_GEOMETRY, "CHAR metadata truncated"),
            (T_HASHTYPE, "CHAR metadata truncated"),
            (T_INTERVAL_YEAR, "INTERVAL metadata truncated"),
            (T_INTERVAL_DAY, "INTERVAL metadata truncated"),
        ] {
            let data = build_result_set_header(&[("C", type_id, Vec::new())]);

            assert_eq!(
                protocol_message(parse_result_set(&data).unwrap_err()),
                expected,
                "type_id {type_id}"
            );
        }
    }

    #[test]
    fn negative_rows_received_in_a_result_set_is_rejected() {
        let mut data = SMALL_RESULTSET.to_le_bytes().to_vec();
        data.extend_from_slice(&0i32.to_le_bytes());
        data.extend_from_slice(&0i64.to_le_bytes());
        data.extend_from_slice(&(-1i64).to_le_bytes());

        assert_eq!(
            protocol_message(parse_result_set(&data).unwrap_err()),
            "Invalid row count from server: -1"
        );
    }

    #[test]
    fn negative_rows_received_in_a_fetch_is_rejected() {
        let data = (-5i64).to_le_bytes();

        assert_eq!(
            protocol_message(parse_fetch_to_record_batch(&data, &[]).unwrap_err()),
            "Invalid row count from server: -5"
        );
    }

    #[test]
    fn batch_without_columns_still_reports_its_row_count() {
        let data = Vec::new();
        let mut offset = 0;

        let batch = build_batch_from_wire(&data, &mut offset, &[], 3).unwrap();

        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 0);
        assert_eq!(offset, 0);
    }

    // --- Truncated column values ---

    #[test]
    fn truncated_column_values_name_the_missing_primitive() {
        let cases: Vec<(u32, Vec<u8>, &str)> = vec![
            (T_INTEGER, vec![], "Data truncated (u8)"),
            (T_BOOLEAN, vec![1u8], "Data truncated (u8)"),
            (T_DOUBLE, vec![1u8, 0, 0], "Data truncated (f64)"),
            (T_REAL, vec![1u8, 0], "Data truncated (f32)"),
            (T_INTEGER, vec![1u8, 0, 0, 0], "Data truncated (i64)"),
            (T_SMALLINT, vec![1u8, 0], "Data truncated (i32)"),
            (T_DATE, vec![1u8, 0], "Data truncated (i32)"),
            (T_BIGDECIMAL, vec![1u8, 0, 0], "Data truncated (i128)"),
            (T_TIMESTAMP, vec![1u8, 0], "Data truncated (i16)"),
        ];

        for (type_id, data, expected) in cases {
            let columns = vec![meta("c", type_id, None, None)];
            let mut offset = 0;

            let err = build_batch_from_wire(&data, &mut offset, &columns, 1).unwrap_err();

            assert_eq!(protocol_message(err), expected, "type_id {type_id}");
        }
    }

    #[test]
    fn string_value_longer_than_the_buffer_is_rejected() {
        let columns = vec![meta("s", T_CHAR, None, None)];
        let mut data = vec![1u8];
        data.extend_from_slice(&10i32.to_le_bytes());
        data.extend_from_slice(b"ab");

        let mut offset = 0;
        let err = build_batch_from_wire(&data, &mut offset, &columns, 1).unwrap_err();

        assert_eq!(protocol_message(err), "String data truncated");
    }

    #[test]
    fn non_utf8_string_value_is_rejected() {
        let columns = vec![meta("s", T_CHAR, None, None)];
        let mut data = vec![1u8];
        data.extend_from_slice(&2i32.to_le_bytes());
        data.extend_from_slice(&[0xFF, 0xFE]);

        let mut offset = 0;
        let err = build_batch_from_wire(&data, &mut offset, &columns, 1).unwrap_err();

        assert!(
            protocol_message(err).starts_with("Invalid UTF-8 in string: "),
            "unexpected message"
        );
    }

    #[test]
    fn binary_value_longer_than_the_buffer_is_rejected() {
        let columns = vec![meta("b", T_BINARY, None, None)];
        let mut data = vec![1u8];
        data.extend_from_slice(&10i32.to_le_bytes());
        data.extend_from_slice(b"ab");

        let mut offset = 0;
        let err = build_batch_from_wire(&data, &mut offset, &columns, 1).unwrap_err();

        assert_eq!(protocol_message(err), "Binary data truncated");
    }

    #[test]
    fn decimal_columns_accept_nulls_on_both_wire_widths() {
        use arrow::array::{Array, Decimal128Array};

        for (precision, value_bytes) in [
            (9, 1_234i32.to_le_bytes().to_vec()),
            (18, 1_234i64.to_le_bytes().to_vec()),
        ] {
            let columns = vec![meta("d", T_DECIMAL, Some(precision), Some(0))];
            let mut data = vec![1u8];
            data.extend_from_slice(&value_bytes);
            data.push(0u8);

            let mut offset = 0;
            let batch = build_batch_from_wire(&data, &mut offset, &columns, 2).unwrap();

            let arr = batch
                .column(0)
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .unwrap();
            assert_eq!(arr.value(0), 1_234i128, "precision {precision}");
            assert!(arr.is_null(1), "precision {precision}");
            assert_eq!(offset, data.len(), "precision {precision}");
        }
    }

    #[test]
    fn bigdecimal_column_accepts_nulls() {
        use arrow::array::{Array, Decimal128Array};

        let columns = vec![meta("bd", T_BIGDECIMAL, Some(36), Some(0))];
        let mut data = vec![1u8];
        data.extend_from_slice(&7i128.to_le_bytes());
        data.push(0u8);

        let mut offset = 0;
        let batch = build_batch_from_wire(&data, &mut offset, &columns, 2).unwrap();

        let arr = batch
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        assert_eq!(arr.value(0), 7i128);
        assert!(arr.is_null(1));
        assert_eq!(offset, data.len());
    }

    #[test]
    fn result_set_with_unconsumed_bytes_is_rejected() {
        let mut data = build_result_set_header(&[]);
        data.push(0xAA);

        assert!(
            protocol_message(parse_result_set(&data).unwrap_err())
                .starts_with("Trailing bytes after result-set response:"),
            "unexpected message"
        );
    }

    #[test]
    fn result_set_promising_rows_it_does_not_carry_is_rejected() {
        let mut data = SMALL_RESULTSET.to_le_bytes().to_vec();
        data.extend_from_slice(&1i32.to_le_bytes());
        data.extend_from_slice(&2i64.to_le_bytes());
        data.extend_from_slice(&2i64.to_le_bytes());
        write_col_meta(&mut data, "I", T_INTEGER, &[]);

        assert_eq!(
            protocol_message(parse_result_set(&data).unwrap_err()),
            "Data truncated (u8)"
        );
    }

    // --- Decimal precision and scale clamping ---

    #[test]
    fn decimal_precision_and_scale_are_clamped_to_the_arrow_range() {
        let cases = [
            (None, None, 18, (18u8, 0i8)),
            (Some(100), Some(2), 18, (38, 2)),
            (Some(0), Some(2), 18, (1, 2)),
            (Some(10), Some(1_000), 18, (10, 127)),
            (Some(10), Some(-1_000), 18, (10, -128)),
        ];

        for (precision, scale, default_precision, expected) in cases {
            let column = meta("d", T_DECIMAL, precision, scale);

            assert_eq!(
                decimal_precision_scale(&column, default_precision),
                expected,
                "precision {precision:?}, scale {scale:?}"
            );
        }
    }
}
