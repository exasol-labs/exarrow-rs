//! Export query builder for generating EXPORT SQL statements.
//!
//! This module provides the `ExportQuery` builder for constructing Exasol EXPORT statements
//! that export data to an HTTP endpoint via CSV format.
//!
//! # Example
//!

/// Row separator options for CSV export.
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
    /// Get the SQL representation of the row separator.
    pub fn as_sql(&self) -> &'static str {
        match self {
            RowSeparator::LF => "LF",
            RowSeparator::CR => "CR",
            RowSeparator::CRLF => "CRLF",
        }
    }
}

/// Compression options for export files.
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
    /// Get the file extension for the compression type.
    pub fn extension(&self) -> &'static str {
        match self {
            Compression::None => "",
            Compression::Gzip => ".gz",
            Compression::Bzip2 => ".bz2",
        }
    }
}

/// Delimit mode for CSV export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DelimitMode {
    /// Automatically detect when delimiters are needed
    #[default]
    Auto,
    /// Always use delimiters
    Always,
    /// Never use delimiters
    Never,
}

impl DelimitMode {
    /// Get the SQL representation of the delimit mode.
    pub fn as_sql(&self) -> &'static str {
        match self {
            DelimitMode::Auto => "AUTO",
            DelimitMode::Always => "ALWAYS",
            DelimitMode::Never => "NEVER",
        }
    }
}

/// Source for the export: either a table or a query.
#[derive(Debug, Clone)]
pub enum ExportSource {
    /// Export from a table.
    Table {
        /// Schema name (optional).
        schema: Option<String>,
        /// Table name.
        name: String,
        /// Columns to export (optional, empty means all columns).
        columns: Vec<String>,
    },
    /// Export from a query.
    Query {
        /// SQL query to export results from.
        sql: String,
    },
}

/// Builder for constructing EXPORT SQL statements.
///
/// Exasol EXPORT statements transfer data from tables or queries to external destinations
/// via HTTP transport using CSV format.
#[derive(Debug, Clone)]
pub struct ExportQuery {
    /// Source of the export (table or query).
    source: ExportSource,
    /// HTTP address for the export destination.
    address: String,
    /// Public key fingerprint for TLS (optional).
    public_key: Option<String>,
    /// File name for the export.
    file_name: String,
    /// Column separator character.
    column_separator: char,
    /// Column delimiter character.
    column_delimiter: char,
    /// Row separator.
    row_separator: RowSeparator,
    /// Character encoding.
    encoding: String,
    /// Custom NULL representation (optional).
    null_value: Option<String>,
    /// Delimit mode.
    delimit_mode: DelimitMode,
    /// Compression type.
    compression: Compression,
    /// Whether to include column names in the output.
    with_column_names: bool,
}

impl ExportQuery {
    /// Create an export query from a table.
    ///
    /// # Arguments
    ///
    /// * `table` - The name of the table to export from.
    ///
    /// # Example
    ///
    pub fn from_table(table: &str) -> Self {
        Self {
            source: ExportSource::Table {
                schema: None,
                name: table.to_string(),
                columns: Vec::new(),
            },
            address: String::new(),
            public_key: None,
            file_name: "001.csv".to_string(),
            column_separator: ',',
            column_delimiter: '"',
            row_separator: RowSeparator::default(),
            encoding: "UTF-8".to_string(),
            null_value: None,
            delimit_mode: DelimitMode::default(),
            compression: Compression::default(),
            with_column_names: false,
        }
    }

    /// Create an export query from a SQL query.
    ///
    /// # Arguments
    ///
    /// * `sql` - The SQL query whose results to export.
    ///
    /// # Example
    ///
    pub fn from_query(sql: &str) -> Self {
        Self {
            source: ExportSource::Query {
                sql: sql.to_string(),
            },
            address: String::new(),
            public_key: None,
            file_name: "001.csv".to_string(),
            column_separator: ',',
            column_delimiter: '"',
            row_separator: RowSeparator::default(),
            encoding: "UTF-8".to_string(),
            null_value: None,
            delimit_mode: DelimitMode::default(),
            compression: Compression::default(),
            with_column_names: false,
        }
    }

