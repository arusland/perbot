//! Interactive "send me the reminder text" flow for time-only messages.
//!
//! When a user sends only a time expression (e.g. `13:30`) with no reminder body,
//! the bot asks for the text and shows a Cancel button. The parsed (body-less)
//! event is held per-chat until the next text message supplies the body, mirroring
//! the in-memory [`crate::import::PendingImport`] pattern. State is in-memory only;
//! a restart simply drops pending requests. The prompt strings and the Cancel
//! keyboard the flows show live in `crate::view`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::types::EventInfo;

/// Per-chat events awaiting a reminder body, keyed by chat id. The stored
/// [`EventInfo`] has its time/recurrence fields set and an empty `message`;
/// `chat_id`/`msg_id`/`message` are filled in when the body arrives.
pub type PendingMessage = Arc<Mutex<HashMap<i64, EventInfo>>>;

pub fn new_pending() -> PendingMessage {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Per-chat events being edited, keyed by chat id; the value is the id of the
/// event whose time and message the next message will replace. Set when the user
/// taps Edit on the `/event<id>` view and cleared when the edit completes or is
/// cancelled. In-memory only, like [`PendingMessage`].
pub type PendingEdit = Arc<Mutex<HashMap<i64, i64>>>;

pub fn new_pending_edit() -> PendingEdit {
    Arc::new(Mutex::new(HashMap::new()))
}
