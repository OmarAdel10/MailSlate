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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every subscriber receives every event, in publish order.
    #[tokio::test]
    async fn events_reach_every_subscriber_in_publish_order() {
        let bus = EventBus::new();
        let mut first = bus.subscribe();
        let mut second = bus.subscribe();

        for step in 0..3 {
            bus.publish(Event::new("sync_progress", json!({ "step": step })));
        }

        for step in 0..3 {
            assert_eq!(
                first.recv().await.expect("first subscriber receives").data["step"],
                step
            );
            assert_eq!(
                second
                    .recv()
                    .await
                    .expect("second subscriber receives")
                    .data["step"],
                step
            );
        }
    }

    /// A subscriber that falls more than [`BUS_CAPACITY`] events behind
    /// is told it lagged — the bus never blocks publishers and never
    /// grows the queue without bound.
    #[tokio::test]
    async fn a_stalled_subscriber_lags_instead_of_blocking_the_bus() {
        let bus = EventBus::new();
        let mut slow = bus.subscribe();

        for _ in 0..=BUS_CAPACITY {
            bus.publish(Event::new("mail_new", json!({})));
        }

        assert!(matches!(
            slow.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
        ));
    }

    /// Publishing with nobody listening is a silent no-op, and a later
    /// subscriber sees only events from its own subscription onward.
    #[tokio::test]
    async fn publishing_without_subscribers_is_a_silent_no_op() {
        let bus = EventBus::new();

        bus.publish(Event::new("mail_new", json!({ "gone": true })));
        let mut late = bus.subscribe();
        bus.publish(Event::new("sync_progress", json!({ "p": 2 })));

        let received = late.recv().await.expect("only the post-subscription event");
        assert_eq!(received.event, "sync_progress");
        assert_eq!(received.data["p"], 2);
    }
}
