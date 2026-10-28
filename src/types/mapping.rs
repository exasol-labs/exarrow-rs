//! Type mapping between Exasol and Apache Arrow data types.

use crate::error::ConversionError;
use arrow::datatypes::{DataType, IntervalUnit, Metadata, TimeUnit};
use serde::{Deserialize, Serialize};

/// Column name handling mode for DDL generation.
///
/// Controls how column names from source schemas are transformed
/// when generating Exasol CREATE TABLE DDL statements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnNameMode {
    /// Preserve original column names exactly, wrapped in double quotes.
    ///
    /// This mode:
    /// - Wraps all names in double quotes
    /// - Escapes internal double quotes by doubling them
    /// - Preserves case sensitivity and special characters
    ///
    /// Example: `my Column` becomes `"my Column"`
    #[default]
    Quoted,

    /// Sanitize column names to valid Exasol identifiers.
    ///
    /// This mode:
    /// - Converts names to uppercase
    /// - Replaces invalid identifier characters with underscores
    /// - Prefixes names starting with digits with an underscore
    ///
    /// Example: `my Column` becomes `MY_COLUMN`
    Sanitize,
}

/// The largest leading field precision of an Exasol INTERVAL type. The default of 2 holds at
/// most 99 years or days, while 9 holds every Arrow Interval(YearMonth) value.
const INTERVAL_LEADING_PRECISION: u8 = 9;

/// The largest precision of an Exasol DECIMAL column.
const MAX_DECIMAL_PRECISION: u8 = 36;

/// Exasol data type representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "UPPERCASE")]
pub enum ExasolType {
    /// BOOLEAN type
    #[serde(rename = "BOOLEAN")]
    Boolean,

    /// CHAR(n) type
    #[serde(rename = "CHAR")]
    Char { size: usize },

    /// VARCHAR(n) type
    #[serde(rename = "VARCHAR")]
    Varchar { size: usize },

    /// DECIMAL(p, s) type
    #[serde(rename = "DECIMAL")]
    Decimal { precision: u8, scale: i8 },

    /// DOUBLE PRECISION type
    #[serde(rename = "DOUBLE")]
    Double,

    /// DATE type
    #[serde(rename = "DATE")]
    Date,

    /// TIMESTAMP type
    #[serde(rename = "TIMESTAMP")]
    Timestamp { with_local_time_zone: bool },

    /// INTERVAL YEAR TO MONTH
    #[serde(rename = "INTERVAL YEAR TO MONTH")]
    IntervalYearToMonth,

    /// INTERVAL DAY TO SECOND
    #[serde(rename = "INTERVAL DAY TO SECOND")]
    IntervalDayToSecond { precision: u8 },

    /// GEOMETRY type
    #[serde(rename = "GEOMETRY")]
    Geometry { srid: Option<i32> },

    /// HASHTYPE type (for hash values)
    #[serde(rename = "HASHTYPE")]
    Hashtype { byte_size: usize },
}

impl ExasolType {
    /// The HASHTYPE that a result column description reports.
    ///
    /// Both protocols report the size of a HASHTYPE column as the number of characters of
    /// its rendered value, which is twice the byte size under the default `HASHTYPE_FORMAT`
    /// of `HEX`. Another session format (`UUID`, `BASE64`) changes the reported size, and
    /// no reported value is independent of the format, so the byte size assumes `HEX`.
    /// Without a reported size the byte size is 16.
    pub(crate) fn hashtype_from_reported_size(reported_size: Option<i64>) -> Self {
        const DEFAULT_BYTE_SIZE: usize = 16;
        const HEX_CHARACTERS_PER_BYTE: i64 = 2;
        let byte_size = reported_size
            .map(|size| (size / HEX_CHARACTERS_PER_BYTE) as usize)
            .unwrap_or(DEFAULT_BYTE_SIZE);
        ExasolType::Hashtype { byte_size }
    }

