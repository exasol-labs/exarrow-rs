//! Schema-activation checks that the native and WebSocket integration suites
//! run over their own transport, so both transports answer the same scenarios.
//!
//! Each check creates uniquely named schemas, quotes them where the case
//! matters, and drops them before it asserts, so a rejected URI schema or a
//! failed assertion leaves no schema behind on the server.

use arrow::array::{Array, StringArray};
use exarrow_rs::adbc::{Connection, Driver};
use exarrow_rs::error::ExasolError;

use crate::common::{
    generate_unique_test_name, get_host, get_password, get_port,
    get_test_connection_with_transport, get_user,
};

/// A connection string for `transport` whose path names `schema`.
fn uri_with_schema(transport: &str, schema: &str) -> String {
    format!(
        "exasol://{}:{}@{}:{}/{}?tls=true&validateservercertificate=0&transport={}",
        get_user(),
        get_password(),
        get_host(),
        get_port(),
        schema,
        transport
    )
}

fn quoted(name: &str) -> String {
    format!("\"{name}\"")
}

/// Connect through the ADBC `Driver` with `schema` in the URI path.
pub async fn connect_with_uri_schema(
    transport: &str,
    schema: &str,
) -> Result<Connection, ExasolError> {
    let database = Driver::new().open(&uri_with_schema(transport, schema))?;
    Ok(database.connect().await?)
}

async fn connect_without_schema(transport: &str) -> Connection {
    get_test_connection_with_transport(transport)
        .await
        .expect("a connection without a schema must succeed")
}

/// The current schema as the server itself reports it in `SELECT CURRENT_SCHEMA`.
pub async fn server_current_schema(conn: &mut Connection) -> Option<String> {
    let batches = conn
        .query("SELECT CURRENT_SCHEMA")
        .await
        .expect("SELECT CURRENT_SCHEMA must succeed");
    let column = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("CURRENT_SCHEMA is a string column");
    (!column.is_null(0)).then(|| column.value(0).to_string())
}

async fn create_schemas(admin: &mut Connection, names: &[&str]) {
    for name in names {
        admin
            .execute_update(format!("CREATE SCHEMA {}", quoted(name)))
            .await
            .expect("CREATE SCHEMA must succeed");
    }
}

async fn drop_schemas(admin: &mut Connection, names: &[&str]) {
    for name in names {
        let _ = admin
            .execute_update(format!("DROP SCHEMA IF EXISTS {} CASCADE", quoted(name)))
            .await;
    }
}

/// What `current_schema()` and the server report right after a connect with
/// `uri_schema` in the URI.
async fn opened_schema(
    transport: &str,
    uri_schema: &str,
) -> Result<(Option<String>, Option<String>), String> {
    let mut conn = connect_with_uri_schema(transport, uri_schema)
        .await
        .map_err(|e| format!("connect with URI schema {uri_schema} failed: {e}"))?;
    let reported = conn.current_schema().await;
    let on_server = server_current_schema(&mut conn).await;
    conn.close().await.expect("close must succeed");
    Ok((reported, on_server))
}

/// A URI schema becomes the current schema at connect, and an unqualified
/// table name resolves against it.
pub async fn check_uri_schema_is_opened_on_connect(transport: &str) {
    let schema = generate_unique_test_name("ZZ_URI_SCHEMA").to_uppercase();
    let mut admin = connect_without_schema(transport).await;
    create_schemas(&mut admin, &[&schema]).await;
    for sql in [
        format!("CREATE TABLE {}.PROBE (ID INTEGER)", quoted(&schema)),
        format!("INSERT INTO {}.PROBE VALUES (1), (2)", quoted(&schema)),
    ] {
        admin
            .execute_update(sql)
            .await
            .expect("the probe table must be set up");
    }

    let connected = match connect_with_uri_schema(transport, &schema).await {
        Ok(mut conn) => {
            let reported = conn.current_schema().await;
            let unqualified = conn.query("SELECT ID FROM PROBE").await;
            conn.close().await.expect("close must succeed");
            Ok((reported, unqualified))
        }
        Err(error) => Err(error),
    };

    drop_schemas(&mut admin, &[&schema]).await;
    admin.close().await.expect("admin close must succeed");

    let (reported, unqualified) =
        connected.expect("connect with an existing URI schema must succeed");
    assert_eq!(reported, Some(schema));
    let rows: usize = unqualified
        .expect("an unqualified SELECT must resolve against the URI schema")
        .iter()
        .map(|batch| batch.num_rows())
        .sum();
    assert_eq!(rows, 2);
}

/// A URI schema that does not exist fails the connect with an error that
/// names the schema and carries the server's message.
pub async fn check_missing_uri_schema_fails_the_connect(transport: &str) {
    let missing = generate_unique_test_name("ZZ_MISSING").to_uppercase();

    let error = match connect_with_uri_schema(transport, &missing).await {
        Ok(_) => panic!("a missing URI schema must fail the connect"),
        Err(error) => error.to_string(),
    };

    assert!(error.contains(&missing), "got: {error}");
    assert!(error.contains("not found"), "got: {error}");
}

