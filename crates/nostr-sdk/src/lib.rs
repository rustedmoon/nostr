// Copyright (c) 2022-2023 Yuki Kishimoto
// Copyright (c) 2023-2024 Rust Nostr Developers
// Distributed under the MIT software license

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::bare_urls)]
#![allow(unknown_lints)]
#![allow(clippy::arc_with_non_send_sync)]

//! High level Nostr client library.

#![cfg_attr(
    feature = "all-nips",
    doc = include_str!("../README.md")
)]

#[cfg(all(target_arch = "wasm32", feature = "blocking"))]
compile_error!("`blocking` feature can't be enabled for WASM targets");

pub use nostr::{self, *};
pub use nostr_database::{self as database, NostrDatabase, NostrDatabaseExt, Profile};
#[cfg(all(target_arch = "wasm32", feature = "indexeddb"))]
pub use nostr_indexeddb::{IndexedDBError, WebDatabase};
#[cfg(feature = "sqlite")]
pub use nostr_sqlite::{Error as SQLiteError, SQLiteDatabase};
#[cfg(feature = "blocking")]
use once_cell::sync::Lazy;
#[cfg(feature = "blocking")]
use tokio::runtime::Runtime;

pub mod client;
pub mod prelude;
pub mod relay;
pub mod util;

#[cfg(feature = "blocking")]
pub use self::client::blocking;
pub use self::client::{Client, ClientBuilder, ClientSigner, Options};
pub use self::relay::{
    ActiveSubscription, FilterOptions, InternalSubscriptionId, NegentropyOptions, Relay,
    RelayConnectionStats, RelayOptions, RelayPoolNotification, RelayPoolOptions, RelaySendOptions,
    RelayStatus,
};

#[cfg(feature = "blocking")]
static RUNTIME: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("Can't start Tokio runtime"));

#[allow(missing_docs)]
#[cfg(feature = "blocking")]
pub fn block_on<F>(future: F) -> F::Output
where
    F: core::future::Future,
{
    RUNTIME.block_on(future)
}
