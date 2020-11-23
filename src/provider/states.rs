pub(crate) mod download_package;
pub(crate) mod install_package;
pub(crate) mod create_config;
pub(crate) mod waiting_config;
pub(crate) mod create_service;
pub(crate) mod download_package_backoff;
pub(crate) mod setup_failed;
pub(crate) mod starting;
pub(crate) mod running;
pub(crate) mod stopping;
pub(crate) mod stopped;
pub(crate) mod failed;
pub(crate) mod terminated;


/// When called in a state's `next` function, exits the current state
/// and transitions to the Error state.
#[macro_export]
macro_rules! transition_to_error {
    ($slf:ident, $err:ident) => {{
        let aerr = anyhow::Error::from($err);
        log::error!("{:?}", aerr);
        let error_state = super::error::Error {
            message: aerr.to_string(),
        };
        return Transition::next($slf, error_state);
    }};
}

/// When called in a state's `next` function, exits the state machine
/// returns a fatal error to the kubelet.
#[macro_export]
macro_rules! fail_fatal {
    ($err:ident) => {{
        let aerr = anyhow::Error::from($err);
        log::error!("{:?}", aerr);
        return Transition::Complete(Err(aerr));
    }};
}
