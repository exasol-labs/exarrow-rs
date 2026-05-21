//! SQL statement handling and execution.
//!
//! This module provides the `Statement` type as a pure data container for SQL queries
//! with parameter binding. Statement execution is handled by Connection.

use crate::error::QueryError;

/// Type of SQL statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementType {
    /// SELECT query
    Select,
    /// INSERT statement
    Insert,
    /// UPDATE statement
    Update,
    /// DELETE statement
    Delete,
    /// DDL statement (CREATE, ALTER, DROP)
    Ddl,
    /// Transaction control (BEGIN, COMMIT, ROLLBACK)
    Transaction,
    /// Unknown or other statement type
    Other,
}

impl StatementType {
    /// Detect statement type from SQL text.
    pub fn from_sql(sql: &str) -> Self {
        let trimmed = sql.trim_start().to_uppercase();

        if trimmed.starts_with("SELECT") || trimmed.starts_with("WITH") {
            Self::Select
        } else if trimmed.starts_with("INSERT") {
            Self::Insert
        } else if trimmed.starts_with("UPDATE") {
            Self::Update
        } else if trimmed.starts_with("DELETE") {
            Self::Delete
        } else if trimmed.starts_with("CREATE")
            || trimmed.starts_with("ALTER")
            || trimmed.starts_with("DROP")
            || trimmed.starts_with("TRUNCATE")
        {
            Self::Ddl
        } else if trimmed.starts_with("BEGIN")
            || trimmed.starts_with("COMMIT")
            || trimmed.starts_with("ROLLBACK")
        {
            Self::Transaction
        } else {
            Self::Other
        }
    }

    /// Check if this statement type returns a result set.
    pub fn returns_result_set(&self) -> bool {
        matches!(self, Self::Select)
    }

    /// Check if this statement type returns a row count.
    pub fn returns_row_count(&self) -> bool {
        matches!(self, Self::Insert | Self::Update | Self::Delete)
    }
}

/// Parameter value for prepared statements.
#[derive(Debug, Clone)]
pub enum Parameter {
    /// NULL value
    Null,
    /// Boolean value
    Boolean(bool),
    /// Integer value
    Integer(i64),
    /// Float value
    Float(f64),
    /// String value
    String(String),
    /// Binary data
    Binary(Vec<u8>),
}

impl Parameter {
    /// Convert parameter to SQL literal string.
    ///
    /// This is a basic implementation for Phase 1.
    /// In production, use proper prepared statement protocol.
    pub fn to_sql_literal(&self) -> Result<String, QueryError> {
        match self {
            Parameter::Null => Ok("NULL".to_string()),
            Parameter::Boolean(b) => Ok(if *b { "TRUE" } else { "FALSE" }.to_string()),
            Parameter::Integer(i) => Ok(i.to_string()),
            Parameter::Float(f) => {
                if f.is_nan() || f.is_infinite() {
                    Err(QueryError::ParameterBindingError {
                        index: 0,
                        message: "NaN and Infinity are not supported".to_string(),
                    })
                } else {
                    Ok(f.to_string())
                }
            }
            Parameter::String(s) => {
                // Additional check for suspicious patterns (before escaping)
                if Self::contains_sql_injection_pattern(s) {
                    return Err(QueryError::SqlInjectionDetected);
                }

                // Basic SQL injection prevention: escape single quotes
                let escaped = s.replace('\'', "''");

                Ok(format!("'{}'", escaped))
            }
            Parameter::Binary(b) => {
                // Convert binary to hex string
                Ok(format!("'{}'", hex::encode(b)))
            }
        }
    }

    /// Basic SQL injection detection.
    fn contains_sql_injection_pattern(s: &str) -> bool {
        let upper = s.to_uppercase();

        // Check for common SQL injection patterns
        let patterns = [
            "'; DROP",
            "'; DELETE",
            "'; UPDATE",
            "'; INSERT",
            "' OR '1'='1",
            "' OR 1=1",
            "' OR TRUE",
            "UNION SELECT",
            "EXEC(",
            "EXECUTE(",
        ];

        patterns.iter().any(|pattern| upper.contains(pattern))
    }
}

