use super::constants::{
    T_BIGDECIMAL, T_BINARY, T_BOOLEAN, T_CHAR, T_DATE, T_DECIMAL, T_DOUBLE, T_GEOMETRY, T_HASHTYPE,
    T_INTEGER, T_INTERVAL_DAY, T_INTERVAL_YEAR, T_REAL, T_SMALLDECIMAL, T_SMALLINT, T_TIMESTAMP,
    T_TIMESTAMP_LOCAL_TZ, T_TIMESTAMP_UTC,
};
use super::result_parser::NativeColumnMeta;

/// Map Exasol native type metadata to the corresponding ColumnInfo DataType for compatibility.
pub fn native_meta_to_data_type(meta: &NativeColumnMeta) -> crate::transport::messages::DataType {
    match meta.type_id {
        T_DECIMAL | T_SMALLDECIMAL | T_BIGDECIMAL => crate::transport::messages::DataType {
            type_name: "DECIMAL".to_string(),
            precision: meta.precision,
            scale: meta.scale,
            size: None,
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        },
        T_DOUBLE => crate::transport::messages::DataType::double(),
        T_REAL => crate::transport::messages::DataType {
            type_name: "DOUBLE".to_string(),
            precision: None,
            scale: None,
            size: None,
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        },
        T_INTEGER => crate::transport::messages::DataType::decimal(18, 0),
        T_SMALLINT => crate::transport::messages::DataType::decimal(9, 0),
        T_BOOLEAN => crate::transport::messages::DataType::boolean(),
        T_CHAR => {
            if meta.is_varchar {
                crate::transport::messages::DataType::varchar(
                    meta.max_len.unwrap_or(2_000_000) as i64
                )
            } else {
                crate::transport::messages::DataType {
                    type_name: "CHAR".to_string(),
                    precision: None,
                    scale: None,
                    size: meta.max_len.map(|l| l as i64),
                    character_set: Some("UTF8".to_string()),
                    with_local_time_zone: None,
                    fraction: None,
                }
            }
        }
        T_DATE => crate::transport::messages::DataType {
            type_name: "DATE".to_string(),
            precision: None,
            scale: None,
            size: None,
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        },
        T_TIMESTAMP => crate::transport::messages::DataType {
            type_name: "TIMESTAMP".to_string(),
            precision: None,
            scale: None,
            size: None,
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        },
        T_TIMESTAMP_LOCAL_TZ => crate::transport::messages::DataType {
            type_name: "TIMESTAMP WITH LOCAL TIME ZONE".to_string(),
            precision: None,
            scale: None,
            size: None,
            character_set: None,
            with_local_time_zone: Some(true),
            fraction: None,
        },
        T_TIMESTAMP_UTC => crate::transport::messages::DataType {
            type_name: "TIMESTAMP WITH LOCAL TIME ZONE".to_string(),
            precision: None,
            scale: None,
            size: None,
            character_set: None,
            with_local_time_zone: Some(true),
            fraction: None,
        },
        T_BINARY => crate::transport::messages::DataType {
            type_name: "BINARY".to_string(),
            precision: None,
            scale: None,
            size: meta.max_len.map(|l| l as i64),
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        },
        T_GEOMETRY => crate::transport::messages::DataType {
            type_name: "GEOMETRY".to_string(),
            precision: None,
            scale: None,
            size: None,
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        },
        T_HASHTYPE => crate::transport::messages::DataType {
            type_name: "HASHTYPE".to_string(),
            precision: None,
            scale: None,
            size: None,
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        },
        T_INTERVAL_YEAR => crate::transport::messages::DataType {
            type_name: "INTERVAL YEAR TO MONTH".to_string(),
            precision: None,
            scale: None,
            size: None,
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        },
        T_INTERVAL_DAY => crate::transport::messages::DataType {
            type_name: "INTERVAL DAY TO SECOND".to_string(),
            precision: None,
            scale: None,
            size: None,
            character_set: None,
            with_local_time_zone: None,
            fraction: None,
        },
        _ => crate::transport::messages::DataType::varchar(2_000_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A type id no Exasol wire type uses, exercising the varchar fallback.
    const UNKNOWN_TYPE_ID: u32 = 9_999;

    /// One row of the type-mapping table: the native metadata that goes in and
    /// every `DataType` field that is expected to come out.
    struct Case {
        type_id: u32,
        is_varchar: bool,
        max_len: Option<i32>,
        type_name: &'static str,
        precision: Option<i32>,
        scale: Option<i32>,
        size: Option<i64>,
        character_set: Option<&'static str>,
        with_local_time_zone: Option<bool>,
    }

    impl Default for Case {
        fn default() -> Self {
            Self {
                type_id: UNKNOWN_TYPE_ID,
                is_varchar: false,
                max_len: None,
                type_name: "VARCHAR",
                precision: None,
                scale: None,
                size: None,
                character_set: None,
                with_local_time_zone: None,
            }
        }
    }

    #[test]
    fn maps_every_native_type_id_to_its_data_type() {
        let cases = vec![
            Case {
                type_id: T_DECIMAL,
                type_name: "DECIMAL",
                precision: Some(12),
                scale: Some(3),
                ..Case::default()
            },
            Case {
                type_id: T_SMALLDECIMAL,
                type_name: "DECIMAL",
                precision: Some(12),
                scale: Some(3),
                ..Case::default()
            },
            Case {
                type_id: T_BIGDECIMAL,
                type_name: "DECIMAL",
                precision: Some(12),
                scale: Some(3),
                ..Case::default()
            },
            Case {
                type_id: T_DOUBLE,
                type_name: "DOUBLE",
                ..Case::default()
            },
            Case {
                type_id: T_REAL,
                type_name: "DOUBLE",
                ..Case::default()
            },
            Case {
                type_id: T_INTEGER,
                type_name: "DECIMAL",
                precision: Some(18),
                scale: Some(0),
                ..Case::default()
            },
            Case {
                type_id: T_SMALLINT,
                type_name: "DECIMAL",
                precision: Some(9),
                scale: Some(0),
                ..Case::default()
            },
            Case {
                type_id: T_BOOLEAN,
                type_name: "BOOLEAN",
                ..Case::default()
            },
            Case {
                type_id: T_CHAR,
                is_varchar: true,
                max_len: Some(100),
                type_name: "VARCHAR",
                size: Some(100),
                character_set: Some("UTF8"),
                ..Case::default()
            },
            Case {
                type_id: T_CHAR,
                is_varchar: true,
                type_name: "VARCHAR",
                size: Some(2_000_000),
                character_set: Some("UTF8"),
                ..Case::default()
            },
            Case {
                type_id: T_CHAR,
                max_len: Some(20),
                type_name: "CHAR",
                size: Some(20),
                character_set: Some("UTF8"),
                ..Case::default()
            },
            Case {
                type_id: T_CHAR,
                type_name: "CHAR",
                character_set: Some("UTF8"),
                ..Case::default()
            },
            Case {
                type_id: T_DATE,
                type_name: "DATE",
                ..Case::default()
            },
            Case {
                type_id: T_TIMESTAMP,
                type_name: "TIMESTAMP",
                ..Case::default()
            },
            Case {
                type_id: T_TIMESTAMP_LOCAL_TZ,
                type_name: "TIMESTAMP WITH LOCAL TIME ZONE",
                with_local_time_zone: Some(true),
                ..Case::default()
            },
            Case {
                type_id: T_TIMESTAMP_UTC,
                type_name: "TIMESTAMP WITH LOCAL TIME ZONE",
                with_local_time_zone: Some(true),
                ..Case::default()
            },
            Case {
                type_id: T_BINARY,
                max_len: Some(64),
                type_name: "BINARY",
                size: Some(64),
                ..Case::default()
            },
            Case {
                type_id: T_BINARY,
                type_name: "BINARY",
                ..Case::default()
            },
            Case {
                type_id: T_GEOMETRY,
                type_name: "GEOMETRY",
                ..Case::default()
            },
            Case {
                type_id: T_HASHTYPE,
                type_name: "HASHTYPE",
                ..Case::default()
            },
            Case {
                type_id: T_INTERVAL_YEAR,
                type_name: "INTERVAL YEAR TO MONTH",
                ..Case::default()
            },
            Case {
                type_id: T_INTERVAL_DAY,
                type_name: "INTERVAL DAY TO SECOND",
                ..Case::default()
            },
            Case {
                type_id: UNKNOWN_TYPE_ID,
                size: Some(2_000_000),
                character_set: Some("UTF8"),
                ..Case::default()
            },
        ];

        for case in cases {
            let meta = NativeColumnMeta {
                name: "c".to_string(),
                type_id: case.type_id,
                precision: Some(12),
                scale: Some(3),
                is_varchar: case.is_varchar,
                max_len: case.max_len,
            };

            let dt = native_meta_to_data_type(&meta);
            let label = format!(
                "type_id {} (is_varchar={}, max_len={:?})",
                case.type_id, case.is_varchar, case.max_len
            );

            assert_eq!(dt.type_name, case.type_name, "type_name for {label}");
            assert_eq!(dt.precision, case.precision, "precision for {label}");
            assert_eq!(dt.scale, case.scale, "scale for {label}");
            assert_eq!(dt.size, case.size, "size for {label}");
            assert_eq!(
                dt.character_set.as_deref(),
                case.character_set,
                "character_set for {label}"
            );
            assert_eq!(
                dt.with_local_time_zone, case.with_local_time_zone,
                "with_local_time_zone for {label}"
            );
            assert_eq!(dt.fraction, None, "fraction for {label}");
        }
    }

    #[test]
    fn maps_integer_to_decimal_18_0() {
        let meta = NativeColumnMeta {
            name: "x".to_string(),
            type_id: T_INTEGER,
            precision: None,
            scale: None,
            is_varchar: false,
            max_len: None,
        };
        let dt = native_meta_to_data_type(&meta);
        assert_eq!(dt.type_name, "DECIMAL");
        assert_eq!(dt.precision, Some(18));
        assert_eq!(dt.scale, Some(0));
    }

    #[test]
    fn maps_char_with_varchar_flag_to_varchar() {
        let meta = NativeColumnMeta {
            name: "name".to_string(),
            type_id: T_CHAR,
            precision: None,
            scale: None,
            is_varchar: true,
            max_len: Some(100),
        };
        let dt = native_meta_to_data_type(&meta);
        assert_eq!(dt.type_name, "VARCHAR");
        assert_eq!(dt.size, Some(100));
    }

    #[test]
    fn maps_timestamp_utc_with_time_zone_flag() {
        let meta = NativeColumnMeta {
            name: "ts".to_string(),
            type_id: T_TIMESTAMP_UTC,
            precision: None,
            scale: None,
            is_varchar: false,
            max_len: None,
        };
        let dt = native_meta_to_data_type(&meta);
        assert_eq!(dt.type_name, "TIMESTAMP WITH LOCAL TIME ZONE");
        assert_eq!(dt.with_local_time_zone, Some(true));
    }
}
