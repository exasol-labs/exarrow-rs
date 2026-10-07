//! Connection parameter parsing and validation.
//!
//! This module handles parsing connection strings and building connection
//! parameters with validation.

use crate::error::ConnectionError;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

/// Connection parameters for establishing a database connection.
#[derive(Clone)]
pub struct ConnectionParams {
    /// Database host address
    pub host: String,

    /// Database port (default: 8563)
    pub port: u16,

    /// Username for authentication
    pub username: String,

    /// Password for authentication (stored securely)
    password: String,

    /// Optional schema to use after connection
    pub schema: Option<String>,

    /// Connection timeout
    pub connection_timeout: Duration,

    /// Query execution timeout. `None` means no `queryTimeout` session
    /// attribute is set and the server's own `QUERY_TIMEOUT` governs.
    pub query_timeout: Option<Duration>,

    /// Idle connection timeout
    pub idle_timeout: Duration,

    /// Enable TLS/SSL encryption
    pub use_tls: bool,

    /// TLS certificate validation mode
    pub validate_server_certificate: bool,

    /// Expected SHA-256 hex fingerprint of the server's DER certificate
    pub certificate_fingerprint: Option<String>,

    /// Client name for session identification
    pub client_name: String,

    /// Client version
    pub client_version: String,

    /// Transport type override (native or websocket)
    pub transport: Option<String>,

    /// Additional connection attributes
    pub attributes: HashMap<String, String>,
}

impl ConnectionParams {
    /// Get the password (for internal use only, never logged).
    pub(crate) fn password(&self) -> &str {
        &self.password
    }

    /// Create a new ConnectionBuilder.
    pub fn builder() -> ConnectionBuilder {
        ConnectionBuilder::new()
    }

    /// Parse a connection URI and merge credentials given outside it, such as
    /// the ADBC `username` and `password` database options.
    ///
    /// Each credential resolves on its own: the given value when it is `Some`,
    /// even when empty, else the URI userinfo, else the `user` then `username`
    /// query key (`password` then `pass` for the password). The password falls
    /// back to empty; a missing username fails with `Username is required`, so
    /// no connection logs in as a user the caller did not name. Given values
    /// are used verbatim and never percent-decoded, because they are not part
    /// of the URI. The credential query keys are always consumed, so none of
    /// them is kept as a connection attribute.
    pub(crate) fn parse_with_credentials(
        uri: &str,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Self, ConnectionError> {
        let url = uri.trim().strip_prefix("exasol://").ok_or_else(|| {
            ConnectionError::ParseError("Connection string must start with 'exasol://'".to_string())
        })?;

        let (main_part, query_string) = match url.split_once('?') {
            Some((main, query)) => (main, Some(query)),
            None => (url, None),
        };

        let mut params = parse_query_params(query_string)?;

        let (auth_part, host_part) = match main_part.rfind('@') {
            Some(pos) => (Some(&main_part[..pos]), &main_part[pos + 1..]),
            None => (None, main_part),
        };

        let (userinfo_username, userinfo_password) = match auth_part {
            Some(auth) => {
                let (user, pass) = parse_auth(auth)?;
                (Some(user), pass)
            }
            None => (None, None),
        };
        let query_user = params.remove("user");
        let query_username = params.remove("username");
        let query_password = params.remove("password");
        let query_pass = params.remove("pass");

        let username = username
            .map(str::to_string)
            .or(userinfo_username)
            .or(query_user)
            .or(query_username)
            .ok_or_else(|| ConnectionError::ParseError("Username is required".to_string()))?;
        let password = password
            .map(str::to_string)
            .or(userinfo_password)
            .or(query_password)
            .or(query_pass)
            .unwrap_or_default();

        let (host_port, schema) = match host_part.split_once('/') {
            Some((host, schema)) => (host, Some(schema).filter(|s| !s.is_empty())),
            None => (host_part, None),
        };

        let (host, port) = parse_host_port(host_port)?;

        let mut builder = ConnectionBuilder::new()
            .host(&host)
            .port(port)
            .username(&username)
            .password(&password);

        if let Some(schema) = schema {
            builder = builder.schema(schema);
        }

        builder = apply_query_params(builder, params)?;

        builder.build()
    }
}

impl FromStr for ConnectionParams {
    type Err = ConnectionError;

    /// Parse a connection string in the format:
    /// `exasol://username[:password]@host[:port][/schema][?param=value&...]`
    ///
    /// The username and password can also come from the `user`/`username` and
    /// `password`/`pass` query keys when the userinfo omits them.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_with_credentials(s, None, None)
    }
}

// Prevent password from being displayed in debug or display output
impl fmt::Debug for ConnectionParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("schema", &self.schema)
            .field("connection_timeout", &self.connection_timeout)
            .field("query_timeout", &self.query_timeout)
            .field("idle_timeout", &self.idle_timeout)
            .field("use_tls", &self.use_tls)
            .field(
                "validate_server_certificate",
                &self.validate_server_certificate,
            )
            .field("certificate_fingerprint", &self.certificate_fingerprint)
            .field("client_name", &self.client_name)
            .field("client_version", &self.client_version)
            .field("transport", &self.transport)
            .field("attributes", &self.attributes)
            .finish()
    }
}

impl fmt::Display for ConnectionParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ConnectionParams {{ host: {}, port: {}, username: {}, schema: {:?}, use_tls: {} }}",
            self.host, self.port, self.username, self.schema, self.use_tls
        )
    }
}

