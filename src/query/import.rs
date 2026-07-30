//! Import query builder for generating IMPORT SQL statements.
//!
//! This module provides a builder pattern for constructing Exasol IMPORT statements
//! that import data from HTTP endpoints into database tables.
//!
//! # Example
//!

use super::clauses;

/// Row separator options for CSV import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowSeparator {
    /// Line feed (Unix-style)
    #[default]
    LF,
    /// Carriage return (old Mac-style)
    CR,
    /// Carriage return + line feed (Windows-style)
    CRLF,
}

impl RowSeparator {
    /// Convert to SQL string representation.
    pub fn to_sql(&self) -> &'static str {
        match self {
            RowSeparator::LF => "LF",
            RowSeparator::CR => "CR",
            RowSeparator::CRLF => "CRLF",
        }
    }
}

/// Compression options for import files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// No compression
    #[default]
    None,
    /// Gzip compression (.gz extension)
    Gzip,
    /// Bzip2 compression (.bz2 extension)
    Bzip2,
}

impl Compression {
    /// Get the file extension for this compression type.
    pub fn extension(&self) -> &'static str {
        match self {
            Compression::None => ".csv",
            Compression::Gzip => ".csv.gz",
            Compression::Bzip2 => ".csv.bz2",
        }
    }
}

/// File format for an IMPORT statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportFormat {
    /// CSV format. Format options (encoding, separators, etc.) apply.
    #[default]
    Csv,
    /// Native Parquet format. The server detects the schema from the file;
    /// CSV format options and compression suffixes do not apply.
    Parquet,
}

/// Trim mode options for CSV import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrimMode {
    /// No trimming
    #[default]
    None,
    /// Trim leading whitespace
    LTrim,
    /// Trim trailing whitespace
    RTrim,
    /// Trim both leading and trailing whitespace
    Trim,
}

impl TrimMode {
    /// Convert to SQL string representation.
    pub fn to_sql(&self) -> Option<&'static str> {
        match self {
            TrimMode::None => None,
            TrimMode::LTrim => Some("LTRIM"),
            TrimMode::RTrim => Some("RTRIM"),
            TrimMode::Trim => Some("TRIM"),
        }
    }
}

/// Entry for a single file in a multi-file IMPORT statement.
///
/// Used with `ImportQuery::with_files()` to build IMPORT statements
/// that reference multiple FILE clauses for parallel import.
#[derive(Debug, Clone)]
pub struct ImportFileEntry {
    /// Internal address from EXA handshake (format: "host:port")
    pub address: String,
    /// File name for this entry (e.g., "001.csv", "002.csv")
    pub file_name: String,
    /// Optional public key fingerprint for TLS
    pub public_key: Option<String>,
}

impl ImportFileEntry {
    /// Create a new import file entry.
    ///
    /// # Arguments
    ///
    /// * `address` - Internal address from EXA handshake
    /// * `file_name` - File name for this entry
    /// * `public_key` - Optional public key fingerprint for TLS
    pub fn new(address: String, file_name: String, public_key: Option<String>) -> Self {
        Self {
            address,
            file_name,
            public_key,
        }
    }
}

/// Builder for constructing Exasol IMPORT SQL statements.
///
/// The ImportQuery builder allows you to configure all aspects of an IMPORT statement
/// including the target table, columns, CSV format options, and error handling.
///
/// # Multi-file Import
///
/// For parallel imports, use `with_files()` instead of `at_address()` and `file_name()`:
///
/// ```rust
/// use exarrow_rs::query::import::{ImportQuery, ImportFileEntry};
///
/// let entries = vec![
///     ImportFileEntry::new("10.0.0.5:8563".to_string(), "001.csv".to_string(), None),
///     ImportFileEntry::new("10.0.0.6:8563".to_string(), "002.csv".to_string(), None),
/// ];
///
/// let query = ImportQuery::new("my_table")
///     .with_files(entries)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct ImportQuery {
    /// Target table name (required)
    table: String,
    /// Target schema (optional)
    schema: Option<String>,
    /// Columns to import into (optional, imports all if not specified)
    columns: Option<Vec<String>>,
    /// HTTP address for the CSV source (e.g., "192.168.1.1:8080") - single file mode
    address: Option<String>,
    /// SHA-256 fingerprint for TLS public key verification - single file mode
    public_key: Option<String>,
    /// File name (default "001.csv") - single file mode
    file_name: String,
    /// Multiple file entries for parallel import
    file_entries: Option<Vec<ImportFileEntry>>,
    /// Column separator character (default ',')
    column_separator: char,
    /// Column delimiter character for quoting (default '"')
    column_delimiter: char,
    /// Row separator (default LF)
    row_separator: RowSeparator,
    /// Character encoding (default "UTF-8")
    encoding: String,
    /// Number of header rows to skip (default 0)
    skip: u32,
    /// Custom NULL value representation
    null_value: Option<String>,
    /// Trim mode for values
    trim: TrimMode,
    /// Compression type
    compression: Compression,
    /// Maximum number of invalid rows before failure
    reject_limit: Option<u32>,
    /// File format (CSV or Parquet)
    format: ImportFormat,
}