    /// The Exasol type name without parameters, such as `DECIMAL` or
    /// `TIMESTAMP WITH LOCAL TIME ZONE`.
    ///
    /// Precision, scale, and size are separate field metadata keys, which follows the
    /// JDBC `getColumnTypeName` convention.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            ExasolType::Boolean => "BOOLEAN",
            ExasolType::Char { .. } => "CHAR",
            ExasolType::Varchar { .. } => "VARCHAR",
            ExasolType::Decimal { .. } => "DECIMAL",
            ExasolType::Double => "DOUBLE",
            ExasolType::Date => "DATE",
            ExasolType::Timestamp {
                with_local_time_zone: true,
            } => "TIMESTAMP WITH LOCAL TIME ZONE",
            ExasolType::Timestamp {
                with_local_time_zone: false,
            } => "TIMESTAMP",
            ExasolType::IntervalYearToMonth => "INTERVAL YEAR TO MONTH",
            ExasolType::IntervalDayToSecond { .. } => "INTERVAL DAY TO SECOND",
            ExasolType::Geometry { .. } => "GEOMETRY",
            ExasolType::Hashtype { .. } => "HASHTYPE",
        }
    }

    /// Convert this Exasol type to a DDL type string suitable for CREATE TABLE statements.
    ///
    /// # Returns
    ///
    /// A string representing the Exasol DDL type, e.g., "VARCHAR(100)", "DECIMAL(18,2)".
    #[must_use]
    pub fn to_ddl_type(&self) -> String {
        match self {
            ExasolType::Boolean => "BOOLEAN".to_string(),
            ExasolType::Char { size } => format!("CHAR({size})"),
            ExasolType::Varchar { size } => format!("VARCHAR({size})"),
            ExasolType::Decimal { precision, scale } => format!("DECIMAL({precision},{scale})"),
            ExasolType::Double => "DOUBLE".to_string(),
            ExasolType::Date => "DATE".to_string(),
            ExasolType::Timestamp {
                with_local_time_zone,
            } => {
                if *with_local_time_zone {
                    "TIMESTAMP WITH LOCAL TIME ZONE".to_string()
                } else {
                    "TIMESTAMP".to_string()
                }
            }
            ExasolType::IntervalYearToMonth => {
                format!("INTERVAL YEAR({INTERVAL_LEADING_PRECISION}) TO MONTH")
            }
            ExasolType::IntervalDayToSecond { precision } => {
                format!("INTERVAL DAY({INTERVAL_LEADING_PRECISION}) TO SECOND({precision})")
            }
            ExasolType::Geometry { srid } => {
                if let Some(srid) = srid {
                    format!("GEOMETRY({srid})")
                } else {
                    "GEOMETRY".to_string()
                }
            }
            ExasolType::Hashtype { byte_size } => format!("HASHTYPE({} BYTE)", byte_size),
        }
    }
}

/// Type mapper for converting between Exasol and Arrow types.
pub struct TypeMapper;

impl TypeMapper {
    /// Convert an Exasol type to an Arrow DataType.
    ///
    /// # Arguments
    /// * `exasol_type` - The Exasol type to convert
    /// * `nullable` - Whether the field is nullable
    ///
    /// # Returns
    /// The corresponding Arrow DataType
    ///
    /// # Errors
    /// Returns `ConversionError::UnsupportedType` if the type cannot be mapped
    pub fn exasol_to_arrow(
        exasol_type: &ExasolType,
        nullable: bool,
    ) -> Result<DataType, ConversionError> {
        let _ = nullable; // Arrow nullability is handled at the Field level

        match exasol_type {
            ExasolType::Boolean => Ok(DataType::Boolean),

            ExasolType::Char { .. } | ExasolType::Varchar { .. } => Ok(DataType::Utf8),

            // Exasol DECIMAL precision is limited to 1-36 digits (per Exasol documentation).
            // Arrow Decimal128 supports up to 38 digits, so all Exasol decimals fit.
            // See: https://docs.exasol.com/db/latest/sql_references/data_types/data_type_size.htm
            ExasolType::Decimal { precision, scale } => {
                Ok(DataType::Decimal128(*precision, *scale))
            }

            ExasolType::Double => Ok(DataType::Float64),

            ExasolType::Date => Ok(DataType::Date32),

            ExasolType::Timestamp {
                with_local_time_zone,
            } => {
                if *with_local_time_zone {
                    // Timestamp with local timezone -> Timestamp with UTC timezone
                    Ok(DataType::Timestamp(
                        TimeUnit::Microsecond,
                        Some("UTC".into()),
                    ))
                } else {
                    // Timestamp without timezone
                    Ok(DataType::Timestamp(TimeUnit::Microsecond, None))
                }
            }

            ExasolType::IntervalYearToMonth => {
                // Map to MonthDayNano interval (only using month component)
                Ok(DataType::Interval(IntervalUnit::MonthDayNano))
            }

            ExasolType::IntervalDayToSecond { .. } => {
                // Map to MonthDayNano interval (using day and nanosecond components)
                Ok(DataType::Interval(IntervalUnit::MonthDayNano))
            }

            ExasolType::Geometry { .. } => {
                // Geometry as binary (WKB - Well-Known Binary)
                Ok(DataType::Binary)
            }

            ExasolType::Hashtype { .. } => {
                // Hash values as fixed-size binary
                Ok(DataType::Binary)
            }
        }
    }