/// The server opens the URI schema by exact name first, then by its
/// upper-case form, and `current_schema()` reports the name it opened.
pub async fn check_uri_schema_case_rule(transport: &str) {
    let suffix = generate_unique_test_name("").to_lowercase();
    let mixed = format!("ZZ_MixedCase{suffix}");
    let hyphen = format!("zz-hyphen{suffix}");
    let upper = format!("ZZ_UPPER{suffix}");
    let both_lower = format!("zz_both{suffix}");
    let both_upper = both_lower.to_uppercase();
    let schemas = [
        mixed.as_str(),
        hyphen.as_str(),
        upper.as_str(),
        both_lower.as_str(),
        both_upper.as_str(),
    ];
    let mut admin = connect_without_schema(transport).await;
    create_schemas(&mut admin, &schemas).await;

    let observed = [
        opened_schema(transport, &mixed).await,
        opened_schema(transport, &hyphen).await,
        opened_schema(transport, &upper.to_lowercase()).await,
        opened_schema(transport, &both_lower).await,
    ];
    let lowered_mixed = mixed.to_lowercase();
    let lowered_mixed_connect = connect_with_uri_schema(transport, &lowered_mixed).await;

    drop_schemas(&mut admin, &schemas).await;
    admin.close().await.expect("admin close must succeed");

    let opened = |name: &str| -> Result<(Option<String>, Option<String>), String> {
        Ok((Some(name.to_string()), Some(name.to_string())))
    };
    assert_eq!(
        observed,
        [
            opened(&mixed),
            opened(&hyphen),
            opened(&upper),
            opened(&both_lower)
        ]
    );
    let error = match lowered_mixed_connect {
        Ok(_) => panic!("a URI schema that matches neither exactly nor in upper case must fail"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains(&lowered_mixed), "got: {error}");
}

/// `set_schema` sets the server's current schema and reports the name the
/// server opened; a rejected name leaves the current schema unchanged.
pub async fn check_set_schema(transport: &str) {
    let schema = generate_unique_test_name("ZZ_SET_SCHEMA").to_uppercase();
    let missing = generate_unique_test_name("ZZ_MISSING").to_uppercase();
    let mut admin = connect_without_schema(transport).await;
    create_schemas(&mut admin, &[&schema]).await;

    let mut conn = connect_without_schema(transport).await;
    let accepted = conn.set_schema(schema.to_lowercase()).await;
    let after_accept = (
        conn.current_schema().await,
        server_current_schema(&mut conn).await,
    );
    let rejected = conn.set_schema(missing.as_str()).await;
    let after_reject = (
        conn.current_schema().await,
        server_current_schema(&mut conn).await,
    );
    conn.close().await.expect("close must succeed");

    drop_schemas(&mut admin, &[&schema]).await;
    admin.close().await.expect("admin close must succeed");

    accepted.expect("an existing schema must be accepted");
    let opened = (Some(schema.clone()), Some(schema.clone()));
    assert_eq!(after_accept, opened);
    let error = rejected
        .expect_err("a missing schema must be rejected")
        .to_string();
    assert!(error.contains(&missing), "got: {error}");
    assert!(error.contains("not found"), "got: {error}");
    assert_eq!(after_reject, opened);
}

/// `current_schema()` follows `CREATE SCHEMA`, `OPEN SCHEMA`, `CLOSE SCHEMA`,
/// and `DROP SCHEMA` run as SQL.
pub async fn check_current_schema_follows_sql(transport: &str) {
    let first = generate_unique_test_name("ZZ_SQL_FIRST").to_uppercase();
    let second = generate_unique_test_name("ZZ_SQL_SECOND").to_uppercase();
    let statements = [
        format!("CREATE SCHEMA {first}"),
        format!("CREATE SCHEMA {second}"),
        format!("OPEN SCHEMA {first}"),
        "CLOSE SCHEMA".to_string(),
        format!("OPEN SCHEMA {second}"),
        format!("DROP SCHEMA {second} CASCADE"),
    ];
    let mut conn = connect_without_schema(transport).await;

    let mut observed = Vec::new();
    for sql in &statements {
        let outcome = conn.execute_update(sql.as_str()).await.map(|_| ());
        observed.push((sql.clone(), outcome.is_ok(), conn.current_schema().await));
    }

    drop_schemas(&mut conn, &[&first, &second]).await;
    conn.close().await.expect("close must succeed");

    let expected_schemas = [
        Some(first.clone()),
        Some(second.clone()),
        Some(first.clone()),
        None,
        Some(second.clone()),
        None,
    ];
    let expected: Vec<_> = statements
        .iter()
        .zip(expected_schemas)
        .map(|(sql, schema)| (sql.clone(), true, schema))
        .collect();
    assert_eq!(observed, expected);
}