impl ImportQuery {
    /// Create a new ImportQuery builder for the specified table.
    ///
    /// # Arguments
    /// * `table` - The name of the target table to import into
    ///
    pub fn new(table: &str) -> Self {
        Self {
            table: table.to_string(),
            schema: None,
            columns: None,
            address: None,
            public_key: None,
            file_name: "001.csv".to_string(),
            file_entries: None,
            column_separator: ',',
            column_delimiter: '"',
            row_separator: RowSeparator::default(),
            encoding: "UTF-8".to_string(),
            skip: 0,
            null_value: None,
            trim: TrimMode::default(),
            compression: Compression::default(),
            reject_limit: None,
            format: ImportFormat::default(),
        }
    }

    /// Set the target schema for the import.
    ///
    /// # Arguments
    /// * `schema` - The schema name
    pub fn schema(mut self, schema: &str) -> Self {
        self.schema = Some(schema.to_string());
        self
    }

    /// Set the columns to import into.
    ///
    /// If not specified, all columns in the table will be used.
    ///
    /// # Arguments
    /// * `cols` - List of column names
    pub fn columns(mut self, cols: Vec<&str>) -> Self {
        self.columns = Some(cols.into_iter().map(String::from).collect());
        self
    }

    /// Set the HTTP address to import from.
    ///
    /// # Arguments
    /// * `addr` - HTTP address in "host:port" format
    pub fn at_address(mut self, addr: &str) -> Self {
        self.address = Some(addr.to_string());
        self
    }

    /// Set the public key fingerprint for TLS verification.
    ///
    /// When set, HTTPS will be used and the PUBLIC KEY clause will be added.
    ///
    /// # Arguments
    /// * `fingerprint` - SHA-256 fingerprint of the server's public key
    pub fn with_public_key(mut self, fingerprint: &str) -> Self {
        self.public_key = Some(fingerprint.to_string());
        self
    }

    /// Set the file name for the import.
    ///
    /// # Arguments
    /// * `name` - File name (default "001.csv")
    pub fn file_name(mut self, name: &str) -> Self {
        self.file_name = name.to_string();
        self
    }

    /// Set the column separator character.
    ///
    /// # Arguments
    /// * `sep` - Separator character (default ',')
    pub fn column_separator(mut self, sep: char) -> Self {
        self.column_separator = sep;
        self
    }

    /// Set the column delimiter character for quoting.
    ///
    /// # Arguments
    /// * `delim` - Delimiter character (default '"')
    pub fn column_delimiter(mut self, delim: char) -> Self {
        self.column_delimiter = delim;
        self
    }

    /// Set the row separator.
    ///
    /// # Arguments
    /// * `sep` - Row separator (default LF)
    pub fn row_separator(mut self, sep: RowSeparator) -> Self {
        self.row_separator = sep;
        self
    }

    /// Set the character encoding.
    ///
    /// # Arguments
    /// * `enc` - Encoding name (default "UTF-8")
    pub fn encoding(mut self, enc: &str) -> Self {
        self.encoding = enc.to_string();
        self
    }

    /// Set the number of header rows to skip.
    ///
    /// # Arguments
    /// * `rows` - Number of rows to skip (default 0)
    pub fn skip(mut self, rows: u32) -> Self {
        self.skip = rows;
        self
    }