    /// Map an Arrow type to the Exasol column type that a created table uses for it.
    ///
    /// Maps only the Arrow types that the import CSV conversion in `crate::import::arrow`
    /// converts, so a table created from this mapping can be filled by an import of the same
    /// data. Binary types map to `VARCHAR(2000000)`, which holds their lowercase hex text,
    /// because Exasol has no binary column type.
    ///
    /// # Errors
    /// Returns `ConversionError::UnsupportedType`, which names the Arrow type, for a type
    /// without an Exasol column type, such as Time64, a list, or a decimal outside Exasol's
    /// DECIMAL limits of precision 1 to 36 and a scale from 0 to the precision.
    pub fn arrow_to_exasol(arrow_type: &DataType) -> Result<ExasolType, ConversionError> {
        const LONGEST_VARCHAR: ExasolType = ExasolType::Varchar { size: 2_000_000 };

        match arrow_type {
            DataType::Boolean => Ok(ExasolType::Boolean),

            DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView
            | DataType::FixedSizeBinary(_) => Ok(LONGEST_VARCHAR),

            DataType::Dictionary(key_type, value_type)
                if key_type.is_dictionary_key_type()
                    && matches!(value_type.as_ref(), DataType::Utf8 | DataType::LargeUtf8) =>
            {
                Ok(LONGEST_VARCHAR)
            }

            DataType::Int8 | DataType::Int16 | DataType::Int32 => Ok(ExasolType::Decimal {
                precision: 18,
                scale: 0,
            }),

            DataType::Int64 => Ok(ExasolType::Decimal {
                precision: 36,
                scale: 0,
            }),

            DataType::UInt8 | DataType::UInt16 | DataType::UInt32 => Ok(ExasolType::Decimal {
                precision: 18,
                scale: 0,
            }),

            DataType::UInt64 => Ok(ExasolType::Decimal {
                precision: 36,
                scale: 0,
            }),

            DataType::Float32 | DataType::Float64 => Ok(ExasolType::Double),

            DataType::Decimal128(precision, scale) | DataType::Decimal256(precision, scale) => {
                if (1..=MAX_DECIMAL_PRECISION).contains(precision)
                    && (0..=*precision as i8).contains(scale)
                {
                    Ok(ExasolType::Decimal {
                        precision: *precision,
                        scale: *scale,
                    })
                } else {
                    Err(ConversionError::UnsupportedType {
                        exasol_type: format!(
                            "Arrow type {arrow_type:?}, because Exasol DECIMAL takes a precision from 1 to {MAX_DECIMAL_PRECISION} and a scale from 0 to the precision"
                        ),
                    })
                }
            }

            DataType::Date32 | DataType::Date64 => Ok(ExasolType::Date),

            DataType::Timestamp(_, tz) => Ok(ExasolType::Timestamp {
                with_local_time_zone: tz.is_some(),
            }),

            DataType::Interval(IntervalUnit::YearMonth) => Ok(ExasolType::IntervalYearToMonth),

            // Exasol stores INTERVAL DAY TO SECOND values in whole milliseconds.
            DataType::Interval(IntervalUnit::DayTime | IntervalUnit::MonthDayNano) => {
                Ok(ExasolType::IntervalDayToSecond { precision: 3 })
            }

            _ => Err(ConversionError::UnsupportedType {
                exasol_type: format!("Arrow type {:?}", arrow_type),
            }),
        }
    }

