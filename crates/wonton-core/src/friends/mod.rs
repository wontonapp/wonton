//! Accepted Matrix friends and groups, persisted in `<data>/core/friends.json` (ADR-0051).
//!
//! The file travels with the core data directory's existing backup/restore. Removed identities
//! remain as tombstones so their plugin usernames can never identify another person. Share grants
//! are deliberately not stored here: the sharing registry remains their only source of truth.
//!
//! Authorization consumers must `subscribe()` **before** their first check, inspect that receiver's
//! current snapshot, and recheck the latest complete snapshot after `changed()`. Watch coalesces
//! changes without losing the final registry (unlike a bounded event broadcast). Each durable
//! meaningful mutation advances the persisted revision, including deletions and group changes.
//! Bind grants and authenticated connections to `FriendRecord::access_generation`, and compare
//! it on access: a coalesced remove/reaccept remains distinguishable from the previous activation.

mod persistence;
mod store;
mod types;

pub use store::Friends;
pub use types::{FriendRecord, FriendStatus, FriendsError, Snapshot};

#[cfg(test)]
mod tests;