/// Builder for constructing ConnectionParams with validation.
#[derive(Clone)]
pub struct ConnectionBuilder {
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password: Option<String>,
    schema: Option<String>,
    connection_timeout: Option<Duration>,
    query_timeout: Option<Duration>,
    idle_timeout: Option<Duration>,
    use_tls: Option<bool>,
    validate_server_certificate: Option<bool>,
    certificate_fingerprint: Option<String>,
    client_name: Option<String>,
    client_version: Option<String>,
    transport: Option<String>,
    attributes: HashMap<String, String>,
}

impl ConnectionBuilder {
    /// Create a new ConnectionBuilder with default values.
    pub fn new() -> Self {
        Self {
            host: None,
            port: None,
            username: None,
            password: None,
            schema: None,
            connection_timeout: None,
            query_timeout: None,
            idle_timeout: None,
            use_tls: None,
            validate_server_certificate: None,
            certificate_fingerprint: None,
            client_name: None,
            client_version: None,
            transport: None,
            attributes: HashMap::new(),
        }
    }

    /// Set the database host.
    pub fn host(mut self, host: &str) -> Self {
        self.host = Some(host.to_string());
        self
    }

    /// Set the database port.
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Set the username.
    pub fn username(mut self, username: &str) -> Self {
        self.username = Some(username.to_string());
        self
    }

    /// Set the password.
    pub fn password(mut self, password: &str) -> Self {
        self.password = Some(password.to_string());
        self
    }

    /// Set the default schema.
    pub fn schema(mut self, schema: &str) -> Self {
        self.schema = Some(schema.to_string());
        self
    }

    /// Set the connection timeout.
    pub fn connection_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = Some(timeout);
        self
    }

    /// Set the query execution timeout.
    pub fn query_timeout(mut self, timeout: Duration) -> Self {
        self.query_timeout = Some(timeout);
        self
    }

    /// Set the idle connection timeout.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = Some(timeout);
        self
    }

    /// Enable or disable TLS/SSL.
    pub fn use_tls(mut self, use_tls: bool) -> Self {
        self.use_tls = Some(use_tls);
        self
    }

    /// Enable or disable server certificate validation.
    pub fn validate_server_certificate(mut self, validate: bool) -> Self {
        self.validate_server_certificate = Some(validate);
        self
    }

    /// Pin TLS connection to a specific certificate fingerprint (SHA-256 hex of DER cert).
    pub fn certificate_fingerprint(mut self, fingerprint: &str) -> Self {
        self.certificate_fingerprint = Some(fingerprint.to_string());
        self
    }

    /// Set the client name.
    pub fn client_name(mut self, name: &str) -> Self {
        self.client_name = Some(name.to_string());
        self
    }

    /// Set the client version.
    pub fn client_version(mut self, version: &str) -> Self {
        self.client_version = Some(version.to_string());
        self
    }

    /// Set the transport type (native or websocket).
    pub fn transport(mut self, transport: &str) -> Self {
        self.transport = Some(transport.to_string());
        self
    }

    /// Add a custom connection attribute.
    pub fn attribute(mut self, key: &str, value: &str) -> Self {
        self.attributes.insert(key.to_string(), value.to_string());
        self
    }

    /// Build the ConnectionParams with validation.
    pub fn build(self) -> Result<ConnectionParams, ConnectionError> {
        // Validate required fields
        let host = self.host.ok_or_else(|| ConnectionError::InvalidParameter {
            parameter: "host".to_string(),
            message: "Host is required".to_string(),
        })?;

        let username = self
            .username
            .ok_or_else(|| ConnectionError::InvalidParameter {
                parameter: "username".to_string(),
                message: "Username is required".to_string(),
            })?;

        // Validate host is not empty
        if host.is_empty() {
            return Err(ConnectionError::InvalidParameter {
                parameter: "host".to_string(),
                message: "Host cannot be empty".to_string(),
            });
        }

        // Validate username is not empty
        if username.is_empty() {
            return Err(ConnectionError::InvalidParameter {
                parameter: "username".to_string(),
                message: "Username cannot be empty".to_string(),
            });
        }

        let port = self.port.unwrap_or(8563);

        // Validate port range
        if port == 0 {
            return Err(ConnectionError::InvalidParameter {
                parameter: "port".to_string(),
                message: "Port must be greater than 0".to_string(),
            });
        }

        // Validate timeouts
        let connection_timeout = self.connection_timeout.unwrap_or(Duration::from_secs(30));
        let query_timeout = self.query_timeout;
        let idle_timeout = self.idle_timeout.unwrap_or(Duration::from_secs(600));

        if connection_timeout.as_secs() > 300 {
            return Err(ConnectionError::InvalidParameter {
                parameter: "connection_timeout".to_string(),
                message: "Connection timeout cannot exceed 300 seconds".to_string(),
            });
        }

        Ok(ConnectionParams {
            host,
            port,
            username,
            password: self.password.unwrap_or_default(),
            schema: self.schema,
            connection_timeout,
            query_timeout,
            idle_timeout,
            use_tls: self.use_tls.unwrap_or(true),
            validate_server_certificate: self.validate_server_certificate.unwrap_or(true),
            certificate_fingerprint: self.certificate_fingerprint,
            client_name: self.client_name.unwrap_or_else(|| "exarrow-rs".to_string()),
            client_version: self
                .client_version
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
            transport: self.transport,
            attributes: self.attributes,
        })
    }
}

