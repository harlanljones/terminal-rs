//! C1′ primitives: fetch keys, provenance, freshness, source policy, and the raw payload store.

mod key;
mod policy;
mod provenance;
mod store;
mod transactions;

pub mod bundled;
pub mod catalog;
pub mod savant;
pub mod sources;
pub mod track;
pub mod xdg;

pub use key::{Fetch, FetchKey, Role};
pub use policy::Policy;
pub use provenance::{Attribution, Provenance};
pub use store::{Freshness, Loaded, RawStore, StoreError, StoredPayload};
pub use track::{AlertRow, TrackError, TrackStore};
pub use transactions::{Transaction, TransactionPage, TransactionsError, normalize};

/// Crate-wide error type for the store layer.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("bad fetch key: {0}")]
    BadKey(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
