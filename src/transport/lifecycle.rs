//! The connection lifecycle that the native and the WebSocket transports share.
//!
//! This module owns the connection state and every rule for moving between
//! states: the pre-flight guards, the login under what remains of the
//! connection deadline, the graceful close, and the termination. A transport
//! supplies only the four steps of [`LifecycleSteps`] that differ per protocol.

use std::time::Duration;

use async_trait::async_trait;

use super::deadline::{SetupDeadline, SetupStep};
use super::messages::SessionInfo;
use super::protocol::Credentials;
use crate::error::TransportError;

const MUST_CONNECT: &str = "Must connect before authenticating";

/// The connection state of a transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionState {
    Disconnected,
    Connected,
    Authenticated,
    /// Ended by a graceful close or by a login that ran out of time.
    Closed,
    /// Ended by [`terminate`]; every later operation except `close` names the termination.
    Terminated,
}

/// The state of one transport's connection and the deadline its `connect()` started.
#[derive(Debug)]
pub(crate) struct ConnectionLifecycle {
    state: ConnectionState,
    deadline: Option<SetupDeadline>,
}

impl ConnectionLifecycle {
    pub(crate) fn new() -> Self {
        Self {
            state: ConnectionState::Disconnected,
            deadline: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn in_state(state: ConnectionState) -> Self {
        Self {
            state,
            deadline: None,
        }
    }

    pub(crate) fn state(&self) -> ConnectionState {
        self.state
    }

    /// True from a finished `connect()` until the transport is closed or terminated.
    pub(crate) fn is_open(&self) -> bool {
        matches!(
            self.state,
            ConnectionState::Connected | ConnectionState::Authenticated
        )
    }

    /// The pre-flight guard of every operation. A terminated transport returns
    /// the termination instead of `message`, so the caller reconnects.
    pub(crate) fn require(
        &self,
        expected: ConnectionState,
        message: &str,
    ) -> Result<(), TransportError> {
        if self.state == expected {
            Ok(())
        } else if self.state == ConnectionState::Terminated {
            Err(terminated_error())
        } else {
            Err(TransportError::ProtocolError(message.to_string()))
        }
    }

    /// Starts the connection-timeout deadline, which bounds every step of
    /// `connect()` and, once [`connected`](Self::connected) stores it, the login.
    pub(crate) fn begin_connect(&self, timeout_ms: u64) -> Result<SetupDeadline, TransportError> {
        self.require(ConnectionState::Disconnected, "Already connected")?;
        Ok(SetupDeadline::start(
            "Connection",
            Duration::from_millis(timeout_ms),
        ))
    }

    pub(crate) fn connected(&mut self, deadline: SetupDeadline) {
        self.state = ConnectionState::Connected;
        self.deadline = Some(deadline);
    }
}

/// The error of every operation on a terminated transport except `close`.
///
/// The export timeout is the only caller of `terminate()` apart from `close()`,
/// so the text names the export; a new caller of `terminate()` must revise it.
fn terminated_error() -> TransportError {
    TransportError::ProtocolError(
        "Transport was terminated after an export gave up on an in-flight response; \
         reconnect before the next operation"
            .to_string(),
    )
}

/// The steps of the lifecycle that differ per transport. The functions of this
/// module run the state rules around them, so a step keeps no state of its own.
#[async_trait]
pub(crate) trait LifecycleSteps: Send {
    fn lifecycle_mut(&mut self) -> &mut ConnectionLifecycle;

    /// Runs the protocol login on the open connection and stores the session.
    async fn login_exchange(
        &mut self,
        credentials: &Credentials,
    ) -> Result<SessionInfo, TransportError>;

    /// Runs the best-effort disconnect I/O of a graceful close, ignoring its errors.
    async fn send_disconnect(&mut self);

    /// Drops the stream and the session without any I/O.
    fn release_connection(&mut self);
}

/// Releases the connection without I/O and records `Terminated`.
pub(crate) fn terminate(transport: &mut impl LifecycleSteps) {
    transport.release_connection();
    transport.lifecycle_mut().state = ConnectionState::Terminated;
}

/// Runs the login under what remains of the deadline that `connect()` started,
/// which stays stored, so a retried login gets only the time that is left.
///
/// A login that ran out of time left unread bytes on the socket, so it releases
/// the connection and records `Closed`. Any earlier login error keeps `Connected`.
pub(crate) async fn authenticate_within_deadline(
    transport: &mut impl LifecycleSteps,
    credentials: &Credentials,
) -> Result<SessionInfo, TransportError> {
    let lifecycle = transport.lifecycle_mut();
    lifecycle.require(ConnectionState::Connected, MUST_CONNECT)?;
    let deadline = lifecycle
        .deadline
        .ok_or_else(|| TransportError::ProtocolError(MUST_CONNECT.to_string()))?;

    match deadline
        .run(SetupStep::Login, transport.login_exchange(credentials))
        .await
    {
        Ok(session) => {
            transport.lifecycle_mut().state = ConnectionState::Authenticated;
            Ok(session)
        }
        Err(error) => {
            if deadline.has_elapsed() {
                terminate(transport);
                transport.lifecycle_mut().state = ConnectionState::Closed;
            }
            Err(error)
        }
    }
}

/// Closes an open transport with its disconnect I/O and the release of
/// [`terminate`], then records `Closed`. A transport that is not open closes
/// without I/O and keeps its state, so a terminated one keeps naming its cause.
pub(crate) async fn close_gracefully(
    transport: &mut impl LifecycleSteps,
) -> Result<(), TransportError> {
    if !transport.lifecycle_mut().is_open() {
        return Ok(());
    }
    transport.send_disconnect().await;
    terminate(transport);
    transport.lifecycle_mut().state = ConnectionState::Closed;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::test_support::{
        assert_names_the_termination, test_credentials, transport_session_info,
    };
    use std::collections::VecDeque;
    use std::time::Duration;
    use tokio::time::Instant;

    const MUST_AUTHENTICATE: &str = "Must authenticate before executing queries";

    enum Login {
        NeverFinishes,
        FailsWithWrongPassword,
        Succeeds,
    }

    /// Runs each login from a script and counts the I/O steps, so the shared
    /// rules can be checked without a socket.
    struct ScriptedSteps {
        lifecycle: ConnectionLifecycle,
        logins: VecDeque<Login>,
        disconnects: usize,
        releases: usize,
    }

    impl ScriptedSteps {
        fn new(lifecycle: ConnectionLifecycle, logins: impl IntoIterator<Item = Login>) -> Self {
            Self {
                lifecycle,
                logins: logins.into_iter().collect(),
                disconnects: 0,
                releases: 0,
            }
        }

        /// A transport whose `connect()` succeeded with `timeout_ms` just now.
        fn connected(timeout_ms: u64, logins: impl IntoIterator<Item = Login>) -> Self {
            let mut lifecycle = ConnectionLifecycle::new();
            let deadline = lifecycle
                .begin_connect(timeout_ms)
                .expect("a new lifecycle may connect");
            lifecycle.connected(deadline);
            Self::new(lifecycle, logins)
        }
    }

    #[async_trait]
    impl LifecycleSteps for ScriptedSteps {
        fn lifecycle_mut(&mut self) -> &mut ConnectionLifecycle {
            &mut self.lifecycle
        }

        async fn login_exchange(
            &mut self,
            _credentials: &Credentials,
        ) -> Result<SessionInfo, TransportError> {
            match self.logins.pop_front().expect("every login is scripted") {
                Login::NeverFinishes => std::future::pending().await,
                Login::FailsWithWrongPassword => Err(TransportError::ProtocolError(
                    "Authentication failed: wrong password".to_string(),
                )),
                Login::Succeeds => Ok(transport_session_info()),
            }
        }

        async fn send_disconnect(&mut self) {
            self.disconnects += 1;
        }

        fn release_connection(&mut self) {
            self.releases += 1;
        }
    }

    /// Scenario: Server that never answers the login
    #[tokio::test(start_paused = true)]
    async fn a_login_that_runs_out_of_time_releases_the_connection_and_records_closed() {
        let mut steps = ScriptedSteps::connected(300, [Login::NeverFinishes]);

        let error = authenticate_within_deadline(&mut steps, &test_credentials())
            .await
            .expect_err("a login that never finishes must run out of time");

        assert!(
            error
                .to_string()
                .contains("Connection timeout after 300ms (login)"),
            "{error}"
        );
        assert_eq!(steps.releases, 1);
        assert_eq!(steps.lifecycle.state(), ConnectionState::Closed);
        assert!(!steps.lifecycle.is_open());
    }

    /// Scenario: One deadline bounds every connection setup step
    #[tokio::test(start_paused = true)]
    async fn a_login_gets_only_the_time_left_after_connect() {
        let started = Instant::now();
        let mut steps = ScriptedSteps::connected(1000, [Login::NeverFinishes]);
        tokio::time::sleep(Duration::from_millis(800)).await;

        let error = authenticate_within_deadline(&mut steps, &test_credentials())
            .await
            .expect_err("a login that never finishes must run out of time");

        assert_eq!(started.elapsed(), Duration::from_millis(1000));
        assert!(error.to_string().contains("(login)"), "{error}");
    }

    /// Scenario: One deadline bounds every connection setup step
    #[tokio::test(start_paused = true)]
    async fn a_login_error_before_the_deadline_is_returned_and_keeps_the_transport_connected() {
        let started = Instant::now();
        let mut steps =
            ScriptedSteps::connected(1000, [Login::FailsWithWrongPassword, Login::NeverFinishes]);

        let error = authenticate_within_deadline(&mut steps, &test_credentials())
            .await
            .expect_err("the scripted login fails");

        assert!(
            matches!(&error, TransportError::ProtocolError(message)
                if message == "Authentication failed: wrong password"),
            "{error:?}"
        );
        assert_eq!(steps.releases, 0);
        assert_eq!(steps.lifecycle.state(), ConnectionState::Connected);

        tokio::time::sleep(Duration::from_millis(500)).await;
        let retry_error = authenticate_within_deadline(&mut steps, &test_credentials())
            .await
            .expect_err("a retried login that never finishes must run out of time");

        assert_eq!(started.elapsed(), Duration::from_millis(1000));
        assert!(
            retry_error
                .to_string()
                .contains("Connection timeout after 1000ms (login)"),
            "{retry_error}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn authenticate_requires_connected_and_records_authenticated_on_success() {
        for lifecycle in [
            ConnectionLifecycle::new(),
            ConnectionLifecycle::in_state(ConnectionState::Connected),
        ] {
            let mut unconnected = ScriptedSteps::new(lifecycle, []);
            let error = authenticate_within_deadline(&mut unconnected, &test_credentials())
                .await
                .expect_err("a login needs a connection and its deadline");
            assert!(
                matches!(&error, TransportError::ProtocolError(message)
                    if message == "Must connect before authenticating"),
                "{error:?}"
            );
        }

        let mut steps = ScriptedSteps::connected(1000, [Login::Succeeds]);
        let session = authenticate_within_deadline(&mut steps, &test_credentials())
            .await
            .expect("the scripted login succeeds");

        assert_eq!(session.session_id, transport_session_info().session_id);
        assert_eq!(steps.lifecycle.state(), ConnectionState::Authenticated);
    }

    /// Scenario: Operations after an export timeout name the termination
    #[tokio::test(start_paused = true)]
    async fn every_guard_names_the_termination_on_a_terminated_transport() {
        let mut steps = ScriptedSteps::new(
            ConnectionLifecycle::in_state(ConnectionState::Authenticated),
            [],
        );
        terminate(&mut steps);

        let errors = [
            steps
                .lifecycle
                .require(ConnectionState::Disconnected, "Already connected"),
            steps.lifecycle.begin_connect(1000).map(|_| ()),
            steps.lifecycle.require(
                ConnectionState::Connected,
                "Must connect before authenticating",
            ),
            steps
                .lifecycle
                .require(ConnectionState::Authenticated, MUST_AUTHENTICATE),
            authenticate_within_deadline(&mut steps, &test_credentials())
                .await
                .map(|_| ()),
        ];

        for outcome in errors {
            let error = outcome.expect_err("a terminated transport refuses every operation");
            assert_names_the_termination(&error);
        }
    }

    /// Scenario: Terminate a connection whose in-flight response is no longer trusted
    #[tokio::test(start_paused = true)]
    async fn terminate_releases_the_connection_without_io_and_records_terminated() {
        let mut steps = ScriptedSteps::new(
            ConnectionLifecycle::in_state(ConnectionState::Authenticated),
            [],
        );

        terminate(&mut steps);

        assert_eq!(steps.releases, 1);
        assert_eq!(steps.disconnects, 0);
        assert_eq!(steps.lifecycle.state(), ConnectionState::Terminated);
        assert!(!steps.lifecycle.is_open());
    }

    /// Scenario: Terminate a connection whose in-flight response is no longer trusted
    /// Scenario: Operations after an export timeout name the termination
    #[tokio::test(start_paused = true)]
    async fn close_after_terminate_succeeds_without_io_and_keeps_reporting_termination() {
        let mut steps = ScriptedSteps::new(
            ConnectionLifecycle::in_state(ConnectionState::Authenticated),
            [],
        );
        terminate(&mut steps);

        close_gracefully(&mut steps)
            .await
            .expect("closing a terminated transport succeeds");

        assert_eq!(steps.disconnects, 0);
        assert_eq!(steps.releases, 1);
        let error = steps
            .lifecycle
            .require(ConnectionState::Authenticated, MUST_AUTHENTICATE)
            .expect_err("a closed terminated transport still refuses queries");
        assert_names_the_termination(&error);
    }

    /// Scenario: Operations after an export timeout name the termination
    #[tokio::test(start_paused = true)]
    async fn close_disconnects_an_open_transport_and_records_closed_not_terminated() {
        let mut open = ScriptedSteps::new(
            ConnectionLifecycle::in_state(ConnectionState::Authenticated),
            [],
        );

        close_gracefully(&mut open)
            .await
            .expect("closing an open transport succeeds");

        assert_eq!(open.disconnects, 1);
        assert_eq!(open.releases, 1);
        assert_eq!(open.lifecycle.state(), ConnectionState::Closed);
        let error = open
            .lifecycle
            .require(ConnectionState::Authenticated, MUST_AUTHENTICATE)
            .expect_err("a closed transport refuses queries");
        assert!(
            matches!(&error, TransportError::ProtocolError(message) if message == MUST_AUTHENTICATE),
            "{error:?}"
        );

        let mut never_connected = ScriptedSteps::new(ConnectionLifecycle::new(), []);
        close_gracefully(&mut never_connected)
            .await
            .expect("closing a transport that never connected succeeds");

        assert_eq!(never_connected.disconnects, 0);
        assert_eq!(never_connected.releases, 0);
    }
}
