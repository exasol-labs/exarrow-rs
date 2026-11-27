# Intent: add-binding-arrow-types

Source: https://github.com/exasol-labs/exarrow-rs/issues/110

## Interview

- Q: How should binding convert the new types? → A: Reuse the import CSV conversion
- Q: Decimal256 value that does not fit DECIMAL(36, s)? → A: Driver rejects it before execution
- Q: Interval and Duration? → A: Interval yes with the import's lossless rules, Duration stays NotImplemented
- Q: Which Dictionary types? → A: String values only, as ingest does
- Q: Native DECIMAL parameters look clamped to i64, text becomes 0. Fix here? → A: Fix it in this plan
- Q: INTERVAL parameters over native are untested. How to handle? → A: Spec on both transports, fix native encoding if needed

## Directions

- [D1] The import CSV conversion exposes each value's text before CSV quoting, and binding uses that text; CSV quoting would corrupt a bound string that holds a comma or quote.
- [D2] Binding resolves a column's type before its null check, so an unsupported column of only nulls fails with NotImplemented instead of binding NULL; import already behaves this way.
- [D3] The Arrow `Null` type binds as NULL although ingest rejects it, because a parameter's Exasol type comes from the statement, not from the Arrow column.
- [D4] Native rejects an integer or decimal text DECIMAL value with more digits after the point than the scale instead of rounding, while a float value is still rounded as today; over WebSocket Exasol casts decimal text itself, so the transports can differ for such text.
- [D5] Integer types bind as integer parameters when the value fits i64, and as decimal text otherwise, so UInt64 values above i64::MAX keep every digit.
- [D6] `docs/type-mapping.md` lists the bindable Arrow types and the ones that stay unsupported: Time32, Time64, Duration, List, and a Dictionary with non-string values.
- [D7] Native DECIMAL parameter layout is assumed to follow the result layout: `T_DECIMAL` with a 4-byte value up to precision 9, 8 bytes up to 18, and 16 bytes above 18; if Exasol rejects it, send the value as `T_CHAR` text and let Exasol cast it. `test_native_decimal_parameters_round_trip` covers one precision per band.
