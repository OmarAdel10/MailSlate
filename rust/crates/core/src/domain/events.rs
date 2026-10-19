//! Core event bus shared by sync, OAuth, and account tasks (ARCHITECTURE §9).
//!
//! Sync, OAuth, and mutation tasks publish [`Event`]s here; every live
//! IPC connection carries a subscriber so the GUI receives unsolicited
//! updates (`mail_new`, `sync_progress`, `account_status`, …) over its
//! existing connection.
//!
//! Underlying primitive is `tokio::sync::broadcast`: a subscriber that
//! starts after a publish does not replay it, and a lagging subscriber
//! loses the oldest events to `RecvError::Lagged` rather than growing
//! the queue without bound.

use std::sync::Arc;

use mailslate_protocol::message::Event;

/// Broadcast capacity. Events are small ("did numbers/docIDs change"
/// sized); 1024 gives a stuck GUI several seconds of headroom at M2
/// eventRates before lag drops oldest entries.
const BUS_CAPACITY: usize = 1024;

/// Fan-out hub for daemon→GUI events.
#[derive(Debug)]
pub struct EventBus {
    tx: tokio::sync::broadcast::Sender<Event>,
}

impl EventBus {
    /// Creates the shared bus. Publishes with no subscribers are
    /// allowed and silently dropped — principled, it means no GUI
    /// connection is interested right now.
    pub fn new() -> Arc<Self> {
        let (tx, _) = tokio::sync::broadcast::channel(BUS_CAPACITY);
        Arc::new(Self { tx })
    }

    /// Subscribes for all future events from now on.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Publishes an event to all current subscribers. No subscribers
    /// is not an error (`send` returns Err with the event back).
    ///
    /// `allow(dead_code)`: producers (OAuth, sync engine) land with
    /// phases 4 and 6; the bus itself ships ahead of them.
    #[allow(dead_code)]
    pub fn publish(&self, event: Event) {
        let _ = self.tx.send(event);
    }
}

impl Default for EventBus {
    fn default() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(BUS_CAPACITY);
        Self { tx }
    }
}
