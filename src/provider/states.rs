pub(crate) mod create_service;
pub(crate) mod creating_config;
pub(crate) mod downloading;
pub(crate) mod downloading_backoff;
pub(crate) mod failed;
pub(crate) mod installing;
pub(crate) mod running;
pub(crate) mod setup_failed;
pub(crate) mod starting;
pub(crate) mod stopped;
pub(crate) mod stopping;
pub(crate) mod terminated;
pub(crate) mod waiting_config_map;

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