    /// Create Arrow field metadata to preserve Exasol type information.
    ///
    /// This allows round-tripping type information.
    pub fn create_field_metadata(exasol_type: &ExasolType) -> Metadata {
        let mut metadata = Metadata::new();

        metadata.insert(
            "exasol:type".to_string(),
            serde_json::to_string(exasol_type).unwrap_or_default(),
        );
        metadata.insert(
            "exasol:type_name".to_string(),
            exasol_type.type_name().to_string(),
        );

        match exasol_type {
            ExasolType::Char { size } => {
                metadata.insert("exasol:size".to_string(), size.to_string());
            }
            ExasolType::Varchar { size } => {
                metadata.insert("exasol:size".to_string(), size.to_string());
            }
            ExasolType::Decimal { precision, scale } => {
                metadata.insert("exasol:precision".to_string(), precision.to_string());
                metadata.insert("exasol:scale".to_string(), scale.to_string());
            }
            ExasolType::Geometry { srid: Some(srid) } => {
                metadata.insert("exasol:srid".to_string(), srid.to_string());
            }
            ExasolType::Geometry { srid: None } => {}
            _ => {}
        }

        metadata
    }

    /// Extract Exasol type from Arrow field metadata.
    pub fn from_field_metadata(metadata: &Metadata) -> Option<ExasolType> {
        metadata
            .get("exasol:type")
            .and_then(|s| serde_json::from_str(s).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boolean_mapping() {
        let exasol_type = ExasolType::Boolean;
        let arrow_type = TypeMapper::exasol_to_arrow(&exasol_type, true).unwrap();
        assert_eq!(arrow_type, DataType::Boolean);

        let reverse = TypeMapper::arrow_to_exasol(&arrow_type).unwrap();
        assert_eq!(reverse, ExasolType::Boolean);
    }

    #[test]
    fn test_string_mapping() {
        let exasol_type = ExasolType::Varchar { size: 100 };
        let arrow_type = TypeMapper::exasol_to_arrow(&exasol_type, true).unwrap();
        assert_eq!(arrow_type, DataType::Utf8);
    }

    #[test]
    fn test_decimal_mapping() {
        let exasol_type = ExasolType::Decimal {
            precision: 18,
            scale: 2,
        };
        let arrow_type = TypeMapper::exasol_to_arrow(&exasol_type, true).unwrap();
        assert_eq!(arrow_type, DataType::Decimal128(18, 2));

        let reverse = TypeMapper::arrow_to_exasol(&arrow_type).unwrap();
        assert_eq!(reverse, exasol_type);
    }

    const DICTIONARY_KEY_TYPES: [DataType; 8] = [
        DataType::Int8,
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
        DataType::UInt8,
        DataType::UInt16,
        DataType::UInt32,
        DataType::UInt64,
    ];

    const TIME_UNITS: [TimeUnit; 4] = [
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ];

    fn dictionary(key: DataType, value: DataType) -> DataType {
        DataType::Dictionary(Box::new(key), Box::new(value))
    }

    fn ddl_type_of(arrow_type: &DataType) -> String {
        TypeMapper::arrow_to_exasol(arrow_type)
            .unwrap_or_else(|e| panic!("{arrow_type:?}: {e}"))
            .to_ddl_type()
    }

    /// Scenario: String, binary, date, timestamp, interval, boolean, and decimal type mappings for DDL generation
    #[test]
    fn test_arrow_to_exasol_maps_string_binary_date_timestamp_interval_boolean_and_decimal_types() {
        let mut cases = vec![
            (DataType::Boolean, "BOOLEAN"),
            (DataType::Date32, "DATE"),
            (DataType::Date64, "DATE"),
            (DataType::Utf8, "VARCHAR(2000000)"),
            (DataType::LargeUtf8, "VARCHAR(2000000)"),
            (DataType::Utf8View, "VARCHAR(2000000)"),
            (DataType::Binary, "VARCHAR(2000000)"),
            (DataType::LargeBinary, "VARCHAR(2000000)"),
            (DataType::BinaryView, "VARCHAR(2000000)"),
            (DataType::FixedSizeBinary(1), "VARCHAR(2000000)"),
            (DataType::FixedSizeBinary(16), "VARCHAR(2000000)"),
            (DataType::Decimal128(1, 0), "DECIMAL(1,0)"),
            (DataType::Decimal128(36, 2), "DECIMAL(36,2)"),
            (DataType::Decimal128(10, 10), "DECIMAL(10,10)"),
            (DataType::Decimal256(36, 0), "DECIMAL(36,0)"),
            (DataType::Decimal256(36, 36), "DECIMAL(36,36)"),
            (
                DataType::Interval(IntervalUnit::YearMonth),
                "INTERVAL YEAR(9) TO MONTH",
            ),
            (
                DataType::Interval(IntervalUnit::DayTime),
                "INTERVAL DAY(9) TO SECOND(3)",
            ),
            (
                DataType::Interval(IntervalUnit::MonthDayNano),
                "INTERVAL DAY(9) TO SECOND(3)",
            ),
        ];
        for unit in TIME_UNITS {
            cases.push((DataType::Timestamp(unit, None), "TIMESTAMP"));
            cases.push((
                DataType::Timestamp(unit, Some("Europe/Berlin".into())),
                "TIMESTAMP WITH LOCAL TIME ZONE",
            ));
        }
        for key in DICTIONARY_KEY_TYPES {
            cases.push((dictionary(key.clone(), DataType::Utf8), "VARCHAR(2000000)"));
            cases.push((dictionary(key, DataType::LargeUtf8), "VARCHAR(2000000)"));
        }

        for (arrow_type, expected) in cases {
            assert_eq!(ddl_type_of(&arrow_type), expected, "{arrow_type:?}");
        }
    }

    /// Scenario: Arrow types without an Exasol column type are rejected
    #[test]
    fn test_arrow_to_exasol_rejects_types_without_an_exasol_column_type() {
        use arrow::datatypes::{Field, Fields, UnionFields, UnionMode};
        use std::sync::Arc;

        let item = Arc::new(Field::new("item", DataType::Int32, true));
        let entries = Arc::new(Field::new(
            "entries",
            DataType::Struct(Fields::from(vec![
                Field::new("key", DataType::Utf8, false),
                Field::new("value", DataType::Int32, true),
            ])),
            false,
        ));
        let mut cases = vec![
            DataType::Decimal128(37, 0),
            DataType::Decimal128(38, 2),
            DataType::Decimal256(40, 5),
            DataType::Decimal256(76, -10),
            DataType::Decimal128(10, -2),
            DataType::Decimal128(5, 6),
            DataType::Decimal256(36, -1),
            DataType::Time32(TimeUnit::Second),
            DataType::Time32(TimeUnit::Millisecond),
            DataType::Time64(TimeUnit::Microsecond),
            DataType::Time64(TimeUnit::Nanosecond),
            DataType::Null,
            DataType::Float16,
            DataType::List(item.clone()),
            DataType::LargeList(item.clone()),
            DataType::FixedSizeList(item.clone(), 2),
            DataType::Struct(Fields::from(vec![Field::new("a", DataType::Int32, true)])),
            DataType::Map(entries, false),
            DataType::Union(
                UnionFields::try_new(vec![0], vec![Field::new("a", DataType::Int32, true)])
                    .unwrap(),
                UnionMode::Sparse,
            ),
            dictionary(DataType::Int32, DataType::Int32),
            dictionary(DataType::Int8, DataType::Binary),
            dictionary(DataType::UInt16, DataType::Utf8View),
        ];
        cases.extend(TIME_UNITS.map(DataType::Duration));

        for arrow_type in cases {
            let error = TypeMapper::arrow_to_exasol(&arrow_type).unwrap_err();
            assert!(
                matches!(&error, ConversionError::UnsupportedType { .. })
                    && error.to_string().contains(&format!("{arrow_type:?}")),
                "{arrow_type:?}: {error}"
            );
        }
    }

    #[test]
    fn test_timestamp_mapping() {
        let exasol_type = ExasolType::Timestamp {
            with_local_time_zone: false,
        };
        let arrow_type = TypeMapper::exasol_to_arrow(&exasol_type, true).unwrap();
        assert!(matches!(
            arrow_type,
            DataType::Timestamp(TimeUnit::Microsecond, None)
        ));
    }

    #[test]
    fn test_timestamp_with_tz_mapping() {
        let exasol_type = ExasolType::Timestamp {
            with_local_time_zone: true,
        };
        let arrow_type = TypeMapper::exasol_to_arrow(&exasol_type, true).unwrap();
        assert!(matches!(
            arrow_type,
            DataType::Timestamp(TimeUnit::Microsecond, Some(_))
        ));
    }

    #[test]
    fn test_date_mapping() {
        let exasol_type = ExasolType::Date;
        let arrow_type = TypeMapper::exasol_to_arrow(&exasol_type, true).unwrap();
        assert_eq!(arrow_type, DataType::Date32);
    }

    #[test]
    fn test_metadata_preservation() {
        let exasol_type = ExasolType::Decimal {
            precision: 18,
            scale: 2,
        };

        let metadata = TypeMapper::create_field_metadata(&exasol_type);
        assert!(metadata.contains_key("exasol:type"));
        assert_eq!(metadata.get("exasol:precision"), Some(&"18".to_string()));
        assert_eq!(metadata.get("exasol:scale"), Some(&"2".to_string()));

        let restored = TypeMapper::from_field_metadata(&metadata).unwrap();
        assert_eq!(restored, exasol_type);
    }

    #[test]
    fn test_type_name_of_every_exasol_type_has_no_parameters() {
        let cases = [
            (ExasolType::Boolean, "BOOLEAN"),
            (ExasolType::Char { size: 3 }, "CHAR"),
            (ExasolType::Varchar { size: 50 }, "VARCHAR"),
            (
                ExasolType::Decimal {
                    precision: 10,
                    scale: 2,
                },
                "DECIMAL",
            ),
            (ExasolType::Double, "DOUBLE"),
            (ExasolType::Date, "DATE"),
            (
                ExasolType::Timestamp {
                    with_local_time_zone: false,
                },
                "TIMESTAMP",
            ),
            (
                ExasolType::Timestamp {
                    with_local_time_zone: true,
                },
                "TIMESTAMP WITH LOCAL TIME ZONE",
            ),
            (ExasolType::IntervalYearToMonth, "INTERVAL YEAR TO MONTH"),
            (
                ExasolType::IntervalDayToSecond { precision: 3 },
                "INTERVAL DAY TO SECOND",
            ),
            (ExasolType::Geometry { srid: Some(4326) }, "GEOMETRY"),
            (ExasolType::Geometry { srid: None }, "GEOMETRY"),
            (ExasolType::Hashtype { byte_size: 32 }, "HASHTYPE"),
        ];

        for (exasol_type, expected) in cases {
            assert_eq!(exasol_type.type_name(), expected, "{exasol_type:?}");
        }
    }

    fn every_exasol_type() -> Vec<ExasolType> {
        vec![
            ExasolType::Boolean,
            ExasolType::Char { size: 3 },
            ExasolType::Varchar { size: 50 },
            ExasolType::Decimal {
                precision: 10,
                scale: 2,
            },
            ExasolType::Double,
            ExasolType::Date,
            ExasolType::Timestamp {
                with_local_time_zone: false,
            },
            ExasolType::Timestamp {
                with_local_time_zone: true,
            },
            ExasolType::IntervalYearToMonth,
            ExasolType::IntervalDayToSecond { precision: 3 },
            ExasolType::Geometry { srid: Some(4326) },
            ExasolType::Hashtype { byte_size: 32 },
        ]
    }

    #[test]
    fn test_create_field_metadata_writes_type_name_for_every_exasol_type() {
        for exasol_type in every_exasol_type() {
            let metadata = TypeMapper::create_field_metadata(&exasol_type);

            assert_eq!(
                metadata.get("exasol:type_name").map(String::as_str),
                Some(exasol_type.type_name()),
                "{exasol_type:?}"
            );
        }
    }

    #[test]
    fn test_from_field_metadata_restores_every_exasol_type() {
        for exasol_type in every_exasol_type() {
            let metadata = TypeMapper::create_field_metadata(&exasol_type);

            assert_eq!(
                TypeMapper::from_field_metadata(&metadata),
                Some(exasol_type)
            );
        }
    }

    #[test]
    fn test_hashtype_from_reported_size_halves_the_hex_character_count() {
        assert_eq!(
            ExasolType::hashtype_from_reported_size(Some(64)),
            ExasolType::Hashtype { byte_size: 32 }
        );
        assert_eq!(
            ExasolType::hashtype_from_reported_size(Some(32)),
            ExasolType::Hashtype { byte_size: 16 }
        );
    }

    #[test]
    fn test_hashtype_from_reported_size_defaults_to_sixteen_bytes() {
        assert_eq!(
            ExasolType::hashtype_from_reported_size(None),
            ExasolType::Hashtype { byte_size: 16 }
        );
    }

    #[test]
    fn test_geometry_mapping() {
        let exasol_type = ExasolType::Geometry { srid: Some(4326) };
        let arrow_type = TypeMapper::exasol_to_arrow(&exasol_type, true).unwrap();
        assert_eq!(arrow_type, DataType::Binary);
    }

    #[test]
    fn test_interval_mapping() {
        let exasol_type = ExasolType::IntervalDayToSecond { precision: 3 };
        let arrow_type = TypeMapper::exasol_to_arrow(&exasol_type, true).unwrap();
        assert!(matches!(
            arrow_type,
            DataType::Interval(IntervalUnit::MonthDayNano)
        ));
    }

    #[test]
    fn test_uint_to_exasol_mapping() {
        // UInt8, UInt16, UInt32 -> DECIMAL(18,0)
        let uint8 = TypeMapper::arrow_to_exasol(&DataType::UInt8).unwrap();
        assert_eq!(
            uint8,
            ExasolType::Decimal {
                precision: 18,
                scale: 0
            }
        );

        let uint16 = TypeMapper::arrow_to_exasol(&DataType::UInt16).unwrap();
        assert_eq!(
            uint16,
            ExasolType::Decimal {
                precision: 18,
                scale: 0
            }
        );

        let uint32 = TypeMapper::arrow_to_exasol(&DataType::UInt32).unwrap();
        assert_eq!(
            uint32,
            ExasolType::Decimal {
                precision: 18,
                scale: 0
            }
        );

        // UInt64 -> DECIMAL(36,0)
        let uint64 = TypeMapper::arrow_to_exasol(&DataType::UInt64).unwrap();
        assert_eq!(
            uint64,
            ExasolType::Decimal {
                precision: 36,
                scale: 0
            }
        );
    }

    #[test]
    fn test_to_ddl_type_boolean() {
        let t = ExasolType::Boolean;
        assert_eq!(t.to_ddl_type(), "BOOLEAN");
    }

    #[test]
    fn test_to_ddl_type_char() {
        let t = ExasolType::Char { size: 50 };
        assert_eq!(t.to_ddl_type(), "CHAR(50)");
    }

    #[test]
    fn test_to_ddl_type_varchar() {
        let t = ExasolType::Varchar { size: 2000000 };
        assert_eq!(t.to_ddl_type(), "VARCHAR(2000000)");
    }

    #[test]
    fn test_to_ddl_type_decimal() {
        let t = ExasolType::Decimal {
            precision: 18,
            scale: 2,
        };
        assert_eq!(t.to_ddl_type(), "DECIMAL(18,2)");
    }

    #[test]
    fn test_to_ddl_type_double() {
        let t = ExasolType::Double;
        assert_eq!(t.to_ddl_type(), "DOUBLE");
    }

    #[test]
    fn test_to_ddl_type_date() {
        let t = ExasolType::Date;
        assert_eq!(t.to_ddl_type(), "DATE");
    }

    #[test]
    fn test_to_ddl_type_timestamp() {
        let t = ExasolType::Timestamp {
            with_local_time_zone: false,
        };
        assert_eq!(t.to_ddl_type(), "TIMESTAMP");

        let t_tz = ExasolType::Timestamp {
            with_local_time_zone: true,
        };
        assert_eq!(t_tz.to_ddl_type(), "TIMESTAMP WITH LOCAL TIME ZONE");
    }

    #[test]
    fn test_to_ddl_type_interval() {
        let t = ExasolType::IntervalYearToMonth;
        assert_eq!(t.to_ddl_type(), "INTERVAL YEAR(9) TO MONTH");

        let t2 = ExasolType::IntervalDayToSecond { precision: 6 };
        assert_eq!(t2.to_ddl_type(), "INTERVAL DAY(9) TO SECOND(6)");
    }

    #[test]
    fn test_to_ddl_type_geometry() {
        let t = ExasolType::Geometry { srid: None };
        assert_eq!(t.to_ddl_type(), "GEOMETRY");

        let t_srid = ExasolType::Geometry { srid: Some(4326) };
        assert_eq!(t_srid.to_ddl_type(), "GEOMETRY(4326)");
    }

    #[test]
    fn test_to_ddl_type_hashtype() {
        let t = ExasolType::Hashtype { byte_size: 16 };
        assert_eq!(t.to_ddl_type(), "HASHTYPE(16 BYTE)");
    }

    #[test]
    fn test_column_name_mode_default() {
        let mode = ColumnNameMode::default();
        assert_eq!(mode, ColumnNameMode::Quoted);
    }
}
