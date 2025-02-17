// Copyright (c) 2022-2023 Yuki Kishimoto
// Copyright (c) 2023-2024 Rust Nostr Developers
// Distributed under the MIT software license

//! Client Options

#[cfg(not(target_arch = "wasm32"))]
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::relay::RelayPoolOptions;

pub(crate) const DEFAULT_SEND_TIMEOUT: Duration = Duration::from_secs(20);

/// Options
#[derive(Debug, Clone)]
pub struct Options {
    /// Wait for the msg to be sent (default: true)
    wait_for_send: Arc<AtomicBool>,
    /// Wait for the subscription msg to be sent (default: false)
    wait_for_subscription: Arc<AtomicBool>,
    /// POW difficulty for all events (default: 0)
    difficulty: Arc<AtomicU8>,
    /// REQ filters chunk size (default: 10)
    req_filters_chunk_size: Arc<AtomicU8>,
    /// Skip disconnected relays during send methods (default: true)
    ///
    /// If the relay made just 1 attempt, the relay will not be skipped
    skip_disconnected_relays: Arc<AtomicBool>,
    /// Timeout (default: 60)
    ///
    /// Used in `get_events_of`, `req_events_of` and similar as default timeout.
    pub timeout: Duration,
    /// Relay connection timeout (default: None)
    ///
    /// If set to `None`, the client will try to connect to relay without waiting.
    pub connection_timeout: Option<Duration>,
    /// Send timeout (default: 20 secs)
    pub send_timeout: Option<Duration>,
    /// NIP46 timeout (default: 180 secs)
    #[cfg(feature = "nip46")]
    pub nip46_timeout: Option<Duration>,
    /// Proxy
    #[cfg(not(target_arch = "wasm32"))]
    pub proxy: Option<SocketAddr>,
    /// Shutdown on [Client](super::Client) drop
    pub shutdown_on_drop: bool,
    /// Pool Options
    pub pool: RelayPoolOptions,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            wait_for_send: Arc::new(AtomicBool::new(true)),
            wait_for_subscription: Arc::new(AtomicBool::new(false)),
            difficulty: Arc::new(AtomicU8::new(0)),
            req_filters_chunk_size: Arc::new(AtomicU8::new(10)),
            skip_disconnected_relays: Arc::new(AtomicBool::new(true)),
            timeout: Duration::from_secs(60),
            connection_timeout: None,
            send_timeout: Some(DEFAULT_SEND_TIMEOUT),
            #[cfg(feature = "nip46")]
            nip46_timeout: Some(Duration::from_secs(180)),
            #[cfg(not(target_arch = "wasm32"))]
            proxy: None,
            shutdown_on_drop: false,
            pool: RelayPoolOptions::default(),
        }
    }
}

impl Options {
    /// Create new (default) [`Options`]
    pub fn new() -> Self {
        Self::default()
    }

    /// If set to `true`, `Client` wait that `Relay` try at least one time to enstablish a connection before continue.
    #[deprecated(since = "0.27.0", note = "Use `connection_timeout` instead")]
    pub fn wait_for_connection(self, _wait: bool) -> Self {
        self
    }

    /// If set to `true`, `Client` wait that a message is sent before continue.
    pub fn wait_for_send(self, wait: bool) -> Self {
        Self {
            wait_for_send: Arc::new(AtomicBool::new(wait)),
            ..self
        }
    }

    pub(crate) fn get_wait_for_send(&self) -> bool {
        self.wait_for_send.load(Ordering::SeqCst)
    }

    /// If set to `true`, `Client` wait that a subscription msg is sent before continue (`subscribe` and `unsubscribe` methods)
    pub fn wait_for_subscription(self, wait: bool) -> Self {
        Self {
            wait_for_subscription: Arc::new(AtomicBool::new(wait)),
            ..self
        }
    }

    pub(crate) fn get_wait_for_subscription(&self) -> bool {
        self.wait_for_subscription.load(Ordering::SeqCst)
    }

    /// Set default POW diffficulty for `Event`
    pub fn difficulty(self, difficulty: u8) -> Self {
        Self {
            difficulty: Arc::new(AtomicU8::new(difficulty)),
            ..self
        }
    }

    pub(crate) fn get_difficulty(&self) -> u8 {
        self.difficulty.load(Ordering::SeqCst)
    }

    pub(crate) fn update_difficulty(&self, difficulty: u8) {
        let _ = self
            .difficulty
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |_| Some(difficulty));
    }

    /// Set `REQ` filters chunk size
    pub fn req_filters_chunk_size(self, size: u8) -> Self {
        Self {
            req_filters_chunk_size: Arc::new(AtomicU8::new(size)),
            ..self
        }
    }

    pub(crate) fn get_req_filters_chunk_size(&self) -> usize {
        self.req_filters_chunk_size.load(Ordering::SeqCst) as usize
    }

    /// Skip disconnected relays during send methods (default: true)
    ///
    /// If the relay made just 1 attempt, the relay will not be skipped
    pub fn skip_disconnected_relays(self, skip: bool) -> Self {
        Self {
            skip_disconnected_relays: Arc::new(AtomicBool::new(skip)),
            ..self
        }
    }

    pub(crate) fn get_skip_disconnected_relays(&self) -> bool {
        self.skip_disconnected_relays.load(Ordering::SeqCst)
    }

    /// Set default timeout
    pub fn timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }

    /// Connection timeout (default: None)
    ///
    /// If set to `None`, the client will try to connect to the relays without waiting.
    pub fn connection_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.connection_timeout = timeout;
        self
    }

    /// Set default send timeout
    pub fn send_timeout(self, timeout: Option<Duration>) -> Self {
        Self {
            send_timeout: timeout,
            ..self
        }
    }

    /// Set NIP46 timeout
    #[cfg(feature = "nip46")]
    pub fn nip46_timeout(self, timeout: Option<Duration>) -> Self {
        Self {
            nip46_timeout: timeout,
            ..self
        }
    }

    /// Proxy
    #[cfg(not(target_arch = "wasm32"))]
    pub fn proxy(mut self, proxy: Option<SocketAddr>) -> Self {
        self.proxy = proxy;
        self
    }

    /// Shutdown client on drop
    pub fn shutdown_on_drop(self, value: bool) -> Self {
        Self {
            shutdown_on_drop: value,
            ..self
        }
    }

    /// Set pool options
    pub fn pool(self, opts: RelayPoolOptions) -> Self {
        Self { pool: opts, ..self }
    }
}