impl From<bool> for Parameter {
    fn from(value: bool) -> Self {
        Parameter::Boolean(value)
    }
}

impl From<i32> for Parameter {
    fn from(value: i32) -> Self {
        Parameter::Integer(value as i64)
    }
}

impl From<i64> for Parameter {
    fn from(value: i64) -> Self {
        Parameter::Integer(value)
    }
}

impl From<f64> for Parameter {
    fn from(value: f64) -> Self {
        Parameter::Float(value)
    }
}

impl From<String> for Parameter {
    fn from(value: String) -> Self {
        Parameter::String(value)
    }
}

impl From<&str> for Parameter {
    fn from(value: &str) -> Self {
        Parameter::String(value.to_string())
    }
}

impl From<Vec<u8>> for Parameter {
    fn from(value: Vec<u8>) -> Self {
        Parameter::Binary(value)
    }
}

/// Lexer state for placeholder scanning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanState {
    Normal,
    SingleQuoted,
    DoubleQuoted,
    LineComment,
    BlockComment,
}

/// Return byte offsets of every `?` that is a positional placeholder.
///
/// A `?` only counts when the scanner is in the `Normal` state — `?`
/// characters inside string literals (`'...'`), double-quoted identifiers
/// (`"..."`), line comments (`-- ...\n`) or block comments (`/* ... */`)
/// are ignored. Doubled quotes (`''` and `""`) inside the matching string
/// state are treated as escapes (stay in that state).
///
/// The scanner is byte-safe for UTF-8 input by driving off `char_indices()`
/// (multi-byte characters in literals are passed through transparently).
/// Unterminated literals and unterminated block comments do not panic — the
/// scanner simply stops emitting placeholders for the remainder of input.
fn scan_placeholders(sql: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut state = ScanState::Normal;
    let bytes = sql.as_bytes();
    let mut iter = sql.char_indices();

    while let Some((i, c)) = iter.next() {
        match state {
            ScanState::Normal => match c {
                '?' => positions.push(i),
                '\'' => state = ScanState::SingleQuoted,
                '"' => state = ScanState::DoubleQuoted,
                '-' if bytes.get(i + 1) == Some(&b'-') => {
                    iter.next();
                    state = ScanState::LineComment;
                }
                '/' if bytes.get(i + 1) == Some(&b'*') => {
                    iter.next();
                    state = ScanState::BlockComment;
                }
                _ => {}
            },
            ScanState::SingleQuoted => {
                if c == '\'' {
                    if bytes.get(i + 1) == Some(&b'\'') {
                        iter.next();
                    } else {
                        state = ScanState::Normal;
                    }
                }
            }
            ScanState::DoubleQuoted => {
                if c == '"' {
                    if bytes.get(i + 1) == Some(&b'"') {
                        iter.next();
                    } else {
                        state = ScanState::Normal;
                    }
                }
            }
            ScanState::LineComment => {
                if c == '\n' {
                    state = ScanState::Normal;
                }
            }
            ScanState::BlockComment => {
                if c == '*' && bytes.get(i + 1) == Some(&b'/') {
                    iter.next();
                    state = ScanState::Normal;
                }
            }
        }
    }

    positions
}

/// SQL statement as a pure data container.
///
/// Statement holds SQL text, parameters, timeout, and statement type.
/// Execution is performed by Connection, not by Statement itself.
///
/// # Example
///
pub struct Statement {
    /// SQL text (may contain parameter placeholders)
    sql: String,
    /// Bound parameters (indexed by position)
    parameters: Vec<Option<Parameter>>,
    /// Query timeout in milliseconds
    timeout_ms: u64,
    /// Statement type
    statement_type: StatementType,
}

impl Statement {
    /// Create a new statement.
    pub fn new(sql: impl Into<String>) -> Self {
        let sql = sql.into();
        let statement_type = StatementType::from_sql(&sql);

        Self {
            sql,
            parameters: Vec::new(),
            timeout_ms: 120_000, // 2 minutes default
            statement_type,
        }
    }