    /// Set a custom NULL value representation.
    ///
    /// # Arguments
    /// * `val` - String representing NULL values in the CSV
    pub fn null_value(mut self, val: &str) -> Self {
        self.null_value = Some(val.to_string());
        self
    }

    /// Set the trim mode for imported values.
    ///
    /// # Arguments
    /// * `trim` - Trim mode to apply
    pub fn trim(mut self, trim: TrimMode) -> Self {
        self.trim = trim;
        self
    }

    /// Set the compression type for the import file.
    ///
    /// This affects the file extension in the generated SQL.
    ///
    /// # Arguments
    /// * `compression` - Compression type
    pub fn compressed(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Set the reject limit for error handling.
    ///
    /// # Arguments
    /// * `limit` - Maximum number of invalid rows before the import fails
    pub fn reject_limit(mut self, limit: u32) -> Self {
        self.reject_limit = Some(limit);
        self
    }

    /// Set the file format for the import.
    ///
    /// When set to `ImportFormat::Parquet`, the generated SQL emits
    /// `FROM PARQUET`, the file extension becomes `.parquet` regardless of
    /// the configured compression, and the CSV format-options clause is
    /// omitted (the server reads the schema from the Parquet file).
    ///
    /// # Arguments
    /// * `format` - File format
    pub fn with_format(mut self, format: ImportFormat) -> Self {
        self.format = format;
        self
    }

    /// Set multiple file entries for parallel import.
    ///
    /// This method enables multi-file IMPORT statements where each file
    /// has its own AT/FILE clause with a unique internal address.
    ///
    /// # Arguments
    /// * `entries` - Vector of file entries with addresses and file names
    ///
    /// # Note
    ///
    /// When using `with_files()`, the `at_address()` and `file_name()` methods
    /// are ignored and should not be called.
    pub fn with_files(mut self, entries: Vec<ImportFileEntry>) -> Self {
        self.file_entries = Some(entries);
        self
    }

    /// Build the complete IMPORT SQL statement.
    ///
    /// # Returns
    /// The generated IMPORT SQL statement as a string.
    ///
    pub fn build(&self) -> String {
        let mut sql = String::with_capacity(512);
        sql.push_str(&self.target_clause());
        sql.push_str(&self.source_clause());
        // Format options apply ONLY to CSV. For PARQUET the server reads the
        // schema from the file directly, so emitting any of these clauses
        // (ENCODING, COLUMN SEPARATOR, COLUMN DELIMITER, ROW SEPARATOR, SKIP,
        // NULL, TRIM, REJECT LIMIT) would either be rejected or ignored.
        if !self.is_parquet() {
            sql.push_str(&self.csv_option_clauses());
        }
        sql
    }

    /// `IMPORT INTO [schema.]table [(col, ...)]`
    fn target_clause(&self) -> String {
        let mut clause = String::from("IMPORT INTO ");
        if let Some(ref schema) = self.schema {
            clause.push_str(schema);
            clause.push('.');
        }
        clause.push_str(&self.table);

        if let Some(ref cols) = self.columns {
            clause.push_str(" (");
            clause.push_str(&cols.join(", "));
            clause.push(')');
        }
        clause
    }

    /// `FROM <FORMAT> AT '<url>' [PUBLIC KEY '...'] FILE '...'`, once per file.
    ///
    /// Multi-file mode keeps every `AT`/`FILE` pair on the `FROM` line so the
    /// server can read the files in parallel; single-file mode puts `FILE` on
    /// its own line. Only that layout differs between the two modes.
    fn source_clause(&self) -> String {
        let mut clause = String::from("\nFROM ");
        clause.push_str(if self.is_parquet() { "PARQUET" } else { "CSV" });

        match self.file_entries {
            Some(ref entries) => {
                for entry in entries {
                    clause.push_str(" AT ");
                    clause.push_str(&self.endpoint_of(&entry.address, entry.public_key.as_deref()));
                    clause.push_str(" FILE '");
                    clause.push_str(&self.file_name_for(&entry.file_name));
                    clause.push('\'');
                }
            }
            None => {
                clause.push_str(" AT ");
                clause.push_str(&self.endpoint_of(
                    self.address.as_deref().unwrap_or_default(),
                    self.public_key.as_deref(),
                ));
                clause.push_str("\nFILE '");
                clause.push_str(&self.file_name_for(&self.file_name));
                clause.push('\'');
            }
        }
        clause
    }

    /// Render one quoted source URL plus its optional `PUBLIC KEY` clause.
    ///
    /// The `;MaxConcurrentReads=1` option is appended INSIDE the quoted URL for
    /// native Parquet imports, mirroring the JDBC reference driver's
    /// on-the-wire shape.
    fn endpoint_of(&self, address: &str, public_key: Option<&str>) -> String {
        let url_suffix = if self.is_parquet() {
            ";MaxConcurrentReads=1"
        } else {
            ""
        };
        clauses::quoted_endpoint(address, url_suffix, public_key)
    }

    /// The CSV-only clauses, in the order Exasol expects them.
    fn csv_option_clauses(&self) -> String {
        let mut clause = clauses::CsvFraming {
            encoding: &self.encoding,
            column_separator: self.column_separator,
            column_delimiter: self.column_delimiter,
            row_separator: self.row_separator.to_sql(),
        }
        .to_sql();

        if self.skip > 0 {
            clause.push_str(&format!("\nSKIP = {}", self.skip));
        }
        clause.push_str(&clauses::null_clause(self.null_value.as_deref()));
        if let Some(trim_sql) = self.trim.to_sql() {
            clause.push_str(&format!("\nTRIM = '{}'", trim_sql));
        }
        if let Some(limit) = self.reject_limit {
            clause.push_str(&format!("\nREJECT LIMIT {}", limit));
        }
        clause
    }

    fn is_parquet(&self) -> bool {
        matches!(self.format, ImportFormat::Parquet)
    }

    /// Get a file name with the appropriate extension for the configured format.
    ///
    /// For `ImportFormat::Parquet`, the extension is always `.parquet` and
    /// any configured compression suffix is bypassed. For `ImportFormat::Csv`,
    /// the extension reflects the configured `Compression`.
    fn file_name_for(&self, base_name: &str) -> String {
        let base = strip_known_extensions(base_name);
        match self.format {
            ImportFormat::Parquet => format!("{}.parquet", base),
            ImportFormat::Csv => format!("{}{}", base, self.compression.extension()),
        }
    }
}

fn strip_known_extensions(name: &str) -> &str {
    let mut current = name;
    loop {
        let stripped = current
            .strip_suffix(".gz")
            .or_else(|| current.strip_suffix(".bz2"))
            .or_else(|| current.strip_suffix(".csv"))
            .or_else(|| current.strip_suffix(".parquet"));
        match stripped {
            Some(next) if next.len() < current.len() => current = next,
            _ => return current,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_import_statement() {
        let sql = ImportQuery::new("users")
            .at_address("192.168.1.1:8080")
            .build();

        assert!(sql.contains("IMPORT INTO users"));
        assert!(sql.contains("FROM CSV AT 'http://192.168.1.1:8080'"));
        assert!(sql.contains("FILE '001.csv'"));
        assert!(sql.contains("ENCODING = 'UTF-8'"));
        assert!(sql.contains("COLUMN SEPARATOR = ','"));
        assert!(sql.contains("COLUMN DELIMITER = '\"'"));
        assert!(sql.contains("ROW SEPARATOR = 'LF'"));
    }

    #[test]
    fn test_import_with_schema() {
        let sql = ImportQuery::new("users")
            .schema("myschema")
            .at_address("192.168.1.1:8080")
            .build();

        assert!(sql.contains("IMPORT INTO myschema.users"));
    }

    #[test]
    fn test_import_with_columns() {
        let sql = ImportQuery::new("users")
            .columns(vec!["id", "name", "email"])
            .at_address("192.168.1.1:8080")
            .build();

        assert!(sql.contains("IMPORT INTO users (id, name, email)"));
    }

    #[test]
    fn test_import_with_all_format_options() {
        let sql = ImportQuery::new("data")
            .at_address("10.0.0.1:9000")
            .column_separator(';')
            .column_delimiter('\'')
            .row_separator(RowSeparator::CRLF)
            .encoding("ISO-8859-1")
            .skip(2)
            .null_value("NULL")
            .trim(TrimMode::Trim)
            .reject_limit(100)
            .build();

        assert!(sql.contains("COLUMN SEPARATOR = ';'"));
        assert!(sql.contains("COLUMN DELIMITER = '''"));
        assert!(sql.contains("ROW SEPARATOR = 'CRLF'"));
        assert!(sql.contains("ENCODING = 'ISO-8859-1'"));
        assert!(sql.contains("SKIP = 2"));
        assert!(sql.contains("NULL = 'NULL'"));
        assert!(sql.contains("TRIM = 'TRIM'"));
        assert!(sql.contains("REJECT LIMIT 100"));
    }

    #[test]
    fn test_import_with_use_tls() {
        let fingerprint = "SHA256:abc123def456";
        let sql = ImportQuery::new("secure_data")
            .at_address("192.168.1.1:8443")
            .with_public_key(fingerprint)
            .build();

        assert!(sql.contains("FROM CSV AT 'https://192.168.1.1:8443'"));
        assert!(sql.contains(&format!("PUBLIC KEY '{}'", fingerprint)));
    }

    #[test]
    fn test_import_with_gzip_compression() {
        let sql = ImportQuery::new("compressed_data")
            .at_address("192.168.1.1:8080")
            .compressed(Compression::Gzip)
            .build();

        assert!(sql.contains("FILE '001.csv.gz'"));
    }

    #[test]
    fn test_import_with_bzip2_compression() {
        let sql = ImportQuery::new("compressed_data")
            .at_address("192.168.1.1:8080")
            .compressed(Compression::Bzip2)
            .build();

        assert!(sql.contains("FILE '001.csv.bz2'"));
    }

    #[test]
    fn test_import_custom_file_name() {
        let sql = ImportQuery::new("data")
            .at_address("192.168.1.1:8080")
            .file_name("custom_file")
            .build();

        assert!(sql.contains("FILE 'custom_file.csv'"));
    }

    #[test]
    fn test_import_custom_file_name_with_compression() {
        let sql = ImportQuery::new("data")
            .at_address("192.168.1.1:8080")
            .file_name("custom_file")
            .compressed(Compression::Gzip)
            .build();

        assert!(sql.contains("FILE 'custom_file.csv.gz'"));
    }

    #[test]
    fn test_row_separator_to_sql() {
        assert_eq!(RowSeparator::LF.to_sql(), "LF");
        assert_eq!(RowSeparator::CR.to_sql(), "CR");
        assert_eq!(RowSeparator::CRLF.to_sql(), "CRLF");
    }

    #[test]
    fn test_compression_extension() {
        assert_eq!(Compression::None.extension(), ".csv");
        assert_eq!(Compression::Gzip.extension(), ".csv.gz");
        assert_eq!(Compression::Bzip2.extension(), ".csv.bz2");
    }

    #[test]
    fn test_trim_mode_to_sql() {
        assert_eq!(TrimMode::None.to_sql(), None);
        assert_eq!(TrimMode::LTrim.to_sql(), Some("LTRIM"));
        assert_eq!(TrimMode::RTrim.to_sql(), Some("RTRIM"));
        assert_eq!(TrimMode::Trim.to_sql(), Some("TRIM"));
    }

    #[test]
    fn test_defaults() {
        assert_eq!(RowSeparator::default(), RowSeparator::LF);
        assert_eq!(Compression::default(), Compression::None);
        assert_eq!(TrimMode::default(), TrimMode::None);
    }

    #[test]
    fn test_import_file_entry() {
        let entry = ImportFileEntry::new(
            "10.0.0.5:8563".to_string(),
            "001.csv".to_string(),
            Some("sha256//abc123".to_string()),
        );

        assert_eq!(entry.address, "10.0.0.5:8563");
        assert_eq!(entry.file_name, "001.csv");
        assert_eq!(entry.public_key, Some("sha256//abc123".to_string()));
    }

    #[test]
    fn test_multi_file_import_basic() {
        let entries = vec![
            ImportFileEntry::new("10.0.0.5:8563".to_string(), "001.csv".to_string(), None),
            ImportFileEntry::new("10.0.0.6:8564".to_string(), "002.csv".to_string(), None),
        ];

        let sql = ImportQuery::new("my_table").with_files(entries).build();

        assert!(sql.contains("IMPORT INTO my_table"));
        assert!(sql.contains("FROM CSV"));
        assert!(sql.contains("AT 'http://10.0.0.5:8563' FILE '001.csv'"));
        assert!(sql.contains("AT 'http://10.0.0.6:8564' FILE '002.csv'"));
    }

    #[test]
    fn test_multi_file_import_with_tls() {
        let entries = vec![
            ImportFileEntry::new(
                "10.0.0.5:8563".to_string(),
                "001.csv".to_string(),
                Some("sha256//fingerprint1".to_string()),
            ),
            ImportFileEntry::new(
                "10.0.0.6:8564".to_string(),
                "002.csv".to_string(),
                Some("sha256//fingerprint2".to_string()),
            ),
        ];

        let sql = ImportQuery::new("secure_table").with_files(entries).build();

        assert!(sql.contains("AT 'https://10.0.0.5:8563' PUBLIC KEY 'sha256//fingerprint1'"));
        assert!(sql.contains("AT 'https://10.0.0.6:8564' PUBLIC KEY 'sha256//fingerprint2'"));
    }

    #[test]
    fn test_multi_file_import_with_compression() {
        let entries = vec![
            ImportFileEntry::new("10.0.0.5:8563".to_string(), "001.csv".to_string(), None),
            ImportFileEntry::new("10.0.0.6:8564".to_string(), "002.csv".to_string(), None),
        ];

        let sql = ImportQuery::new("compressed_table")
            .with_files(entries)
            .compressed(Compression::Gzip)
            .build();

        assert!(sql.contains("FILE '001.csv.gz'"));
        assert!(sql.contains("FILE '002.csv.gz'"));
    }

    #[test]
    fn test_multi_file_import_three_files() {
        let entries = vec![
            ImportFileEntry::new("10.0.0.5:8563".to_string(), "001.csv".to_string(), None),
            ImportFileEntry::new("10.0.0.6:8564".to_string(), "002.csv".to_string(), None),
            ImportFileEntry::new("10.0.0.7:8565".to_string(), "003.csv".to_string(), None),
        ];

        let sql = ImportQuery::new("data").with_files(entries).build();

        assert!(sql.contains("AT 'http://10.0.0.5:8563' FILE '001.csv'"));
        assert!(sql.contains("AT 'http://10.0.0.6:8564' FILE '002.csv'"));
        assert!(sql.contains("AT 'http://10.0.0.7:8565' FILE '003.csv'"));
    }

    #[test]
    fn test_multi_file_import_with_schema_and_columns() {
        let entries = vec![
            ImportFileEntry::new("10.0.0.5:8563".to_string(), "001.csv".to_string(), None),
            ImportFileEntry::new("10.0.0.6:8564".to_string(), "002.csv".to_string(), None),
        ];

        let sql = ImportQuery::new("my_table")
            .schema("my_schema")
            .columns(vec!["id", "name", "value"])
            .with_files(entries)
            .build();

        assert!(sql.contains("IMPORT INTO my_schema.my_table (id, name, value)"));
    }

    #[test]
    fn test_multi_file_import_with_all_options() {
        let entries = vec![
            ImportFileEntry::new("10.0.0.5:8563".to_string(), "001.csv".to_string(), None),
            ImportFileEntry::new("10.0.0.6:8564".to_string(), "002.csv".to_string(), None),
        ];

        let sql = ImportQuery::new("data")
            .with_files(entries)
            .encoding("ISO-8859-1")
            .column_separator(';')
            .skip(1)
            .null_value("NULL")
            .reject_limit(100)
            .build();

        assert!(sql.contains("ENCODING = 'ISO-8859-1'"));
        assert!(sql.contains("COLUMN SEPARATOR = ';'"));
        assert!(sql.contains("SKIP = 1"));
        assert!(sql.contains("NULL = 'NULL'"));
        assert!(sql.contains("REJECT LIMIT 100"));
    }

    #[test]
    fn test_import_no_skip_when_zero() {
        let sql = ImportQuery::new("data")
            .at_address("192.168.1.1:8080")
            .build();

        // SKIP should not appear when it's 0
        assert!(!sql.contains("SKIP"));
    }

    #[test]
    fn test_import_skip_header_row() {
        let sql = ImportQuery::new("data")
            .at_address("192.168.1.1:8080")
            .skip(1)
            .build();

        assert!(sql.contains("SKIP = 1"));
    }

    #[test]
    fn test_complete_import_statement_format() {
        let sql = ImportQuery::new("employees")
            .schema("hr")
            .columns(vec!["id", "first_name", "last_name", "department"])
            .at_address("10.20.30.40:8080")
            .with_public_key("SHA256:fingerprint123")
            .skip(1)
            .reject_limit(10)
            .build();

        // Verify the complete structure
        let expected_parts = [
            "IMPORT INTO hr.employees (id, first_name, last_name, department)",
            "FROM CSV AT 'https://10.20.30.40:8080' PUBLIC KEY 'SHA256:fingerprint123'",
            "FILE '001.csv'",
            "ENCODING = 'UTF-8'",
            "COLUMN SEPARATOR = ','",
            "COLUMN DELIMITER = '\"'",
            "ROW SEPARATOR = 'LF'",
            "SKIP = 1",
            "REJECT LIMIT 10",
        ];

        for part in expected_parts {
            assert!(sql.contains(part), "SQL should contain: {}", part);
        }
    }

    #[test]
    fn test_import_query_format_parquet_single_file() {
        let sql = ImportQuery::new("my_table")
            .at_address("10.0.0.5:8563")
            .with_format(ImportFormat::Parquet)
            .build();
        assert!(sql.contains("FROM PARQUET AT 'http://10.0.0.5:8563;MaxConcurrentReads=1'"));
        assert!(sql.contains("FILE '001.parquet'"));
        assert!(!sql.contains("ENCODING"));
    }

    #[test]
    fn test_import_query_format_parquet_multi_file() {
        let entries = vec![
            ImportFileEntry::new("10.0.0.5:8563".to_string(), "001.parquet".to_string(), None),
            ImportFileEntry::new("10.0.0.6:8564".to_string(), "002.parquet".to_string(), None),
        ];
        let sql = ImportQuery::new("my_table")
            .with_files(entries)
            .with_format(ImportFormat::Parquet)
            .build();
        assert!(sql.contains("FROM PARQUET"));
        assert!(sql.contains("AT 'http://10.0.0.5:8563;MaxConcurrentReads=1'"));
        assert!(sql.contains("AT 'http://10.0.0.6:8564;MaxConcurrentReads=1'"));
        assert!(sql.contains("FILE '001.parquet'"));
        assert!(sql.contains("FILE '002.parquet'"));
        assert!(!sql.contains("MULTIPLE LOCAL FILES"));
    }

    #[test]
    fn test_import_query_format_parquet_with_tls_emits_public_key() {
        let sql = ImportQuery::new("t")
            .at_address("10.0.0.5:8563")
            .with_public_key("sha256//abc")
            .with_format(ImportFormat::Parquet)
            .build();
        assert!(sql
            .contains("AT 'https://10.0.0.5:8563;MaxConcurrentReads=1' PUBLIC KEY 'sha256//abc'"));
    }

    #[test]
    fn test_import_query_format_parquet_omits_csv_format_options() {
        let sql = ImportQuery::new("t")
            .at_address("addr:1234")
            .with_format(ImportFormat::Parquet)
            .skip(2)
            .null_value("NULL")
            .trim(TrimMode::Trim)
            .reject_limit(100)
            .build();
        for keyword in &[
            "ENCODING",
            "COLUMN SEPARATOR",
            "COLUMN DELIMITER",
            "ROW SEPARATOR",
            "SKIP",
            "NULL",
            "TRIM",
            "REJECT LIMIT",
        ] {
            assert!(
                !sql.contains(keyword),
                "SQL should NOT contain {} for Parquet format",
                keyword
            );
        }
    }

    #[test]
    fn test_import_query_parquet_ignores_compression() {
        let sql = ImportQuery::new("t")
            .at_address("addr:1234")
            .with_format(ImportFormat::Parquet)
            .compressed(Compression::Gzip)
            .build();
        assert!(sql.contains("FILE '001.parquet'"));
        assert!(!sql.contains(".gz"));
    }

    #[test]
    fn test_import_query_format_csv_unchanged() {
        let sql = ImportQuery::new("t").at_address("192.168.1.1:8080").build();
        assert!(sql.contains("FROM CSV AT 'http://192.168.1.1:8080'"));
        assert!(sql.contains("ENCODING = 'UTF-8'"));
        assert!(sql.contains("COLUMN SEPARATOR = ','"));
    }

    // =========================================================================
    // Whole-statement characterization: exact text, exact clause order
    // =========================================================================

    #[test]
    fn test_build_renders_the_full_default_csv_statement_verbatim() {
        let sql = ImportQuery::new("users")
            .at_address("192.168.1.1:8080")
            .build();

        assert_eq!(
            sql,
            "IMPORT INTO users\n\
             FROM CSV AT 'http://192.168.1.1:8080'\n\
             FILE '001.csv'\n\
             ENCODING = 'UTF-8'\n\
             COLUMN SEPARATOR = ','\n\
             COLUMN DELIMITER = '\"'\n\
             ROW SEPARATOR = 'LF'"
        );
    }

    #[test]
    fn test_build_renders_every_csv_option_in_order() {
        let sql = ImportQuery::new("data")
            .schema("hr")
            .columns(vec!["a", "b"])
            .at_address("10.0.0.1:9000")
            .with_public_key("SHA256:fp")
            .column_separator(';')
            .column_delimiter('|')
            .row_separator(RowSeparator::CRLF)
            .encoding("ISO-8859-1")
            .skip(2)
            .null_value("\\N")
            .trim(TrimMode::Trim)
            .reject_limit(100)
            .compressed(Compression::Gzip)
            .build();

        assert_eq!(
            sql,
            "IMPORT INTO hr.data (a, b)\n\
             FROM CSV AT 'https://10.0.0.1:9000' PUBLIC KEY 'SHA256:fp'\n\
             FILE '001.csv.gz'\n\
             ENCODING = 'ISO-8859-1'\n\
             COLUMN SEPARATOR = ';'\n\
             COLUMN DELIMITER = '|'\n\
             ROW SEPARATOR = 'CRLF'\n\
             SKIP = 2\n\
             NULL = '\\N'\n\
             TRIM = 'TRIM'\n\
             REJECT LIMIT 100"
        );
    }

    #[test]
    fn test_build_renders_multi_file_entries_on_one_from_line() {
        let entries = vec![
            ImportFileEntry::new("10.0.0.5:8563".to_string(), "001.csv".to_string(), None),
            ImportFileEntry::new(
                "10.0.0.6:8564".to_string(),
                "002.csv".to_string(),
                Some("sha256//fp2".to_string()),
            ),
        ];

        let sql = ImportQuery::new("my_table").with_files(entries).build();

        assert_eq!(
            sql,
            "IMPORT INTO my_table\n\
             FROM CSV AT 'http://10.0.0.5:8563' FILE '001.csv' \
             AT 'https://10.0.0.6:8564' PUBLIC KEY 'sha256//fp2' FILE '002.csv'\n\
             ENCODING = 'UTF-8'\n\
             COLUMN SEPARATOR = ','\n\
             COLUMN DELIMITER = '\"'\n\
             ROW SEPARATOR = 'LF'"
        );
    }

    #[test]
    fn test_build_renders_the_full_parquet_statement_verbatim() {
        let sql = ImportQuery::new("my_table")
            .at_address("10.0.0.5:8563")
            .with_format(ImportFormat::Parquet)
            .build();

        assert_eq!(
            sql,
            "IMPORT INTO my_table\n\
             FROM PARQUET AT 'http://10.0.0.5:8563;MaxConcurrentReads=1'\n\
             FILE '001.parquet'"
        );
    }

    #[test]
    fn test_build_without_address_still_renders_a_scheme_only_url() {
        let sql = ImportQuery::new("t").build();

        assert!(sql.contains("FROM CSV AT 'http://'"), "got: {}", sql);
    }

    #[test]
    fn test_strip_known_extensions_unwinds_stacked_suffixes() {
        assert_eq!(strip_known_extensions("001"), "001");
        assert_eq!(strip_known_extensions("001.csv"), "001");
        assert_eq!(strip_known_extensions("001.csv.gz"), "001");
        assert_eq!(strip_known_extensions("001.csv.bz2"), "001");
        assert_eq!(strip_known_extensions("001.parquet"), "001");
        assert_eq!(strip_known_extensions("archive.tar"), "archive.tar");
    }
}
