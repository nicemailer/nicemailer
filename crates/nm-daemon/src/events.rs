//! Event stream: epoch, sequence numbers and replay buffer (`evenements.md`).
//!
//! Every change committed to the database produces events, numbered with a
//! sequence `seq` that increases by one each time. Events are:
//! - kept in a bounded in-memory buffer, so that a client that reconnects
//!   can ask for the events it missed (`subscribe` with `since`);
//! - broadcast to every subscribed client through a tokio channel.
//!
//! Each event is serialized to JSON once, in `push`, and the same string is
//! shared by all clients through an `Arc` instead of being re-serialized
//! for each of them.
//!
//! Rust note: `Arc<T>` ("atomically reference counted") is a shared pointer.
//! Cloning an `Arc` does not copy the data: it increments a counter, and the
//! data is freed when the last clone is dropped. It is safe to share
//! between threads.

use std::collections::VecDeque;
use std::sync::Arc;

use nm_api::{Event, Notification, Position};
use nm_store::ChangeSet;
use tokio::sync::broadcast;

/// The event log of the core. There is a single instance, behind a mutex.
pub struct EventLog {
    /// Random identifier of this run of the core.
    epoch: String,
    /// Sequence number of the last event emitted (0 = none yet).
    seq: u64,
    /// Maximum number of events kept for replay.
    capacity: usize,
    /// Most recent events, oldest first: `(seq, JSON line)`. A `VecDeque` is
    /// a double-ended queue: cheap to push at the back and pop at the front.
    buffer: VecDeque<(u64, Arc<String>)>,
    /// Sending side of the broadcast channel. Each subscriber holds a
    /// `Receiver` obtained with `sender.subscribe()`.
    sender: broadcast::Sender<(u64, Arc<String>)>,
}

/// Result of a subscription: what the client must receive first, before
/// the live events.
pub enum Replay {
    /// The events the client missed (possibly none if it is up to date).
    Events(Vec<Arc<String>>),
    /// The missed events are no longer available (too old, or the core
    /// restarted): the client must reload everything.
    Resync,
    /// The client did not give a position: nothing to replay.
    None,
}

impl EventLog {
    /// Creates the log. `capacity` is the size of the replay buffer;
    /// `client_queue` is how many events a slow client may lag behind before
    /// it is told to resync.
    pub fn new(capacity: usize, client_queue: usize) -> EventLog {
        // 48 random bits, written as 12 hexadecimal digits.
        let epoch = format!("e_{:012x}", rand::random::<u64>() & 0xffff_ffff_ffff);
        // `broadcast::channel` returns a sender and a first receiver; that
        // receiver is not needed (subscribers create their own), so it is
        // discarded with `_`.
        let (sender, _) = broadcast::channel(client_queue);
        EventLog { epoch, seq: 0, capacity, buffer: VecDeque::new(), sender }
    }

    /// Current position: the last event emitted.
    pub fn position(&self) -> Position {
        Position { epoch: self.epoch.clone(), seq: self.seq }
    }

    /// JSON line of a notification, with `epoch` and `seq` added to the
    /// params: `{"jsonrpc":"2.0","method":"FolderChanged","params":{"folder_id":3,"epoch":"e_…","seq":7}}`.
    ///
    /// The event is first converted into a generic JSON `Value`, so that two
    /// fields can be added to its params before it is turned into text.
    pub fn encode(event: &Event, epoch: &str, seq: u64) -> String {
        // `expect` panics with the given message on error. Serializing these
        // simple types cannot fail, so a panic would reveal a programming bug.
        let mut value = serde_json::to_value(event).expect("événement sérialisable");
        let method = value["method"].as_str().unwrap_or_default().to_string();
        // `take` moves the params out of `value`, leaving `null` behind.
        // Variants without fields may serialize their params differently, so
        // anything that is not an object is replaced by `{}`.
        let mut params = value
            .get_mut("params")
            .map(|p| p.take())
            .filter(|p| p.is_object())
            .unwrap_or_else(|| serde_json::json!({}));
        params["epoch"] = epoch.into();
        params["seq"] = seq.into();
        serde_json::to_string(&Notification { jsonrpc: "2.0".into(), method, params }).unwrap()
    }

    /// `ResyncRequired` line at the current position. It is not stored in the
    /// buffer and does not consume a sequence number: it is addressed to one
    /// client only.
    pub fn resync_line(&self) -> Arc<String> {
        Arc::new(Self::encode(&Event::ResyncRequired {}, &self.epoch, self.seq))
    }

    /// Numbers, stores and broadcasts one event. Public for events that do
    /// not come from a `ChangeSet`, such as import progress.
    pub fn push(&mut self, event: Event) {
        self.seq += 1;
        let line = Arc::new(Self::encode(&event, &self.epoch, self.seq));
        // Drop the oldest event when the buffer is full.
        if self.buffer.len() == self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back((self.seq, line.clone()));
        // `send` fails only when there is no subscriber at all, which is a
        // normal situation (no client connected): the error is ignored.
        // `let _ =` explicitly discards a value the compiler would otherwise
        // warn about.
        let _ = self.sender.send((self.seq, line));
    }

