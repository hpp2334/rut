//! The actor — the machinery shell around the session. ONE
//! long-lived event loop ([`event_loop`]) owns the session state
//! exclusively and answers the bridges' messages FIFO; [`wire`] is
//! the one place that spells every envelope's field names. The shell
//! is business-blind: it routes on the machinery tier only, never
//! inspects a command.

pub(crate) mod event_loop;
pub(crate) mod wire;