impl Default for ConnectionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// Prevent password from being displayed in debug output
impl fmt::Debug for ConnectionBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionBuilder")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("schema", &self.schema)
            .field("connection_timeout", &self.connection_timeout)
            .field("query_timeout", &self.query_timeout)
            .field("idle_timeout", &self.idle_timeout)
            .field("use_tls", &self.use_tls)
            .field(
                "validate_server_certificate",
                &self.validate_server_certificate,
            )
            .field("certificate_fingerprint", &self.certificate_fingerprint)
            .field("client_name", &self.client_name)
            .field("client_version", &self.client_version)
            .field("transport", &self.transport)
            .field("attributes", &self.attributes)
            .finish()
    }
}

/// Parse query parameters from URL query string.
fn parse_query_params(query: Option<&str>) -> Result<HashMap<String, String>, ConnectionError> {
    let mut params = HashMap::new();

    if let Some(query) = query {
        for (index, pair) in query.split('&').enumerate() {
            if pair.is_empty() {
                continue;
            }

            // An unencoded password can shift into the query, so the error names the position only.
            let (key, value) = pair.split_once('=').ok_or_else(|| {
                ConnectionError::ParseError(format!(
                    "Invalid query parameter format at position {}: expected key=value",
                    index + 1
                ))
            })?;

            // URL decode the values
            let key = urlencoding::decode(key)
                .map_err(|e| ConnectionError::ParseError(format!("Failed to decode key: {}", e)))?
                .into_owned();
            let value = urlencoding::decode(value)
                .map_err(|e| ConnectionError::ParseError(format!("Failed to decode value: {}", e)))?
                .into_owned();

            params.insert(key, value);
        }
    }

    Ok(params)
}

/// Parse authentication part (username[:password]). The password is `Some`
/// only when the part contains `:`, so `alice:` gives an empty password.
fn parse_auth(auth: &str) -> Result<(String, Option<String>), ConnectionError> {
    let (user, pass) = match auth.split_once(':') {
        Some((user, pass)) => (user, Some(pass)),
        None => (auth, None),
    };
    let user = urlencoding::decode(user)
        .map_err(|e| ConnectionError::ParseError(format!("Failed to decode username: {}", e)))?
        .into_owned();
    let pass = pass
        .map(|pass| {
            urlencoding::decode(pass)
                .map(|decoded| decoded.into_owned())
                .map_err(|e| {
                    ConnectionError::ParseError(format!("Failed to decode password: {}", e))
                })
        })
        .transpose()?;
    Ok((user, pass))
}

/// Parse host and port.
fn parse_host_port(host_port: &str) -> Result<(String, u16), ConnectionError> {
    // The port text is never echoed: an unencoded '?' in a password leaves the password there.
    let invalid_port =
        || ConnectionError::ParseError("Invalid port: expected a number from 1 to 65535".into());

    // Check for IPv6 address format [host]:port
    if host_port.starts_with('[') {
        if let Some(close_bracket) = host_port.find(']') {
            let host = host_port[1..close_bracket].to_string();
            let port_part = &host_port[close_bracket + 1..];

            let port = if let Some(stripped) = port_part.strip_prefix(':') {
                stripped.parse().map_err(|_| invalid_port())?
            } else {
                8563
            };

            return Ok((host, port));
        }
    }

    // Regular host:port or just host
    match host_port.rsplit_once(':') {
        Some((host, port_str)) => {
            let port = port_str.parse().map_err(|_| invalid_port())?;
            Ok((host.to_string(), port))
        }
        None => Ok((host_port.to_string(), 8563)),
    }
}

/// Apply query parameters to builder.
///
/// Errors name the query key and never repeat its value, because an unencoded
/// password can shift into the query string.
fn apply_query_params(
    mut builder: ConnectionBuilder,
    params: HashMap<String, String>,
) -> Result<ConnectionBuilder, ConnectionError> {
    for (key, value) in params {
        match key.as_str() {
            "timeout" | "connection_timeout" => {
                builder = builder.connection_timeout(parse_timeout(&key, &value)?);
            }
            "query_timeout" => {
                builder = builder.query_timeout(parse_timeout(&key, &value)?);
            }
            "idle_timeout" => {
                builder = builder.idle_timeout(parse_timeout(&key, &value)?);
            }
            "tls" | "use_tls" | "ssl" => {
                let use_tls = parse_bool(&key, &value)?;
                builder = builder.use_tls(use_tls);
            }
            "validate_certificate" | "verify_certificate" | "validateservercertificate" => {
                let validate = parse_bool(&key, &value)?;
                builder = builder.validate_server_certificate(validate);
            }
            "client_name" => {
                builder = builder.client_name(&value);
            }
            "client_version" => {
                builder = builder.client_version(&value);
            }
            "certificate_fingerprint" | "certificatefingerprint" => {
                builder = builder.certificate_fingerprint(&value);
            }
            "transport" => {
                let transport_lower = value.to_lowercase();
                match transport_lower.as_str() {
                    "native" | "websocket" => {
                        builder = builder.transport(&transport_lower);
                    }
                    _ => {
                        return Err(ConnectionError::InvalidParameter {
                            parameter: "transport".to_string(),
                            message: "Invalid transport value. Must be 'native' or 'websocket'"
                                .to_string(),
                        });
                    }
                }
            }
            _ => {
                // Store as custom attribute
                builder = builder.attribute(&key, &value);
            }
        }
    }

    Ok(builder)
}

