// Copyright (c) 2022-2023 Yuki Kishimoto
// Copyright (c) 2023-2024 Rust Nostr Developers
// Distributed under the MIT software license

//! Prelude

#![allow(unknown_lints)]
#![allow(ambiguous_glob_reexports)]

// External crates
#[cfg(feature = "nip06")]
pub use bip39::*;
pub use bitcoin::bech32::*;
pub use bitcoin::hashes::*;
pub use bitcoin::secp256k1::*;
pub use bitcoin::*;
pub use negentropy::*;
pub use serde_json::*;
pub use url_fork::*;

// Internal modules
pub use crate::event::builder::*;
pub use crate::event::id::*;
pub use crate::event::kind::*;
pub use crate::event::tag::*;
pub use crate::event::unsigned::*;
pub use crate::event::*;
pub use crate::key::*;
pub use crate::message::*;
// NIPs
pub use crate::nips::nip01::{self, *};
#[cfg(feature = "nip04")]
pub use crate::nips::nip04::{self, *};
#[cfg(all(feature = "std", feature = "nip05"))]
pub use crate::nips::nip05::{self, *};
#[cfg(feature = "nip06")]
pub use crate::nips::nip06::{self, *};
#[cfg(all(feature = "nip07", target_arch = "wasm32"))]
pub use crate::nips::nip07::{self, *};
#[cfg(all(feature = "std", feature = "nip11"))]
pub use crate::nips::nip11::{self, *};
pub use crate::nips::nip13::{self, *};
pub use crate::nips::nip15::{self, *};
pub use crate::nips::nip19::{self, *};
pub use crate::nips::nip21::{self, *};
pub use crate::nips::nip26::{self, *};
#[cfg(feature = "nip44")]
pub use crate::nips::nip44::{self, *};
#[cfg(all(feature = "std", feature = "nip46"))]
pub use crate::nips::nip46::{self, *};
#[cfg(feature = "nip47")]
pub use crate::nips::nip47::{self, *};
pub use crate::nips::nip48::{self, *};
pub use crate::nips::nip53::{self, *};
#[cfg(feature = "nip57")]
pub use crate::nips::nip57::{self, *};
pub use crate::nips::nip65::{self, *};
pub use crate::nips::nip90::{self, *};
pub use crate::nips::nip94::{self, *};
pub use crate::nips::nip98::{self, *};
pub use crate::types::*;
pub use crate::util::*;
#[cfg(feature = "std")]
pub use crate::{Result, SECP256K1};
