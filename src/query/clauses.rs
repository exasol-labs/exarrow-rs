//! SQL clause fragments shared by the IMPORT and EXPORT statement builders.
//!
//! Exasol spells the HTTP endpoint and the CSV framing options identically in
//! both statements. Keeping that spelling in one place means a change to it —
//! a new scheme, a different quoting rule — cannot silently apply to only one
//! of the two builders.

/// Render the quoted URL of an `AT` clause plus its optional `PUBLIC KEY` clause.
///
/// A public-key fingerprint both selects the `https` scheme and pins the
/// server certificate, so the two are decided together rather than left to
/// each caller. `url_suffix` is appended inside the quotes, which is where
/// Exasol expects per-URL connection options.
pub(crate) fn quoted_endpoint(address: &str, url_suffix: &str, public_key: Option<&str>) -> String {
    match public_key {
        Some(fingerprint) => format!(
            "'https://{}{}' PUBLIC KEY '{}'",
            address, url_suffix, fingerprint
        ),
        None => format!("'http://{}{}'", address, url_suffix),
    }
}

/// The CSV framing options that IMPORT and EXPORT share verbatim.
///
/// Named fields rather than positional arguments, because the two `char`
/// separators are otherwise trivially swappable at a call site.
pub(crate) struct CsvFraming<'a> {
    pub encoding: &'a str,
    pub column_separator: char,
    pub column_delimiter: char,
    pub row_separator: &'a str,
}

impl CsvFraming<'_> {
    /// Render the `ENCODING`/`COLUMN SEPARATOR`/`COLUMN DELIMITER`/`ROW SEPARATOR`
    /// clauses, each on its own line and led by a newline.
    pub(crate) fn to_sql(&self) -> String {
        format!(
            "\nENCODING = '{}'\nCOLUMN SEPARATOR = '{}'\nCOLUMN DELIMITER = '{}'\nROW SEPARATOR = '{}'",
            self.encoding, self.column_separator, self.column_delimiter, self.row_separator
        )
    }
}

/// Render the optional `NULL = '...'` clause; empty when the default (an empty
/// field) is wanted.
pub(crate) fn null_clause(null_value: Option<&str>) -> String {
    null_value.map_or_else(String::new, |value| format!("\nNULL = '{}'", value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quoted_endpoint_uses_http_when_unpinned() {
        assert_eq!(
            quoted_endpoint("10.0.0.1:8080", "", None),
            "'http://10.0.0.1:8080'"
        );
    }

    #[test]
    fn test_quoted_endpoint_uses_https_and_appends_public_key_when_pinned() {
        assert_eq!(
            quoted_endpoint("10.0.0.1:8080", "", Some("sha256//abc")),
            "'https://10.0.0.1:8080' PUBLIC KEY 'sha256//abc'"
        );
    }

    #[test]
    fn test_quoted_endpoint_places_the_suffix_inside_the_quotes() {
        assert_eq!(
            quoted_endpoint("10.0.0.1:8080", ";MaxConcurrentReads=1", None),
            "'http://10.0.0.1:8080;MaxConcurrentReads=1'"
        );
        assert_eq!(
            quoted_endpoint("10.0.0.1:8080", ";MaxConcurrentReads=1", Some("fp")),
            "'https://10.0.0.1:8080;MaxConcurrentReads=1' PUBLIC KEY 'fp'"
        );
    }

    #[test]
    fn test_quoted_endpoint_with_empty_address_renders_scheme_only() {
        assert_eq!(quoted_endpoint("", "", None), "'http://'");
    }

    #[test]
    fn test_csv_framing_renders_all_four_clauses_in_order() {
        let framing = CsvFraming {
            encoding: "UTF-8",
            column_separator: ',',
            column_delimiter: '"',
            row_separator: "LF",
        };

        assert_eq!(
            framing.to_sql(),
            "\nENCODING = 'UTF-8'\nCOLUMN SEPARATOR = ','\nCOLUMN DELIMITER = '\"'\nROW SEPARATOR = 'LF'"
        );
    }

    #[test]
    fn test_csv_framing_passes_through_custom_characters() {
        let framing = CsvFraming {
            encoding: "ISO-8859-1",
            column_separator: ';',
            column_delimiter: '\'',
            row_separator: "CRLF",
        };

        assert_eq!(
            framing.to_sql(),
            "\nENCODING = 'ISO-8859-1'\nCOLUMN SEPARATOR = ';'\nCOLUMN DELIMITER = '''\nROW SEPARATOR = 'CRLF'"
        );
    }

    #[test]
    fn test_null_clause_is_empty_when_no_representation_is_set() {
        assert_eq!(null_clause(None), "");
    }

    #[test]
    fn test_null_clause_quotes_the_configured_representation() {
        assert_eq!(null_clause(Some("\\N")), "\nNULL = '\\N'");
    }
}