/// Parse a timeout given in whole seconds.
fn parse_timeout(key: &str, value: &str) -> Result<Duration, ConnectionError> {
    value
        .parse()
        .map(Duration::from_secs)
        .map_err(|_| ConnectionError::InvalidParameter {
            parameter: key.to_string(),
            message: "Invalid timeout value: expected a whole number of seconds".to_string(),
        })
}

/// Parse the boolean value of query key `key`.
fn parse_bool(key: &str, value: &str) -> Result<bool, ConnectionError> {
    match value.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(ConnectionError::InvalidParameter {
            parameter: key.to_string(),
            message: "Invalid boolean value: expected true, false, 1, 0, yes, no, on, or off"
                .to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_minimal() {
        let params = ConnectionBuilder::new()
            .host("localhost")
            .username("test")
            .build()
            .unwrap();

        assert_eq!(params.host, "localhost");
        assert_eq!(params.port, 8563);
        assert_eq!(params.username, "test");
        assert_eq!(params.password(), "");
    }

    #[test]
    fn test_builder_full() {
        let params = ConnectionBuilder::new()
            .host("db.example.com")
            .port(9000)
            .username("admin")
            .password("secret")
            .schema("MY_SCHEMA")
            .connection_timeout(Duration::from_secs(20))
            .query_timeout(Duration::from_secs(60))
            .use_tls(true)
            .client_name("test-client")
            .attribute("custom", "value")
            .build()
            .unwrap();

        assert_eq!(params.host, "db.example.com");
        assert_eq!(params.port, 9000);
        assert_eq!(params.username, "admin");
        assert_eq!(params.password(), "secret");
        assert_eq!(params.schema, Some("MY_SCHEMA".to_string()));
        assert_eq!(params.connection_timeout, Duration::from_secs(20));
        assert_eq!(params.query_timeout, Some(Duration::from_secs(60)));
        assert!(params.use_tls);
        assert_eq!(params.client_name, "test-client");
        assert_eq!(params.attributes.get("custom"), Some(&"value".to_string()));
    }

    #[test]
    fn test_builder_validation_missing_host() {
        let result = ConnectionBuilder::new().username("test").build();

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConnectionError::InvalidParameter { parameter, .. } if parameter == "host"
        ));
    }

    #[test]
    fn test_builder_validation_empty_host() {
        let result = ConnectionBuilder::new().host("").username("test").build();

        assert!(result.is_err());
    }

    #[test]
    fn test_builder_validation_timeout() {
        let result = ConnectionBuilder::new()
            .host("localhost")
            .username("test")
            .connection_timeout(Duration::from_secs(400))
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_basic() {
        let params = ConnectionParams::from_str("exasol://user@localhost").unwrap();

        assert_eq!(params.host, "localhost");
        assert_eq!(params.port, 8563);
        assert_eq!(params.username, "user");
    }

    #[test]
    fn test_parse_with_port() {
        let params = ConnectionParams::from_str("exasol://user@localhost:9000").unwrap();

        assert_eq!(params.host, "localhost");
        assert_eq!(params.port, 9000);
    }

    #[test]
    fn test_parse_with_password() {
        let params = ConnectionParams::from_str("exasol://user:pass@localhost").unwrap();

        assert_eq!(params.username, "user");
        assert_eq!(params.password(), "pass");
    }

    #[test]
    fn test_parse_with_schema() {
        let params = ConnectionParams::from_str("exasol://user@localhost/MY_SCHEMA").unwrap();

        assert_eq!(params.schema, Some("MY_SCHEMA".to_string()));
    }

    #[test]
    fn test_parse_with_query_params() {
        let params = ConnectionParams::from_str(
            "exasol://user@localhost?timeout=20&tls=true&client_name=test",
        )
        .unwrap();

        assert_eq!(params.connection_timeout, Duration::from_secs(20));
        assert!(params.use_tls);
        assert_eq!(params.client_name, "test");
    }

    #[test]
    fn test_parse_full_url() {
        let params = ConnectionParams::from_str(
            "exasol://admin:secret@db.example.com:9000/PROD?timeout=30&tls=true",
        )
        .unwrap();

        assert_eq!(params.host, "db.example.com");
        assert_eq!(params.port, 9000);
        assert_eq!(params.username, "admin");
        assert_eq!(params.password(), "secret");
        assert_eq!(params.schema, Some("PROD".to_string()));
        assert_eq!(params.connection_timeout, Duration::from_secs(30));
        assert!(params.use_tls);
    }

    #[test]
    fn test_parse_url_encoded() {
        let params = ConnectionParams::from_str("exasol://user%40test:p%40ss@localhost").unwrap();

        assert_eq!(params.username, "user@test");
        assert_eq!(params.password(), "p@ss");
    }

    #[test]
    fn test_parse_ipv6() {
        let params = ConnectionParams::from_str("exasol://user@[::1]:8563").unwrap();

        assert_eq!(params.host, "::1");
        assert_eq!(params.port, 8563);
    }

    #[test]
    fn test_parse_invalid_scheme() {
        let result = ConnectionParams::from_str("postgres://user@localhost");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_missing_username() {
        let result = ConnectionParams::from_str("exasol://localhost");
        assert!(result.is_err());
    }

    #[test]
    fn test_display_no_password_leak() {
        let params = ConnectionBuilder::new()
            .host("localhost")
            .username("admin")
            .password("super_secret")
            .build()
            .unwrap();

        let display = format!("{}", params);
        assert!(!display.contains("super_secret"));
        assert!(display.contains("localhost"));
        assert!(display.contains("admin"));
    }

    #[test]
    fn test_debug_no_password_leak() {
        let params = ConnectionBuilder::new()
            .host("localhost")
            .username("admin")
            .password("super_secret")
            .build()
            .unwrap();

        let debug = format!("{:?}", params);
        // Debug output should not contain the password
        assert!(!debug.contains("super_secret"));
    }

    // ============================================================
    // Builder validation tests
    // ============================================================

    #[test]
    fn test_builder_validation_missing_username() {
        let result = ConnectionBuilder::new().host("localhost").build();

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConnectionError::InvalidParameter { parameter, .. } if parameter == "username"
        ));
    }

    #[test]
    fn test_builder_validation_empty_username() {
        let result = ConnectionBuilder::new()
            .host("localhost")
            .username("")
            .build();

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConnectionError::InvalidParameter { parameter, message }
                if parameter == "username" && message.contains("empty")
        ));
    }

    #[test]
    fn test_builder_validation_port_zero() {
        let result = ConnectionBuilder::new()
            .host("localhost")
            .username("test")
            .port(0)
            .build();

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConnectionError::InvalidParameter { parameter, message }
                if parameter == "port" && message.contains("greater than 0")
        ));
    }

    #[test]
    fn test_builder_default() {
        let builder = ConnectionBuilder::default();
        let result = builder.host("localhost").username("user").build().unwrap();
        assert_eq!(result.host, "localhost");
    }

    #[test]
    fn test_connection_params_builder_method() {
        let builder = ConnectionParams::builder();
        let params = builder.host("localhost").username("user").build().unwrap();
        assert_eq!(params.host, "localhost");
    }

    #[test]
    fn test_builder_idle_timeout() {
        let params = ConnectionBuilder::new()
            .host("localhost")
            .username("test")
            .idle_timeout(Duration::from_secs(120))
            .build()
            .unwrap();

        assert_eq!(params.idle_timeout, Duration::from_secs(120));
    }

    #[test]
    fn test_builder_validate_server_certificate() {
        let params = ConnectionBuilder::new()
            .host("localhost")
            .username("test")
            .validate_server_certificate(false)
            .build()
            .unwrap();

        assert!(!params.validate_server_certificate);
    }

    #[test]
    fn test_builder_client_version() {
        let params = ConnectionBuilder::new()
            .host("localhost")
            .username("test")
            .client_version("1.2.3")
            .build()
            .unwrap();

        assert_eq!(params.client_version, "1.2.3");
    }

    #[test]
    fn test_builder_default_values() {
        let params = ConnectionBuilder::new()
            .host("localhost")
            .username("test")
            .build()
            .unwrap();

        assert_eq!(params.connection_timeout, Duration::from_secs(30));
        assert_eq!(params.query_timeout, None);
        assert_eq!(params.idle_timeout, Duration::from_secs(600));
        assert!(params.use_tls);
        assert!(params.validate_server_certificate);
        assert_eq!(params.client_name, "exarrow-rs");
    }

    // ============================================================
    // Query parameter parsing tests
    // ============================================================

    #[test]
    fn test_parse_query_param_without_equals() {
        let result = ConnectionParams::from_str("exasol://user@localhost?invalid_param");

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConnectionError::ParseError(msg) if msg.contains("Invalid query parameter format")
        ));
    }

    #[test]
    fn test_parse_query_param_empty_pairs() {
        // Empty pairs between && should be skipped
        let params =
            ConnectionParams::from_str("exasol://user@localhost?timeout=10&&tls=true").unwrap();

        assert_eq!(params.connection_timeout, Duration::from_secs(10));
        assert!(params.use_tls);
    }

    #[test]
    fn test_parse_query_timeout() {
        let params =
            ConnectionParams::from_str("exasol://user@localhost?query_timeout=60").unwrap();

        assert_eq!(params.query_timeout, Some(Duration::from_secs(60)));
    }

    #[test]
    fn test_parse_idle_timeout() {
        let params =
            ConnectionParams::from_str("exasol://user@localhost?idle_timeout=120").unwrap();

        assert_eq!(params.idle_timeout, Duration::from_secs(120));
    }

    #[test]
    fn test_parse_connection_timeout_param() {
        let params =
            ConnectionParams::from_str("exasol://user@localhost?connection_timeout=15").unwrap();

        assert_eq!(params.connection_timeout, Duration::from_secs(15));
    }

    #[test]
    fn test_parse_invalid_timeout_value() {
        let result = ConnectionParams::from_str("exasol://user@localhost?timeout=not_a_number");

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConnectionError::InvalidParameter { parameter, message }
                if parameter == "timeout" && message.contains("Invalid timeout value")
        ));
    }

    #[test]
    fn test_parse_invalid_query_timeout_value() {
        let result =
            ConnectionParams::from_str("exasol://user@localhost?query_timeout=not_a_number");

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConnectionError::InvalidParameter { parameter, .. } if parameter == "query_timeout"
        ));
    }

    #[test]
    fn test_parse_invalid_idle_timeout_value() {
        let result =
            ConnectionParams::from_str("exasol://user@localhost?idle_timeout=not_a_number");

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConnectionError::InvalidParameter { parameter, .. } if parameter == "idle_timeout"
        ));
    }

    // ============================================================
    // TLS/SSL parameter tests
    // ============================================================

    #[test]
    fn test_parse_ssl_param() {
        let params = ConnectionParams::from_str("exasol://user@localhost?ssl=true").unwrap();

        assert!(params.use_tls);
    }

    #[test]
    fn test_parse_use_tls_param() {
        let params = ConnectionParams::from_str("exasol://user@localhost?use_tls=1").unwrap();

        assert!(params.use_tls);
    }

    #[test]
    fn test_parse_validate_certificate_param() {
        let params =
            ConnectionParams::from_str("exasol://user@localhost?validate_certificate=false")
                .unwrap();

        assert!(!params.validate_server_certificate);
    }

    #[test]
    fn test_parse_verify_certificate_param() {
        let params =
            ConnectionParams::from_str("exasol://user@localhost?verify_certificate=0").unwrap();

        assert!(!params.validate_server_certificate);
    }

    #[test]
    fn test_parse_validateservercertificate_param() {
        let params =
            ConnectionParams::from_str("exasol://user@localhost?validateservercertificate=no")
                .unwrap();

        assert!(!params.validate_server_certificate);
    }

    // ============================================================
    // Boolean parsing tests
    // ============================================================

    #[test]
    fn test_parse_bool_yes() {
        let params = ConnectionParams::from_str("exasol://user@localhost?tls=yes").unwrap();
        assert!(params.use_tls);
    }

    #[test]
    fn test_parse_bool_no() {
        let params = ConnectionParams::from_str("exasol://user@localhost?tls=no").unwrap();
        assert!(!params.use_tls);
    }

    #[test]
    fn test_parse_bool_on() {
        let params = ConnectionParams::from_str("exasol://user@localhost?tls=on").unwrap();
        assert!(params.use_tls);
    }

    #[test]
    fn test_parse_bool_off() {
        let params = ConnectionParams::from_str("exasol://user@localhost?tls=off").unwrap();
        assert!(!params.use_tls);
    }

    #[test]
    fn test_parse_bool_one() {
        let params = ConnectionParams::from_str("exasol://user@localhost?tls=1").unwrap();
        assert!(params.use_tls);
    }

    #[test]
    fn test_parse_bool_zero() {
        let params = ConnectionParams::from_str("exasol://user@localhost?tls=0").unwrap();
        assert!(!params.use_tls);
    }

    #[test]
    fn test_parse_bool_case_insensitive() {
        let params = ConnectionParams::from_str("exasol://user@localhost?tls=TRUE").unwrap();
        assert!(params.use_tls);

        let params = ConnectionParams::from_str("exasol://user@localhost?tls=FALSE").unwrap();
        assert!(!params.use_tls);
    }

    #[test]
    fn test_parse_bool_invalid() {
        let result = ConnectionParams::from_str("exasol://user@localhost?tls=maybe");

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConnectionError::InvalidParameter { parameter, message }
                if parameter == "tls" && message.contains("Invalid boolean value")
        ));
    }

    // ============================================================
    // Client info parameter tests
    // ============================================================

    #[test]
    fn test_parse_client_version_param() {
        let params =
            ConnectionParams::from_str("exasol://user@localhost?client_version=2.0.0").unwrap();

        assert_eq!(params.client_version, "2.0.0");
    }

    #[test]
    fn test_parse_custom_attribute_param() {
        let params =
            ConnectionParams::from_str("exasol://user@localhost?custom_key=custom_value").unwrap();

        assert_eq!(
            params.attributes.get("custom_key"),
            Some(&"custom_value".to_string())
        );
    }

    // ============================================================
    // Authentication from query params tests
    // ============================================================

    #[test]
    fn test_parse_username_from_query_user() {
        let params = ConnectionParams::from_str("exasol://localhost?user=testuser").unwrap();

        assert_eq!(params.username, "testuser");
    }

    #[test]
    fn test_parse_username_from_query_username() {
        let params = ConnectionParams::from_str("exasol://localhost?username=testuser").unwrap();

        assert_eq!(params.username, "testuser");
    }

    #[test]
    fn test_parse_password_from_query_password() {
        let params =
            ConnectionParams::from_str("exasol://localhost?user=testuser&password=secret").unwrap();

        assert_eq!(params.password(), "secret");
    }

    #[test]
    fn test_parse_password_from_query_pass() {
        let params =
            ConnectionParams::from_str("exasol://localhost?user=testuser&pass=secret").unwrap();

        assert_eq!(params.password(), "secret");
    }

    #[test]
    fn test_parse_auth_from_query_no_password() {
        let params = ConnectionParams::from_str("exasol://localhost?user=testuser").unwrap();

        assert_eq!(params.username, "testuser");
        assert_eq!(params.password(), "");
    }

    // ============================================================
    // IPv6 tests
    // ============================================================

    #[test]
    fn test_parse_ipv6_without_port() {
        let params = ConnectionParams::from_str("exasol://user@[::1]").unwrap();

        assert_eq!(params.host, "::1");
        assert_eq!(params.port, 8563);
    }

    #[test]
    fn test_parse_ipv6_full_address() {
        let params = ConnectionParams::from_str("exasol://user@[2001:db8::1]:9000/schema").unwrap();

        assert_eq!(params.host, "2001:db8::1");
        assert_eq!(params.port, 9000);
        assert_eq!(params.schema, Some("schema".to_string()));
    }

    // ============================================================
    // Schema edge cases
    // ============================================================

    #[test]
    fn test_parse_empty_schema_path() {
        let params = ConnectionParams::from_str("exasol://user@localhost/").unwrap();

        assert_eq!(params.schema, None);
    }

    #[test]
    fn test_parse_schema_with_query_params() {
        let params =
            ConnectionParams::from_str("exasol://user@localhost/MY_SCHEMA?tls=true").unwrap();

        assert_eq!(params.schema, Some("MY_SCHEMA".to_string()));
        assert!(params.use_tls);
    }

    // ============================================================
    // Port parsing edge cases
    // ============================================================

    #[test]
    fn test_parse_invalid_port() {
        let result = ConnectionParams::from_str("exasol://user@localhost:not_a_port");

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConnectionError::ParseError(msg) if msg.contains("Invalid port")
        ));
    }

    #[test]
    fn test_parse_ipv6_invalid_port() {
        let result = ConnectionParams::from_str("exasol://user@[::1]:invalid");

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConnectionError::ParseError(msg) if msg.contains("Invalid port")
        ));
    }

    // ============================================================
    // URL encoding edge cases
    // ============================================================

    #[test]
    fn test_parse_url_encoded_query_params() {
        let params =
            ConnectionParams::from_str("exasol://user@localhost?client_name=my%20client").unwrap();

        assert_eq!(params.client_name, "my client");
    }

    #[test]
    fn test_parse_auth_without_password() {
        let params = ConnectionParams::from_str("exasol://testuser@localhost").unwrap();

        assert_eq!(params.username, "testuser");
        assert_eq!(params.password(), "");
    }

    // ============================================================
    // Whitespace handling tests
    // ============================================================

    #[test]
    fn test_parse_url_with_whitespace_trim() {
        let params = ConnectionParams::from_str("  exasol://user@localhost  ").unwrap();

        assert_eq!(params.host, "localhost");
        assert_eq!(params.username, "user");
    }

    #[test]
    fn test_parse_certificate_fingerprint_param() {
        let params = "exasol://user:pass@localhost?tls=true&certificate_fingerprint=aabbcc"
            .parse::<ConnectionParams>()
            .unwrap();
        assert_eq!(params.certificate_fingerprint.as_deref(), Some("aabbcc"));
    }

    #[test]
    fn test_parse_certificatefingerprint_alias() {
        let params = "exasol://user:pass@localhost?tls=true&certificatefingerprint=ddeeff"
            .parse::<ConnectionParams>()
            .unwrap();
        assert_eq!(params.certificate_fingerprint.as_deref(), Some("ddeeff"));
    }

    // ============================================================
    // Credential sources
    // ============================================================

    /// Scenario: Option credentials take precedence over URI credentials
    #[test]
    fn test_parse_with_credentials_options_override_userinfo() {
        let params = ConnectionParams::parse_with_credentials(
            "exasol://nobody:Wrong1@db.example.com:8563",
            Some("alice"),
            Some("Secret1"),
        )
        .unwrap();

        assert_eq!(params.username, "alice");
        assert_eq!(params.password(), "Secret1");
    }

    /// Scenario: A single credential option replaces only its own field
    #[test]
    fn test_parse_with_credentials_password_option_keeps_userinfo_username() {
        let params = ConnectionParams::parse_with_credentials(
            "exasol://alice:Wrong1@db.example.com:8563",
            None,
            Some("Secret1"),
        )
        .unwrap();

        assert_eq!(params.username, "alice");
        assert_eq!(params.password(), "Secret1");
    }

    /// Scenario: Option password reaches the server verbatim
    #[test]
    fn test_parse_with_credentials_keeps_option_password_verbatim() {
        for password in ["Ab?cd1234", "Ab%41cd1234", "Ab@:/#cd1234"] {
            let params = ConnectionParams::parse_with_credentials(
                "exasol://db.example.com:8563",
                Some("alice"),
                Some(password),
            )
            .unwrap();

            assert_eq!(params.password(), password);
            assert_eq!(params.host, "db.example.com");
        }
    }

    /// Scenario: An at sign in a query parameter value does not change the host
    #[test]
    fn test_parse_with_credentials_at_sign_in_query_value_keeps_host() {
        let params = ConnectionParams::parse_with_credentials(
            "exasol://db.example.com:8563?client_name=dbt@ci",
            Some("alice"),
            Some("Secret1"),
        )
        .unwrap();

        assert_eq!(params.host, "db.example.com");
        assert_eq!(params.port, 8563);
        assert_eq!(params.client_name, "dbt@ci");
    }

    /// Scenario: Missing username is rejected
    #[test]
    fn test_parse_with_credentials_requires_username() {
        let message = ConnectionParams::parse_with_credentials(
            "exasol://db.example.com:8563",
            None,
            Some("Secret1"),
        )
        .expect_err("a URI without any username source must be refused")
        .to_string();

        assert!(message.contains("Username is required"), "got: {message}");
        assert!(!message.contains("Secret1"), "got: {message}");
    }

    #[test]
    fn test_parse_with_credentials_empty_username_option_is_rejected() {
        let message = ConnectionParams::parse_with_credentials(
            "exasol://alice:Pw1@db.example.com:8563",
            Some(""),
            None,
        )
        .expect_err("an empty username option must not fall back to the URI user")
        .to_string();

        assert!(
            message.contains("Username cannot be empty"),
            "got: {message}"
        );
    }

    /// Scenario: URI credentials apply when no credential option is set
    #[test]
    fn test_parse_with_credentials_decodes_userinfo_without_options() {
        let params = ConnectionParams::parse_with_credentials(
            "exasol://alice:Ab%3Fcd1234@db.example.com:8563",
            None,
            None,
        )
        .unwrap();

        assert_eq!(params.username, "alice");
        assert_eq!(params.password(), "Ab?cd1234");
    }

    /// Scenario: Query credentials fill only what the userinfo omits
    #[test]
    fn test_parse_query_credentials_fill_only_what_userinfo_omits() {
        let params = ConnectionParams::from_str(
            "exasol://alice@db.example.com:8563?user=bob&username=carol&password=Secret1&pass=Other1",
        )
        .unwrap();

        assert_eq!(params.username, "alice");
        assert_eq!(params.password(), "Secret1");
        for key in ["user", "username", "password", "pass"] {
            assert!(
                !params.attributes.contains_key(key),
                "credential key {key} must not be kept as an attribute"
            );
        }
    }

    /// Scenario: Query user applies when only the password option is set
    #[test]
    fn test_parse_with_credentials_query_user_with_password_option() {
        let params = ConnectionParams::parse_with_credentials(
            "exasol://db.example.com:8563?user=bob&username=carol",
            None,
            Some("Secret1"),
        )
        .unwrap();

        assert_eq!(params.username, "bob");
        assert_eq!(params.password(), "Secret1");
    }

    /// Scenario: Password stays out of Debug output
    #[test]
    fn test_debug_redacts_password_from_every_source() {
        let from_option = ConnectionParams::parse_with_credentials(
            "exasol://db.example.com:8563",
            Some("alice"),
            Some("Secret1"),
        )
        .unwrap();
        let from_userinfo =
            ConnectionParams::from_str("exasol://alice:Secret1@db.example.com:8563").unwrap();
        let from_query =
            ConnectionParams::from_str("exasol://alice@db.example.com:8563?password=Secret1")
                .unwrap();
        for params in [&from_option, &from_userinfo, &from_query] {
            assert_eq!(params.password(), "Secret1");
        }
        let builder = ConnectionBuilder::new()
            .host("localhost")
            .password("Secret1");

        for debug in [
            format!("{from_option:?}"),
            format!("{from_userinfo:?}"),
            format!("{from_query:?}"),
            format!("{builder:?}"),
        ] {
            assert!(!debug.contains("Secret1"), "got: {debug}");
        }
    }

    // ============================================================
    // Parse errors
    // ============================================================

    /// Scenario: URI parse errors do not repeat URI values
    #[test]
    fn test_parse_errors_do_not_repeat_uri_values() {
        // (URI, field named without the username option, field named with it)
        let cases = [
            (
                "exasol://alice:Qx7?Kp9@db.example.com:8563",
                "position 1",
                "position 1",
            ),
            (
                "exasol://alice:Qx7?k=1@db.example.com:8563",
                "Username is required",
                "Invalid port",
            ),
            (
                "exasol://alice@db.example.com:Qx7",
                "Invalid port",
                "Invalid port",
            ),
            ("exasol://alice@[::1]:Qx7", "Invalid port", "Invalid port"),
            (
                "exasol://alice@db.example.com:8563?timeout=Qx7",
                "'timeout'",
                "'timeout'",
            ),
            (
                "exasol://alice@db.example.com:8563?tls=Qx7",
                "'tls'",
                "'tls'",
            ),
            (
                "exasol://alice@db.example.com:8563?transport=Qx7",
                "'transport'",
                "'transport'",
            ),
        ];

        for (uri, without_option, with_option) in cases {
            let outcomes = [
                (ConnectionParams::from_str(uri), without_option),
                (
                    ConnectionParams::parse_with_credentials(uri, Some("alice"), None),
                    with_option,
                ),
            ];
            for (result, field) in outcomes {
                let message = result
                    .expect_err("an invalid URI must be refused")
                    .to_string();
                assert!(message.contains(field), "{uri}: got {message}");
                for value in ["Qx7", "Kp9", "db.example.com"] {
                    assert!(!message.contains(value), "{uri}: {message} repeats {value}");
                }
            }
        }
    }

    #[test]
    fn test_parse_query_param_error_counts_every_ampersand_separated_part() {
        let cases = [
            (
                "exasol://alice@db.example.com:8563?tls=true&Kp9",
                "position 2",
            ),
            (
                "exasol://alice@db.example.com:8563?tls=true&&Kp9",
                "position 3",
            ),
        ];

        for (uri, position) in cases {
            let message = ConnectionParams::from_str(uri)
                .expect_err("a query part without '=' must be refused")
                .to_string();

            assert!(message.contains(position), "{uri}: got {message}");
            assert!(!message.contains("Kp9"), "{uri}: got {message}");
        }
    }

    /// Scenario: URI parse errors do not repeat URI values
    #[test]
    fn test_parse_error_for_unencoded_password_omits_password() {
        let message = ConnectionParams::from_str("exasol://u:pa?ss@host")
            .expect_err("an unencoded '?' in the password must fail to parse")
            .to_string();
        let other_password = ConnectionParams::from_str("exasol://u:xy?zw@host")
            .expect_err("an unencoded '?' in the password must fail to parse")
            .to_string();

        assert_eq!(
            message, other_password,
            "the error text must not depend on the password"
        );
        assert!(!message.contains("ss"), "got: {message}");
        assert!(!message.contains("pa?ss"), "got: {message}");
    }
}