    /// Set the schema for a table export.
    ///
    /// This method only has an effect when exporting from a table, not from a query.
    ///
    /// # Arguments
    ///
    /// * `schema` - The schema name.
    pub fn schema(mut self, schema: &str) -> Self {
        if let ExportSource::Table {
            schema: ref mut s, ..
        } = self.source
        {
            *s = Some(schema.to_string());
        }
        self
    }

    /// Set the columns to export from a table.
    ///
    /// This method only has an effect when exporting from a table, not from a query.
    ///
    /// # Arguments
    ///
    /// * `cols` - The column names to export.
    pub fn columns(mut self, cols: Vec<&str>) -> Self {
        if let ExportSource::Table {
            columns: ref mut c, ..
        } = self.source
        {
            *c = cols.into_iter().map(String::from).collect();
        }
        self
    }

    /// Set the HTTP address for the export destination.
    ///
    /// The protocol (http:// or https://) will be determined automatically
    /// based on whether a public key is set.
    ///
    /// # Arguments
    ///
    /// * `addr` - The address in the format "host:port".
    pub fn at_address(mut self, addr: &str) -> Self {
        self.address = addr.to_string();
        self
    }

    /// Set the public key fingerprint for TLS encryption.
    ///
    /// When set, the export will use HTTPS and include a PUBLIC KEY clause.
    ///
    /// # Arguments
    ///
    /// * `fingerprint` - The SHA-256 fingerprint of the public key.
    pub fn with_public_key(mut self, fingerprint: &str) -> Self {
        self.public_key = Some(fingerprint.to_string());
        self
    }

    /// Set the output file name.
    ///
    /// Default is "001.csv". The compression extension will be appended automatically
    /// if compression is enabled.
    ///
    /// # Arguments
    ///
    /// * `name` - The file name.
    pub fn file_name(mut self, name: &str) -> Self {
        self.file_name = name.to_string();
        self
    }

    /// Set the column separator character.
    ///
    /// Default is ','.
    ///
    /// # Arguments
    ///
    /// * `sep` - The separator character.
    pub fn column_separator(mut self, sep: char) -> Self {
        self.column_separator = sep;
        self
    }

    /// Set the column delimiter character.
    ///
    /// Default is '"'.
    ///
    /// # Arguments
    ///
    /// * `delim` - The delimiter character.
    pub fn column_delimiter(mut self, delim: char) -> Self {
        self.column_delimiter = delim;
        self
    }

    /// Set the row separator.
    ///
    /// Default is LF (Unix-style line endings).
    ///
    /// # Arguments
    ///
    /// * `sep` - The row separator.
    pub fn row_separator(mut self, sep: RowSeparator) -> Self {
        self.row_separator = sep;
        self
    }

    /// Set the character encoding.
    ///
    /// Default is "UTF-8".
    ///
    /// # Arguments
    ///
    /// * `enc` - The encoding name.
    pub fn encoding(mut self, enc: &str) -> Self {
        self.encoding = enc.to_string();
        self
    }

    /// Set a custom NULL value representation.
    ///
    /// By default, NULL values are exported as empty strings.
    ///
    /// # Arguments
    ///
    /// * `val` - The string to use for NULL values.
    pub fn null_value(mut self, val: &str) -> Self {
        self.null_value = Some(val.to_string());
        self
    }

    /// Set the delimit mode.
    ///
    /// Default is Auto.
    ///
    /// # Arguments
    ///
    /// * `mode` - The delimit mode.
    pub fn delimit_mode(mut self, mode: DelimitMode) -> Self {
        self.delimit_mode = mode;
        self
    }