    /// Get the SQL text.
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Get the statement type.
    pub fn statement_type(&self) -> StatementType {
        self.statement_type
    }

    /// Get the timeout in milliseconds.
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// Set query timeout.
    pub fn set_timeout(&mut self, timeout_ms: u64) {
        self.timeout_ms = timeout_ms;
    }

    /// Bind a parameter at the given index.
    ///
    /// # Arguments
    /// * `index` - Parameter index (0-based)
    /// * `value` - Parameter value
    ///
    /// # Errors
    /// Returns `QueryError::ParameterBindingError` if binding fails.
    pub fn bind<T: Into<Parameter>>(&mut self, index: usize, value: T) -> Result<(), QueryError> {
        // Ensure parameters vector is large enough
        if index >= self.parameters.len() {
            self.parameters.resize(index + 1, None);
        }

        self.parameters[index] = Some(value.into());
        Ok(())
    }

    /// Bind multiple parameters.
    pub fn bind_all<T: Into<Parameter> + Clone>(&mut self, params: &[T]) -> Result<(), QueryError> {
        for (index, param) in params.iter().enumerate() {
            self.bind(index, param.clone())?;
        }
        Ok(())
    }

    /// Clear all bound parameters.
    pub fn clear_parameters(&mut self) {
        self.parameters.clear();
    }

    /// Get bound parameters.
    pub fn parameters(&self) -> &[Option<Parameter>] {
        &self.parameters
    }

    /// Build the final SQL with parameters substituted.
    ///
    /// Scans for real `?` placeholders (skipping those inside string literals
    /// and comments), then substitutes right-to-left so that earlier byte
    /// offsets stay valid after each replacement.
    pub fn build_sql(&self) -> Result<String, QueryError> {
        let positions = scan_placeholders(&self.sql);

        if positions.len() > self.parameters.len() {
            return Err(QueryError::ParameterBindingError {
                index: self.parameters.len(),
                message: "Not enough parameters bound".to_string(),
            });
        }

        let mut sql = self.sql.clone();

        for (param_index, &pos) in positions.iter().enumerate().rev() {
            let param = self.parameters[param_index].as_ref().ok_or_else(|| {
                QueryError::ParameterBindingError {
                    index: param_index,
                    message: "Parameter not bound".to_string(),
                }
            })?;

            let literal = param.to_sql_literal()?;
            sql.replace_range(pos..pos + 1, &literal);
        }

        Ok(sql)
    }
}

impl std::fmt::Debug for Statement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Statement")
            .field("sql", &self.sql)
            .field("statement_type", &self.statement_type)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

