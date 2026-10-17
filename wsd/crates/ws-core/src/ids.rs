//! ULIDs identify sessions, events, and devices.

pub use ulid::Ulid;

pub type SessionId = Ulid;
pub type EventId = Ulid;
pub type DeviceId = Ulid;