    /// Set the compression type.
    ///
    /// The appropriate file extension will be added automatically.
    ///
    /// # Arguments
    ///
    /// * `compression` - The compression type.
    pub fn compressed(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Set whether to include column names in the output.
    ///
    /// Default is false.
    ///
    /// # Arguments
    ///
    /// * `include` - Whether to include column names.
    pub fn with_column_names(mut self, include: bool) -> Self {
        self.with_column_names = include;
        self
    }

    /// Build the EXPORT SQL statement.
    ///
    /// # Returns
    ///
    /// The complete EXPORT SQL statement as a string.
    pub fn build(&self) -> String {
        let mut sql = String::new();

        // Build the source part
        match &self.source {
            ExportSource::Table {
                schema,
                name,
                columns,
            } => {
                sql.push_str("EXPORT ");
                if let Some(s) = schema {
                    sql.push_str(s);
                    sql.push('.');
                }
                sql.push_str(name);
                if !columns.is_empty() {
                    sql.push_str(" (");
                    sql.push_str(&columns.join(", "));
                    sql.push(')');
                }
            }
            ExportSource::Query { sql: query } => {
                sql.push_str("EXPORT (");
                sql.push_str(query);
                sql.push(')');
            }
        }

        // Build the destination part
        sql.push_str("\nINTO CSV AT '");

        // Determine protocol based on public key
        if self.public_key.is_some() {
            sql.push_str("https://");
        } else {
            sql.push_str("http://");
        }
        sql.push_str(&self.address);
        sql.push('\'');

        // Add PUBLIC KEY clause if set
        if let Some(ref fingerprint) = self.public_key {
            sql.push_str(" PUBLIC KEY '");
            sql.push_str(fingerprint);
            sql.push('\'');
        }

        // Add file name with compression extension
        sql.push_str("\nFILE '");
        sql.push_str(&self.file_name);
        sql.push_str(self.compression.extension());
        sql.push('\'');

        // Add format options
        sql.push_str("\nENCODING = '");
        sql.push_str(&self.encoding);
        sql.push('\'');

        sql.push_str("\nCOLUMN SEPARATOR = '");
        sql.push(self.column_separator);
        sql.push('\'');

        sql.push_str("\nCOLUMN DELIMITER = '");
        sql.push(self.column_delimiter);
        sql.push('\'');

        sql.push_str("\nROW SEPARATOR = '");
        sql.push_str(self.row_separator.as_sql());
        sql.push('\'');

        // Add NULL value if set
        if let Some(ref null_val) = self.null_value {
            sql.push_str("\nNULL = '");
            sql.push_str(null_val);
            sql.push('\'');
        }

        // Add WITH COLUMN NAMES if enabled
        if self.with_column_names {
            sql.push_str("\nWITH COLUMN NAMES");
        }

        // Add DELIMIT mode
        sql.push_str("\nDELIMIT = ");
        sql.push_str(self.delimit_mode.as_sql());

        sql
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test basic export statement generation from table
    #[test]
    fn test_export_from_table_basic() {
        let sql = ExportQuery::from_table("users")
            .at_address("192.168.1.100:8080")
            .build();

        assert!(sql.starts_with("EXPORT users"));
        assert!(sql.contains("INTO CSV AT 'http://192.168.1.100:8080'"));
        assert!(sql.contains("FILE '001.csv'"));
        assert!(sql.contains("ENCODING = 'UTF-8'"));
        assert!(sql.contains("COLUMN SEPARATOR = ','"));
        assert!(sql.contains("COLUMN DELIMITER = '\"'"));
        assert!(sql.contains("ROW SEPARATOR = 'LF'"));
        assert!(sql.contains("DELIMIT = AUTO"));
    }

    #[test]
    fn test_export_from_table_with_schema() {
        let sql = ExportQuery::from_table("users")
            .schema("my_schema")
            .at_address("192.168.1.100:8080")
            .build();

        assert!(sql.starts_with("EXPORT my_schema.users"));
    }

    #[test]
    fn test_export_from_table_with_columns() {
        let sql = ExportQuery::from_table("users")
            .columns(vec!["id", "name", "email"])
            .at_address("192.168.1.100:8080")
            .build();

        assert!(sql.starts_with("EXPORT users (id, name, email)"));
    }

    #[test]
    fn test_export_from_table_with_schema_and_columns() {
        let sql = ExportQuery::from_table("users")
            .schema("my_schema")
            .columns(vec!["id", "name"])
            .at_address("192.168.1.100:8080")
            .build();

        assert!(sql.starts_with("EXPORT my_schema.users (id, name)"));
    }

    // Test export from query
    #[test]
    fn test_export_from_query() {
        let sql = ExportQuery::from_query("SELECT * FROM users WHERE active = true")
            .at_address("192.168.1.100:8080")
            .build();

        assert!(sql.starts_with("EXPORT (SELECT * FROM users WHERE active = true)"));
        assert!(sql.contains("INTO CSV AT 'http://192.168.1.100:8080'"));
    }

    #[test]
    fn test_export_from_query_complex() {
        let sql = ExportQuery::from_query(
            "SELECT u.id, u.name, COUNT(o.id) FROM users u JOIN orders o ON u.id = o.user_id GROUP BY u.id, u.name",
        )
        .at_address("192.168.1.100:8080")
        .build();

        assert!(sql.contains("EXPORT (SELECT u.id, u.name, COUNT(o.id) FROM users u JOIN orders o ON u.id = o.user_id GROUP BY u.id, u.name)"));
    }

    // Test all format options
    #[test]
    fn test_export_with_all_format_options() {
        let sql = ExportQuery::from_table("data")
            .at_address("192.168.1.100:8080")
            .column_separator(';')
            .column_delimiter('\'')
            .row_separator(RowSeparator::CRLF)
            .encoding("ISO-8859-1")
            .null_value("NULL")
            .delimit_mode(DelimitMode::Always)
            .build();

        assert!(sql.contains("COLUMN SEPARATOR = ';'"));
        assert!(sql.contains("COLUMN DELIMITER = '''"));
        assert!(sql.contains("ROW SEPARATOR = 'CRLF'"));
        assert!(sql.contains("ENCODING = 'ISO-8859-1'"));
        assert!(sql.contains("NULL = 'NULL'"));
        assert!(sql.contains("DELIMIT = ALWAYS"));
    }

    #[test]
    fn test_export_row_separator_cr() {
        let sql = ExportQuery::from_table("data")
            .at_address("192.168.1.100:8080")
            .row_separator(RowSeparator::CR)
            .build();

        assert!(sql.contains("ROW SEPARATOR = 'CR'"));
    }

    #[test]
    fn test_export_delimit_mode_never() {
        let sql = ExportQuery::from_table("data")
            .at_address("192.168.1.100:8080")
            .delimit_mode(DelimitMode::Never)
            .build();

        assert!(sql.contains("DELIMIT = NEVER"));
    }

    // Test with encryption (PUBLIC KEY)
    #[test]
    fn test_export_with_public_key() {
        let fingerprint = "AB:CD:EF:12:34:56:78:90:AB:CD:EF:12:34:56:78:90";
        let sql = ExportQuery::from_table("users")
            .at_address("192.168.1.100:8080")
            .with_public_key(fingerprint)
            .build();

        assert!(sql.contains("INTO CSV AT 'https://192.168.1.100:8080'"));
        assert!(sql.contains(&format!("PUBLIC KEY '{}'", fingerprint)));
    }

    #[test]
    fn test_export_without_public_key_uses_http() {
        let sql = ExportQuery::from_table("users")
            .at_address("192.168.1.100:8080")
            .build();

        assert!(sql.contains("INTO CSV AT 'http://192.168.1.100:8080'"));
        assert!(!sql.contains("PUBLIC KEY"));
    }

    // Test WITH COLUMN NAMES
    #[test]
    fn test_export_with_column_names() {
        let sql = ExportQuery::from_table("users")
            .at_address("192.168.1.100:8080")
            .with_column_names(true)
            .build();

        assert!(sql.contains("WITH COLUMN NAMES"));
    }

    #[test]
    fn test_export_without_column_names() {
        let sql = ExportQuery::from_table("users")
            .at_address("192.168.1.100:8080")
            .with_column_names(false)
            .build();

        assert!(!sql.contains("WITH COLUMN NAMES"));
    }

    // Test compression file extensions
    #[test]
    fn test_export_no_compression() {
        let sql = ExportQuery::from_table("users")
            .at_address("192.168.1.100:8080")
            .compressed(Compression::None)
            .build();

        assert!(sql.contains("FILE '001.csv'"));
        assert!(!sql.contains(".gz"));
        assert!(!sql.contains(".bz2"));
    }

    #[test]
    fn test_export_gzip_compression() {
        let sql = ExportQuery::from_table("users")
            .at_address("192.168.1.100:8080")
            .compressed(Compression::Gzip)
            .build();

        assert!(sql.contains("FILE '001.csv.gz'"));
    }

    #[test]
    fn test_export_bzip2_compression() {
        let sql = ExportQuery::from_table("users")
            .at_address("192.168.1.100:8080")
            .compressed(Compression::Bzip2)
            .build();

        assert!(sql.contains("FILE '001.csv.bz2'"));
    }

    #[test]
    fn test_export_custom_file_name_with_compression() {
        let sql = ExportQuery::from_table("users")
            .at_address("192.168.1.100:8080")
            .file_name("export_data.csv")
            .compressed(Compression::Gzip)
            .build();

        assert!(sql.contains("FILE 'export_data.csv.gz'"));
    }

    // Test complete export statement
    #[test]
    fn test_export_complete_statement() {
        let sql = ExportQuery::from_table("orders")
            .schema("sales")
            .columns(vec!["order_id", "customer_id", "total"])
            .at_address("10.0.0.1:3000")
            .with_public_key("SHA256:fingerprint123")
            .file_name("orders_export.csv")
            .column_separator('|')
            .column_delimiter('"')
            .row_separator(RowSeparator::LF)
            .encoding("UTF-8")
            .null_value("\\N")
            .with_column_names(true)
            .delimit_mode(DelimitMode::Always)
            .compressed(Compression::Gzip)
            .build();

        // Verify all parts of the statement
        assert!(sql.starts_with("EXPORT sales.orders (order_id, customer_id, total)"));
        assert!(sql.contains("INTO CSV AT 'https://10.0.0.1:3000'"));
        assert!(sql.contains("PUBLIC KEY 'SHA256:fingerprint123'"));
        assert!(sql.contains("FILE 'orders_export.csv.gz'"));
        assert!(sql.contains("ENCODING = 'UTF-8'"));
        assert!(sql.contains("COLUMN SEPARATOR = '|'"));
        assert!(sql.contains("COLUMN DELIMITER = '\"'"));
        assert!(sql.contains("ROW SEPARATOR = 'LF'"));
        assert!(sql.contains("NULL = '\\N'"));
        assert!(sql.contains("WITH COLUMN NAMES"));
        assert!(sql.contains("DELIMIT = ALWAYS"));
    }

    // Test enum helper methods
    #[test]
    fn test_row_separator_as_sql() {
        assert_eq!(RowSeparator::LF.as_sql(), "LF");
        assert_eq!(RowSeparator::CR.as_sql(), "CR");
        assert_eq!(RowSeparator::CRLF.as_sql(), "CRLF");
    }

    #[test]
    fn test_compression_extension() {
        assert_eq!(Compression::None.extension(), "");
        assert_eq!(Compression::Gzip.extension(), ".gz");
        assert_eq!(Compression::Bzip2.extension(), ".bz2");
    }

    #[test]
    fn test_delimit_mode_as_sql() {
        assert_eq!(DelimitMode::Auto.as_sql(), "AUTO");
        assert_eq!(DelimitMode::Always.as_sql(), "ALWAYS");
        assert_eq!(DelimitMode::Never.as_sql(), "NEVER");
    }

    // Test default values
    #[test]
    fn test_default_values() {
        assert_eq!(RowSeparator::default(), RowSeparator::LF);
        assert_eq!(Compression::default(), Compression::None);
        assert_eq!(DelimitMode::default(), DelimitMode::Auto);
    }

    // Test that schema/columns methods are no-op for query source
    #[test]
    fn test_schema_columns_ignored_for_query() {
        let sql = ExportQuery::from_query("SELECT 1")
            .schema("ignored_schema")
            .columns(vec!["ignored_col"])
            .at_address("192.168.1.100:8080")
            .build();

        assert!(sql.starts_with("EXPORT (SELECT 1)"));
        assert!(!sql.contains("ignored_schema"));
        assert!(!sql.contains("ignored_col"));
    }
}