impl std::fmt::Display for Statement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Statement({})", self.sql)
    }
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    #[test]
    fn test_statement_type_detection() {
        assert_eq!(
            StatementType::from_sql("SELECT * FROM users"),
            StatementType::Select
        );
        assert_eq!(
            StatementType::from_sql("  select * from users"),
            StatementType::Select
        );
        assert_eq!(
            StatementType::from_sql("WITH cte AS (SELECT 1) SELECT * FROM cte"),
            StatementType::Select
        );
        assert_eq!(
            StatementType::from_sql("INSERT INTO users VALUES (1)"),
            StatementType::Insert
        );
        assert_eq!(
            StatementType::from_sql("UPDATE users SET name = 'John'"),
            StatementType::Update
        );
        assert_eq!(
            StatementType::from_sql("DELETE FROM users WHERE id = 1"),
            StatementType::Delete
        );
        assert_eq!(
            StatementType::from_sql("CREATE TABLE test (id INT)"),
            StatementType::Ddl
        );
        assert_eq!(
            StatementType::from_sql("DROP TABLE test"),
            StatementType::Ddl
        );
        assert_eq!(StatementType::from_sql("BEGIN"), StatementType::Transaction);
        assert_eq!(
            StatementType::from_sql("COMMIT"),
            StatementType::Transaction
        );
        assert_eq!(
            StatementType::from_sql("ROLLBACK"),
            StatementType::Transaction
        );
    }

    #[test]
    fn test_statement_type_returns_result_set() {
        assert!(StatementType::Select.returns_result_set());
        assert!(!StatementType::Insert.returns_result_set());
        assert!(!StatementType::Update.returns_result_set());
        assert!(!StatementType::Delete.returns_result_set());
    }

    #[test]
    fn test_parameter_to_sql_literal() {
        assert_eq!(Parameter::Null.to_sql_literal().unwrap(), "NULL");
        assert_eq!(Parameter::Boolean(true).to_sql_literal().unwrap(), "TRUE");
        assert_eq!(Parameter::Boolean(false).to_sql_literal().unwrap(), "FALSE");
        assert_eq!(Parameter::Integer(42).to_sql_literal().unwrap(), "42");
        assert_eq!(Parameter::Float(3.14).to_sql_literal().unwrap(), "3.14");
        assert_eq!(
            Parameter::String("hello".to_string())
                .to_sql_literal()
                .unwrap(),
            "'hello'"
        );
    }

    #[test]
    fn test_parameter_string_escaping() {
        let param = Parameter::String("O'Reilly".to_string());
        assert_eq!(param.to_sql_literal().unwrap(), "'O''Reilly'");
    }

    #[test]
    fn test_parameter_sql_injection_detection() {
        let dangerous = Parameter::String("'; DROP TABLE users; --".to_string());
        assert!(dangerous.to_sql_literal().is_err());

        let malicious = Parameter::String("' OR '1'='1".to_string());
        assert!(malicious.to_sql_literal().is_err());

        let safe = Parameter::String("It's a nice day".to_string());
        assert!(safe.to_sql_literal().is_ok());
    }

    #[test]
    fn test_parameter_conversions() {
        let _p: Parameter = true.into();
        let _p: Parameter = 42i32.into();
        let _p: Parameter = 42i64.into();
        let _p: Parameter = 3.14f64.into();
        let _p: Parameter = "test".into();
        let _p: Parameter = String::from("test").into();
        let _p: Parameter = vec![1u8, 2, 3].into();
    }

    #[test]
    fn test_statement_creation() {
        let stmt = Statement::new("SELECT * FROM users");

        assert_eq!(stmt.sql(), "SELECT * FROM users");
        assert_eq!(stmt.statement_type(), StatementType::Select);
        assert_eq!(stmt.timeout_ms(), 120_000);
    }

    #[test]
    fn test_statement_parameter_binding() {
        let mut stmt = Statement::new("SELECT * FROM users WHERE id = ?");

        stmt.bind(0, 42).unwrap();

        let final_sql = stmt.build_sql().unwrap();
        assert_eq!(final_sql, "SELECT * FROM users WHERE id = 42");
    }

    #[test]
    fn test_statement_multiple_parameters() {
        let mut stmt = Statement::new("SELECT * FROM users WHERE age > ? AND name = ?");

        stmt.bind(0, 18).unwrap();
        stmt.bind(1, "John").unwrap();

        let final_sql = stmt.build_sql().unwrap();
        assert_eq!(
            final_sql,
            "SELECT * FROM users WHERE age > 18 AND name = 'John'"
        );
    }

    #[test]
    fn test_statement_set_timeout() {
        let mut stmt = Statement::new("SELECT * FROM users");
        stmt.set_timeout(30_000);
        assert_eq!(stmt.timeout_ms(), 30_000);
    }

    #[test]
    fn test_statement_clear_parameters() {
        let mut stmt = Statement::new("SELECT * FROM users WHERE id = ?");
        stmt.bind(0, 42).unwrap();
        stmt.clear_parameters();
        assert!(stmt.parameters().is_empty());
    }

    #[test]
    fn test_statement_display() {
        let stmt = Statement::new("SELECT 1");
        let display = format!("{}", stmt);
        assert!(display.contains("SELECT 1"));
    }

    // --- build_sql tests (task 2.4) ---

    #[test]
    fn build_sql_substitutes_question_mark_in_normal_text() {
        let mut stmt = Statement::new("SELECT ?");
        stmt.bind(0, 42i64).unwrap();
        assert_eq!(stmt.build_sql().unwrap(), "SELECT 42");
    }

    #[test]
    fn build_sql_does_not_substitute_question_mark_in_single_quoted_string() {
        // The '?' inside the string literal must not be touched; no params needed.
        let stmt = Statement::new("SELECT 'a?b' AS v");
        assert_eq!(stmt.build_sql().unwrap(), "SELECT 'a?b' AS v");
    }

    #[test]
    fn build_sql_does_not_substitute_question_mark_in_double_quoted_identifier() {
        let stmt = Statement::new("SELECT 1 AS \"col?name\"");
        assert_eq!(stmt.build_sql().unwrap(), "SELECT 1 AS \"col?name\"");
    }

    #[test]
    fn build_sql_does_not_substitute_question_mark_in_line_comment() {
        // The '?' after -- is in a comment; only the trailing real '?' counts.
        let mut stmt = Statement::new("SELECT 1 -- has ?\n WHERE x = ?");
        stmt.bind(0, 7i64).unwrap();
        assert_eq!(stmt.build_sql().unwrap(), "SELECT 1 -- has ?\n WHERE x = 7");
    }

    #[test]
    fn build_sql_does_not_substitute_question_mark_in_block_comment() {
        let mut stmt = Statement::new("SELECT /* what? */ ?");
        stmt.bind(0, 99i64).unwrap();
        assert_eq!(stmt.build_sql().unwrap(), "SELECT /* what? */ 99");
    }

    #[test]
    fn build_sql_mixed_placeholder_and_literal_question_mark() {
        // Only the unquoted '?' after the comma should be replaced.
        let mut stmt = Statement::new("SELECT 'a?b', ?");
        stmt.bind(0, 5i64).unwrap();
        assert_eq!(stmt.build_sql().unwrap(), "SELECT 'a?b', 5");
    }

    #[test]
    fn build_sql_escaped_single_quote_in_string_with_question_mark_stays_literal() {
        // 'O''Reilly?' — the '?' inside is protected by the surrounding literal.
        let stmt = Statement::new("SELECT 'O''Reilly?'");
        assert_eq!(stmt.build_sql().unwrap(), "SELECT 'O''Reilly?'");
    }

    #[test]
    fn build_sql_empty_sql_with_no_params_returns_empty_string() {
        let stmt = Statement::new("");
        assert_eq!(stmt.build_sql().unwrap(), "");
    }

    #[test]
    fn build_sql_sql_ending_mid_string_literal_does_not_panic() {
        // Unterminated literal — no placeholder found, no params needed.
        let stmt = Statement::new("SELECT 'unclosed");
        assert_eq!(stmt.build_sql().unwrap(), "SELECT 'unclosed");
    }

    #[test]
    fn build_sql_round_trip_select_by_id() {
        let mut stmt = Statement::new("SELECT * FROM users WHERE id = ?");
        stmt.bind(0, 42i64).unwrap();
        assert_eq!(
            stmt.build_sql().unwrap(),
            "SELECT * FROM users WHERE id = 42"
        );
    }

    #[test]
    fn build_sql_not_enough_parameters_returns_error() {
        let stmt = Statement::new("SELECT ?, ?");
        // Only zero params bound → two placeholders but zero bound → error.
        let err = stmt.build_sql().unwrap_err();
        assert!(matches!(err, QueryError::ParameterBindingError { .. }));
    }

    #[test]
    fn build_sql_parameter_not_bound_returns_error() {
        let mut stmt = Statement::new("SELECT ?, ?");
        // Bind only index 1 (skipping 0) — index 0 remains None.
        stmt.bind(1, 99i64).unwrap();
        let err = stmt.build_sql().unwrap_err();
        assert!(matches!(
            err,
            QueryError::ParameterBindingError { index: 0, .. }
        ));
    }

    // --- scan_placeholders smoke tests (full coverage lives in task 2.4) ---

    #[test]
    fn scan_placeholders_empty_sql() {
        assert_eq!(scan_placeholders(""), Vec::<usize>::new());
    }

    #[test]
    fn scan_placeholders_returns_byte_offsets_in_normal_text() {
        // "SELECT ? , ?" → '?' at byte offset 7 and 11.
        let positions = scan_placeholders("SELECT ? , ?");
        assert_eq!(positions, vec![7, 11]);
    }

    #[test]
    fn scan_placeholders_ignores_question_mark_in_single_quoted_string() {
        // "SELECT 'a?b' AS v" → no placeholders.
        assert!(scan_placeholders("SELECT 'a?b' AS v").is_empty());
    }

    #[test]
    fn scan_placeholders_ignores_question_mark_in_double_quoted_identifier() {
        assert!(scan_placeholders("SELECT 1 AS \"col?name\"").is_empty());
    }

    #[test]
    fn scan_placeholders_ignores_question_mark_in_line_comment() {
        // -- comment ?\n then real placeholder
        let sql = "SELECT 1 -- has ?\n WHERE x = ?";
        let positions = scan_placeholders(sql);
        assert_eq!(positions.len(), 1, "got {:?}", positions);
        assert_eq!(&sql[positions[0]..positions[0] + 1], "?");
        // The real `?` is the last char.
        assert_eq!(positions[0], sql.len() - 1);
    }

    #[test]
    fn scan_placeholders_ignores_question_mark_in_block_comment() {
        let sql = "SELECT /* what? */ ?";
        let positions = scan_placeholders(sql);
        assert_eq!(positions.len(), 1);
        assert_eq!(&sql[positions[0]..positions[0] + 1], "?");
    }

    #[test]
    fn scan_placeholders_handles_escaped_single_quote() {
        // 'O''Reilly?' — escaped quote keeps us inside SingleQuoted, so the
        // '?' inside is ignored. The final '?' after the string is recorded.
        let sql = "SELECT 'O''Reilly?' = ?";
        let positions = scan_placeholders(sql);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0], sql.len() - 1);
    }

    #[test]
    fn scan_placeholders_handles_escaped_double_quote() {
        let sql = "SELECT \"a\"\"b?\" = ?";
        let positions = scan_placeholders(sql);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0], sql.len() - 1);
    }

    #[test]
    fn scan_placeholders_unterminated_string_does_not_panic() {
        // No closing quote — must not panic and must not record any '?'
        // inside the unterminated literal.
        let positions = scan_placeholders("SELECT 'a?b");
        assert!(positions.is_empty());
    }

    #[test]
    fn scan_placeholders_unterminated_block_comment_does_not_panic() {
        let positions = scan_placeholders("SELECT /* what? AND ? then EOF");
        assert!(positions.is_empty());
    }

    #[test]
    fn scan_placeholders_utf8_multibyte_offsets_are_byte_safe() {
        // "ä" is two bytes (0xC3 0xA4) in UTF-8. The '?' after "ä" sits at
        // byte offset 2. We must NOT panic and must record byte offset 2.
        let sql = "ä?";
        assert_eq!(sql.len(), 3);
        let positions = scan_placeholders(sql);
        assert_eq!(positions, vec![2]);
        assert_eq!(&sql[positions[0]..positions[0] + 1], "?");
    }

    #[test]
    fn scan_placeholders_consecutive_question_marks() {
        let positions = scan_placeholders("??");
        assert_eq!(positions, vec![0, 1]);
    }

    #[test]
    fn scan_placeholders_block_comment_inside_string_is_ignored() {
        // The "/*" appears inside a string literal, so we never enter
        // BlockComment. The trailing '?' after the string is a placeholder.
        let sql = "SELECT '/* ?  */', ?";
        let positions = scan_placeholders(sql);
        assert_eq!(positions, vec![sql.len() - 1]);
    }

    #[test]
    fn scan_placeholders_line_comment_inside_string_is_ignored() {
        let sql = "SELECT '-- still in string ?', ?";
        let positions = scan_placeholders(sql);
        assert_eq!(positions, vec![sql.len() - 1]);
    }
}