    /// Emits the events of a change committed to the database: messages
    /// first, then threads, then folders, then accounts.
    pub fn emit(&mut self, changes: &ChangeSet) {
        for (message_id, thread_id) in &changes.messages {
            self.push(Event::MessageChanged { message_id: *message_id, thread_id: *thread_id });
        }
        for (thread_id, folder_ids) in &changes.threads {
            self.push(Event::ThreadChanged { thread_id: *thread_id, folder_ids: folder_ids.clone() });
        }
        for folder_id in &changes.folders {
            self.push(Event::FolderChanged { folder_id: *folder_id });
        }
        for account_id in &changes.accounts {
            self.push(Event::AccountChanged { account_id: *account_id });
        }
        // After the folders: a client reloads the list, then notifies.
        for (message_id, folder_id) in &changes.arrived {
            self.push(Event::MessageArrived { message_id: *message_id, folder_id: *folder_id });
        }
    }

    /// Broadcasts a transient event (`SyncProgress`, `evenements.md`): no
    /// sequence number (`seq` is omitted), not kept for replay. A client
    /// that misses one loses nothing that a query would not give back.
    pub fn push_transient(&mut self, event: Event) {
        let mut line = Self::encode(&event, &self.epoch, 0);
        // `encode` always adds a seq: remove it, transient events have none.
        if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&line)
            && let Some(p) = v.get_mut("params").and_then(|p| p.as_object_mut())
        {
            p.remove("seq");
            line = v.to_string();
        }
        let _ = self.sender.send((0, Arc::new(line)));
    }

    /// Subscribes a client and works out what it must receive first.
    ///
    /// Must be called while holding the lock on the log: then no event can
    /// be emitted between computing the replay and creating the receiver,
    /// so the client sees neither a gap nor a duplicate.
    ///
    /// Rust note: `Option<&Position>` is an optional *reference*: the
    /// function only looks at the position, it does not take it.
    pub fn subscribe(&self, since: Option<&Position>) -> (Replay, broadcast::Receiver<(u64, Arc<String>)>) {
        let rx = self.sender.subscribe();
        let replay = match since {
            None => Replay::None,
            // Same epoch, same seq: the client is up to date.
            Some(p) if p.epoch == self.epoch && p.seq == self.seq => Replay::Events(vec![]),
            // Same epoch, behind: replay if every missed event is still in
            // the buffer, i.e. if the first missed one (p.seq + 1) is not
            // older than the oldest kept.
            Some(p) if p.epoch == self.epoch && p.seq < self.seq => {
                let oldest = self.buffer.front().map(|(s, _)| *s).unwrap_or(self.seq + 1);
                if p.seq + 1 >= oldest {
                    Replay::Events(
                        self.buffer.iter().filter(|(s, _)| *s > p.seq).map(|(_, l)| l.clone()).collect(),
                    )
                } else {
                    Replay::Resync
                }
            }
            // Other epoch (the core restarted), or a seq in the future.
            Some(_) => Replay::Resync,
        };
        (replay, rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change touching folder 1 only, i.e. exactly one event.
    fn change() -> ChangeSet {
        let mut c = ChangeSet::default();
        c.folders.insert(1);
        c
    }

    /// A client two events behind gets those two events.
    #[test]
    fn replay_within_buffer() {
        let mut log = EventLog::new(10, 10);
        log.emit(&change());
        let pos = log.position();
        log.emit(&change());
        log.emit(&change());
        // `.0` is the first element of the returned tuple.
        match log.subscribe(Some(&pos)).0 {
            Replay::Events(e) => assert_eq!(e.len(), 2),
            _ => panic!("expected a replay"),
        }
    }

    /// A client too far behind, or from another epoch, must resync.
    #[test]
    fn resync_when_out_of_buffer_or_other_epoch() {
        let mut log = EventLog::new(2, 10);
        let start = log.position();
        for _ in 0..5 {
            log.emit(&change());
        }
        assert!(matches!(log.subscribe(Some(&start)).0, Replay::Resync));
        let other = Position { epoch: "e_other".into(), seq: 5 };
        assert!(matches!(log.subscribe(Some(&other)).0, Replay::Resync));
    }

    /// The JSON line has the notification shape, with the position in params.
    #[test]
    fn notification_carries_position() {
        let line = EventLog::encode(&Event::FolderChanged { folder_id: 3 }, "e_1", 7);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["method"], "FolderChanged");
        assert_eq!(v["params"]["folder_id"], 3);
        assert_eq!(v["params"]["seq"], 7);
    }

    /// A transient event goes to the subscribers, without a sequence number
    /// and without taking one, and is never replayed.
    #[test]
    fn transient_events_have_no_seq_and_are_not_replayed() {
        let mut log = EventLog::new(10, 10);
        let start = log.position();
        let (_, mut rx) = log.subscribe(None);
        log.push_transient(Event::SyncProgress { account_id: 1, folder_id: None, stage: "bodies".into(), done: 1, total: 2 });
        assert_eq!(log.position().seq, start.seq);
        let (_, line) = rx.try_recv().unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["method"], "SyncProgress");
        assert!(v["params"].get("seq").is_none());
        assert_eq!(v["params"]["done"], 1);
        assert!(matches!(log.subscribe(Some(&start)).0, Replay::Events(e) if e.is_empty()));
    }

    /// Import events carry the status fields directly in `params`, next to
    /// the position, like every other notification.
    #[test]
    fn import_progress_is_a_flat_notification() {
        let status = nm_api::ImportStatus { job_id: 2, imported: 500, total_bytes: 9, running: true, ..Default::default() };
        let line = EventLog::encode(&Event::ImportProgress(status), "e_1", 8);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["method"], "ImportProgress");
        assert_eq!(v["params"]["job_id"], 2);
        assert_eq!(v["params"]["imported"], 500);
        assert_eq!(v["params"]["running"], true);
        assert_eq!(v["params"]["seq"], 8);
        // `error` is omitted while there is none.
        assert!(v["params"].get("error").is_none());
    }
}
