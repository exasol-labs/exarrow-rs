//! One deadline shared by the ordered steps of a connection or tunnel setup.
//!
//! The steps run under one budget, so time spent in one step is not granted
//! again to a later one. The module that runs the steps names the step in the
//! timeout error, because a caller that wraps the whole setup in one timer sees
//! a single future and cannot tell a stalled TLS handshake from a stalled login.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;

use crate::error::TransportError;

/// Mirrors the fallback of `tokio::time::timeout` for a budget that overflows
/// the clock: about 30 years, which no setup outlives.
const FAR_FUTURE: Duration = Duration::from_secs(86_400 * 365 * 30);

/// A setup step that a [`SetupDeadline`] names when it runs out of time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetupStep {
    TcpConnect,
    TlsHandshake,
    #[cfg(feature = "websocket")]
    WebSocketUpgrade,
    Login,
    ExaHandshake,
}

impl fmt::Display for SetupStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            SetupStep::TcpConnect => "TCP connect",
            SetupStep::TlsHandshake => "TLS handshake",
            #[cfg(feature = "websocket")]
            SetupStep::WebSocketUpgrade => "WebSocket upgrade",
            SetupStep::Login => "login",
            SetupStep::ExaHandshake => "EXA handshake",
        };
        f.write_str(name)
    }
}

/// One budget for an ordered sequence of async setup steps.
///
/// It is `Copy`, so a transport can start it in `connect()`, store it, and
/// spend what remains of it in `authenticate()`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SetupDeadline {
    deadline: Instant,
    budget: Duration,
    label: &'static str,
}

impl SetupDeadline {
    /// Starts the budget now. `label` opens the timeout message, for example
    /// `Connection` or `HTTP tunnel setup`.
    pub(crate) fn start(label: &'static str, budget: Duration) -> Self {
        let now = Instant::now();
        let deadline = now.checked_add(budget).unwrap_or_else(|| now + FAR_FUTURE);
        Self {
            deadline,
            budget,
            label,
        }
    }

    /// Runs `step` until it finishes or the deadline passes, whichever comes
    /// first. A step that finishes returns its own result unchanged; a step
    /// that runs out of time returns `<label> timeout after <budget>ms (<step>)`.
    pub(crate) async fn run<T>(
        &self,
        step: SetupStep,
        future: impl Future<Output = Result<T, TransportError>>,
    ) -> Result<T, TransportError> {
        tokio::time::timeout_at(self.deadline, future)
            .await
            .unwrap_or_else(|_| Err(self.timeout_error(step)))
    }

    /// Reports whether the deadline has passed, so a caller can tell a step
    /// that ran out of time from one that failed on its own.
    pub(crate) fn has_elapsed(&self) -> bool {
        Instant::now() >= self.deadline
    }

    fn timeout_error(&self, step: SetupStep) -> TransportError {
        TransportError::IoError(format!(
            "{} timeout after {}ms ({step})",
            self.label,
            self.budget.as_millis()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never_finishes() -> impl Future<Output = Result<(), TransportError>> {
        std::future::pending()
    }

    /// Scenario: One deadline bounds every connection setup step
    #[tokio::test(start_paused = true)]
    async fn a_later_step_gets_only_the_time_left_by_earlier_steps() {
        let started = Instant::now();
        let deadline = SetupDeadline::start("Connection", Duration::from_millis(1000));

        deadline
            .run(SetupStep::TcpConnect, async {
                tokio::time::sleep(Duration::from_millis(600)).await;
                Ok(())
            })
            .await
            .expect("a step that finishes in time returns its own result");
        let error = deadline
            .run(SetupStep::TlsHandshake, never_finishes())
            .await
            .expect_err("a step that never finishes must run out of time");

        assert_eq!(started.elapsed(), Duration::from_millis(1000));
        assert!(error.to_string().contains("(TLS handshake)"), "{error}");
    }

    /// Scenario: One deadline bounds every connection setup step
    /// Scenario: Tunnel setup fails with the step named when the peer stops answering
    #[tokio::test(start_paused = true)]
    async fn an_elapsed_deadline_names_the_label_the_budget_and_the_step() {
        let connection = SetupDeadline::start("Connection", Duration::from_millis(3000));
        let tunnel = SetupDeadline::start("HTTP tunnel setup", Duration::from_millis(300));

        let connection_error = connection
            .run(SetupStep::TlsHandshake, never_finishes())
            .await
            .expect_err("the connection deadline must elapse");
        let tunnel_error = tunnel
            .run(SetupStep::ExaHandshake, never_finishes())
            .await
            .expect_err("the tunnel deadline must elapse");

        assert_eq!(
            connection_error.to_string(),
            "Network I/O error: Connection timeout after 3000ms (TLS handshake)"
        );
        assert!(
            tunnel_error
                .to_string()
                .contains("HTTP tunnel setup timeout after 300ms (EXA handshake)"),
            "{tunnel_error}"
        );
    }

    /// Scenario: One deadline bounds every connection setup step
    #[tokio::test(start_paused = true)]
    async fn a_step_error_before_the_deadline_is_returned_unchanged() {
        let deadline = SetupDeadline::start("Connection", Duration::from_millis(1000));

        let error = deadline
            .run(SetupStep::TlsHandshake, async {
                Err::<(), _>(TransportError::TlsError("bad certificate".to_string()))
            })
            .await
            .expect_err("the step's own error must reach the caller");

        assert!(
            matches!(&error, TransportError::TlsError(message) if message == "bad certificate"),
            "{error:?}"
        );
        assert!(!deadline.has_elapsed());
    }

    #[tokio::test(start_paused = true)]
    async fn a_budget_that_overflows_the_clock_does_not_panic() {
        let deadline = SetupDeadline::start("HTTP tunnel setup", Duration::MAX);

        let outcome = deadline
            .run(SetupStep::TcpConnect, async { Ok(42) })
            .await
            .expect("a step that finishes at once returns its own result");

        assert_eq!(outcome, 42);
    }
}
