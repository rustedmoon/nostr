// Copyright (c) 2022-2023 Yuki Kishimoto
// Copyright (c) 2023-2024 Rust Nostr Developers
// Distributed under the MIT software license

//! Client

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_utility::thread;
use nostr::event::builder::Error as EventBuilderError;
use nostr::key::XOnlyPublicKey;
#[cfg(feature = "nip46")]
use nostr::nips::nip46::{Request, Response};
use nostr::nips::nip94::FileMetadata;
use nostr::types::metadata::Error as MetadataError;
use nostr::url::Url;
use nostr::util::EventIdOrCoordinate;
use nostr::{
    ClientMessage, Contact, Event, EventBuilder, EventId, Filter, JsonUtil, Keys, Kind, Metadata,
    Result, Tag, Timestamp,
};
use nostr_database::DynNostrDatabase;
use tokio::sync::{broadcast, RwLock};

#[cfg(feature = "blocking")]
pub mod blocking;
pub mod builder;
pub mod options;
pub mod signer;

pub use self::builder::ClientBuilder;
pub use self::options::Options;
#[cfg(feature = "nip46")]
pub use self::signer::nip46::Nip46Signer;
pub use self::signer::{ClientSigner, ClientSignerType};
use crate::relay::pool::{self, Error as RelayPoolError, RelayPool};
use crate::relay::{
    FilterOptions, NegentropyOptions, Relay, RelayOptions, RelayPoolNotification, RelaySendOptions,
};
use crate::util::TryIntoUrl;

/// [`Client`] error
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Keys error
    #[error(transparent)]
    Keys(#[from] nostr::key::Error),
    /// Url parse error
    #[error("impossible to parse URL: {0}")]
    Url(#[from] nostr::url::ParseError),
    /// [`RelayPool`] error
    #[error("relay pool error: {0}")]
    RelayPool(#[from] RelayPoolError),
    /// [`EventBuilder`] error
    #[error("event builder error: {0}")]
    EventBuilder(#[from] EventBuilderError),
    /// Unsigned event error
    #[error("unsigned event error: {0}")]
    UnsignedEvent(#[from] nostr::event::unsigned::Error),
    /// Secp256k1 error
    #[error("secp256k1 error: {0}")]
    Secp256k1(#[from] nostr::secp256k1::Error),
    /// Hex error
    #[error("hex decoding error: {0}")]
    Hex(#[from] nostr::hashes::hex::Error),
    /// Metadata error
    #[error(transparent)]
    Metadata(#[from] MetadataError),
    /// Notification Handler error
    #[error("notification handler error: {0}")]
    Handler(String),
    /// Signer not configured
    #[error("signer not configured")]
    SignerNotConfigured,
    /// Signer not configured
    #[error("wrong signer: expected={expected}, found={found}")]
    WrongSigner {
        /// Expected client signer type
        expected: ClientSignerType,
        /// Found client signer type
        found: ClientSignerType,
    },
    /// NIP04 error
    #[cfg(feature = "nip04")]
    #[error(transparent)]
    NIP04(#[from] nostr::nips::nip04::Error),
    /// NIP07 error
    #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
    #[error(transparent)]
    NIP07(#[from] nostr::nips::nip07::Error),
    /// NIP46 error
    #[cfg(feature = "nip46")]
    #[error(transparent)]
    NIP46(#[from] nostr::nips::nip46::Error),
    /// JSON error
    #[cfg(feature = "nip46")]
    #[error(transparent)]
    JSON(#[from] nostr::serde_json::Error),
    /// Generic NIP46 error
    #[cfg(feature = "nip46")]
    #[error("generic error")]
    Generic,
    /// NIP46 response error
    #[cfg(feature = "nip46")]
    #[error("response error: {0}")]
    Response(String),
    /// Signer public key not found
    #[cfg(feature = "nip46")]
    #[error("signer public key not found")]
    SignerPublicKeyNotFound,
    /// Timeout
    #[cfg(feature = "nip46")]
    #[error("timeout")]
    Timeout,
    /// Response not match to the request
    #[cfg(feature = "nip46")]
    #[error("response not match to the request")]
    ResponseNotMatchRequest,
}

/// Nostr client
#[derive(Debug, Clone)]
pub struct Client {
    pool: RelayPool,
    signer: Arc<RwLock<Option<ClientSigner>>>,
    opts: Options,
    dropped: Arc<AtomicBool>,
}

impl Default for Client {
    fn default() -> Self {
        ClientBuilder::new().build()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.opts.shutdown_on_drop {
            if self.dropped.load(Ordering::SeqCst) {
                tracing::warn!("Client already dropped");
            } else {
                tracing::debug!("Dropping the Client...");
                let _ = self
                    .dropped
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |_| Some(true));
                let client: Client = self.clone();
                thread::spawn(async move {
                    client
                        .shutdown()
                        .await
                        .expect("Impossible to drop the client")
                });
            }
        }
    }
}

impl Client {
    /// Create a new [`Client`] with signer
    ///
    /// To create a [`Client`] without any signer use `Client::default()`.
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// let my_keys = Keys::generate();
    /// let client = Client::new(&my_keys);
    /// ```
    pub fn new<S>(signer: S) -> Self
    where
        S: Into<ClientSigner>,
    {
        Self::with_opts(signer, Options::default())
    }

    /// Create a new [`Client`] with [`Options`]
    ///
    /// To create a [`Client`] with custom [`Options`] and without any signer use `ClientBuilder::new().opts(opts).build()`.
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// let my_keys = Keys::generate();
    /// let opts = Options::new().wait_for_send(true);
    /// let client = Client::with_opts(&my_keys, opts);
    /// ```
    pub fn with_opts<S>(signer: S, opts: Options) -> Self
    where
        S: Into<ClientSigner>,
    {
        ClientBuilder::new().signer(signer).opts(opts).build()
    }

    /// Compose [`Client`] from [`ClientBuilder`]
    pub fn from_builder(builder: ClientBuilder) -> Self {
        Self {
            pool: RelayPool::with_database(builder.opts.pool, builder.database),
            signer: Arc::new(RwLock::new(builder.signer)),
            opts: builder.opts,
            dropped: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Update default difficulty for new [`Event`]
    pub fn update_difficulty(&self, difficulty: u8) {
        self.opts.update_difficulty(difficulty);
    }

    /// Get current client signer
    ///
    /// Rise error if it not set.
    pub async fn signer(&self) -> Result<ClientSigner, Error> {
        let signer = self.signer.read().await;
        signer.clone().ok_or(Error::SignerNotConfigured)
    }

    /// Set client signer
    pub async fn set_signer(&self, signer: Option<ClientSigner>) {
        let mut s = self.signer.write().await;
        *s = signer;
    }

    /// Get current [`Keys`]
    #[deprecated(since = "0.27.0", note = "Use `client.signer().await` instead.")]
    pub async fn keys(&self) -> Keys {
        let signer = self.signer.read().await;
        if let Some(ClientSigner::Keys(keys)) = &*signer {
            keys.clone()
        } else {
            Keys::generate()
        }
    }

    /// Change [`Keys`]
    #[deprecated(since = "0.27.0", note = "Use `client.set_signer(...).await` instead.")]
    pub async fn set_keys(&self, keys: &Keys) {
        self.set_signer(Some(ClientSigner::Keys(keys.clone())))
            .await;
    }

    /// Get [`RelayPool`]
    pub fn pool(&self) -> RelayPool {
        self.pool.clone()
    }

    /// Get database
    pub fn database(&self) -> Arc<DynNostrDatabase> {
        self.pool.database()
    }

    /// Start a previously stopped client
    pub async fn start(&self) {
        self.pool.start();
        self.connect().await;
    }

    /// Stop the client
    ///
    /// Disconnect all relays and set their status to `RelayStatus::Stopped`.
    pub async fn stop(&self) -> Result<(), Error> {
        Ok(self.pool.stop().await?)
    }

    /// Check if [`RelayPool`] is running
    pub fn is_running(&self) -> bool {
        self.pool.is_running()
    }

    /// Completely shutdown [`Client`]
    pub async fn shutdown(self) -> Result<(), Error> {
        Ok(self.pool.clone().shutdown().await?)
    }

    /// Get new notification listener
    pub fn notifications(&self) -> broadcast::Receiver<RelayPoolNotification> {
        self.pool.notifications()
    }

    /// Get relays
    pub async fn relays(&self) -> HashMap<Url, Relay> {
        self.pool.relays().await
    }

    /// Get a previously added [`Relay`]
    pub async fn relay<U>(&self, url: U) -> Result<Relay, Error>
    where
        U: TryIntoUrl,
        pool::Error: From<<U as TryIntoUrl>::Err>,
    {
        Ok(self.pool.relay(url).await?)
    }

    /// Add new relay
    ///
    /// This method **NOT** automatically start connection with relay!
    ///
    /// Return `false` if the relay already exists.
    ///
    /// To use a proxy, see `Client::add_relay_with_opts`.
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// client.add_relay("wss://relay.nostr.info").await.unwrap();
    /// client.add_relay("wss://relay.damus.io").await.unwrap();
    ///
    /// client.connect().await;
    /// # }
    /// ```
    pub async fn add_relay<U>(&self, url: U) -> Result<bool, Error>
    where
        U: TryIntoUrl,
        pool::Error: From<<U as TryIntoUrl>::Err>,
    {
        #[cfg(not(target_arch = "wasm32"))]
        let opts: RelayOptions = RelayOptions::new().proxy(self.opts.proxy);
        #[cfg(target_arch = "wasm32")]
        let opts: RelayOptions = RelayOptions::new();
        self.add_relay_with_opts(url, opts).await
    }

    /// Add new relay with [`RelayOptions`]
    ///
    /// This method **NOT** automatically start connection with relay!
    ///
    /// Return `false` if the relay already exists.
    ///
    /// # Example
    /// ```rust,no_run
    /// use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    ///
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// let proxy = Some(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9050)));
    /// let opts = RelayOptions::new().proxy(proxy).write(false).retry_sec(11);
    /// client
    ///     .add_relay_with_opts("wss://relay.nostr.info", opts)
    ///     .await
    ///     .unwrap();
    ///
    /// client.connect().await;
    /// # }
    /// ```
    pub async fn add_relay_with_opts<U>(&self, url: U, opts: RelayOptions) -> Result<bool, Error>
    where
        U: TryIntoUrl,
        pool::Error: From<<U as TryIntoUrl>::Err>,
    {
        Ok(self.pool.add_relay(url, opts).await?)
    }

    /// Disconnect and remove relay
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// client.remove_relay("wss://relay.nostr.info").await.unwrap();
    /// # }
    /// ```
    pub async fn remove_relay<U>(&self, url: U) -> Result<(), Error>
    where
        U: TryIntoUrl,
        pool::Error: From<<U as TryIntoUrl>::Err>,
    {
        self.pool.remove_relay(url).await?;
        Ok(())
    }

    /// Add multiple relays
    ///
    /// This method **NOT** automatically start connection with relays!
    pub async fn add_relays<I, U>(&self, relays: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = U>,
        U: TryIntoUrl,
        pool::Error: From<<U as TryIntoUrl>::Err>,
    {
        for url in relays.into_iter() {
            self.add_relay(url).await?;
        }
        Ok(())
    }

    /// Connect to a previously added relay
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// client
    ///     .connect_relay("wss://relay.nostr.info")
    ///     .await
    ///     .unwrap();
    /// # }
    /// ```
    pub async fn connect_relay<U>(&self, url: U) -> Result<(), Error>
    where
        U: TryIntoUrl,
        pool::Error: From<<U as TryIntoUrl>::Err>,
    {
        let relay: Relay = self.relay(url).await?;
        self.pool
            .connect_relay(&relay, self.opts.connection_timeout)
            .await;
        Ok(())
    }

    /// Disconnect relay
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// client
    ///     .disconnect_relay("wss://relay.nostr.info")
    ///     .await
    ///     .unwrap();
    /// # }
    /// ```
    pub async fn disconnect_relay<U>(&self, url: U) -> Result<(), Error>
    where
        U: TryIntoUrl,
        pool::Error: From<<U as TryIntoUrl>::Err>,
    {
        let relay = self.relay(url).await?;
        self.pool.disconnect_relay(&relay).await?;
        Ok(())
    }

    /// Connect relays
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// client.connect().await;
    /// # }
    /// ```
    pub async fn connect(&self) {
        self.pool.connect(self.opts.connection_timeout).await;
    }

    /// Disconnect from all relays
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// client.disconnect().await.unwrap();
    /// # }
    /// ```
    pub async fn disconnect(&self) -> Result<(), Error> {
        Ok(self.pool.disconnect().await?)
    }

    /// Subscribe to filters
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// let subscription = Filter::new()
    ///     .pubkeys(vec![my_keys.public_key()])
    ///     .since(Timestamp::now());
    ///
    /// client.subscribe(vec![subscription]).await;
    /// # }
    /// ```
    pub async fn subscribe(&self, filters: Vec<Filter>) {
        let wait: Option<Duration> = if self.opts.get_wait_for_subscription() {
            self.opts.send_timeout
        } else {
            None
        };
        self.pool.subscribe(filters, wait).await;
    }

    /// Subscribe to filters with custom wait
    pub async fn subscribe_with_custom_wait(&self, filters: Vec<Filter>, wait: Option<Duration>) {
        self.pool.subscribe(filters, wait).await;
    }

    /// Unsubscribe from filters
    pub async fn unsubscribe(&self) {
        let wait: Option<Duration> = if self.opts.get_wait_for_subscription() {
            self.opts.send_timeout
        } else {
            None
        };
        self.pool.unsubscribe(wait).await;
    }

    /// Unsubscribe from filters with custom wait
    pub async fn unsubscribe_with_custom_wait(&self, wait: Option<Duration>) {
        self.pool.unsubscribe(wait).await;
    }

    /// Get events of filters
    ///
    /// If timeout is set to `None`, the default from [`Options`] will be used.
    ///
    /// # Example
    /// ```rust,no_run
    /// use std::time::Duration;
    ///
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// let subscription = Filter::new()
    ///     .pubkeys(vec![my_keys.public_key()])
    ///     .since(Timestamp::now());
    ///
    /// let timeout = Duration::from_secs(10);
    /// let _events = client
    ///     .get_events_of(vec![subscription], Some(timeout))
    ///     .await
    ///     .unwrap();
    /// # }
    /// ```
    pub async fn get_events_of(
        &self,
        filters: Vec<Filter>,
        timeout: Option<Duration>,
    ) -> Result<Vec<Event>, Error> {
        self.get_events_of_with_opts(filters, timeout, FilterOptions::ExitOnEOSE)
            .await
    }

    /// Get events of filters with [`FilterOptions`]
    ///
    /// If timeout is set to `None`, the default from [`Options`] will be used.
    pub async fn get_events_of_with_opts(
        &self,
        filters: Vec<Filter>,
        timeout: Option<Duration>,
        opts: FilterOptions,
    ) -> Result<Vec<Event>, Error> {
        let timeout: Duration = match timeout {
            Some(t) => t,
            None => self.opts.timeout,
        };
        Ok(self.pool.get_events_of(filters, timeout, opts).await?)
    }

    /// Request events of filters
    /// All events will be received on notification listener (`client.notifications()`)
    /// until the EOSE "end of stored events" message is received from the relay.
    ///
    /// If timeout is set to `None`, the default from [`Options`] will be used.
    pub async fn req_events_of(&self, filters: Vec<Filter>, timeout: Option<Duration>) {
        self.req_events_of_with_opts(filters, timeout, FilterOptions::ExitOnEOSE)
            .await
    }

    /// Request events of filters with [`FilterOptions`]
    ///
    /// If timeout is set to `None`, the default from [`Options`] will be used.
    pub async fn req_events_of_with_opts(
        &self,
        filters: Vec<Filter>,
        timeout: Option<Duration>,
        opts: FilterOptions,
    ) {
        let timeout: Duration = match timeout {
            Some(t) => t,
            None => self.opts.timeout,
        };
        self.pool.req_events_of(filters, timeout, opts).await;
    }

    /// Send client message
    pub async fn send_msg(&self, msg: ClientMessage) -> Result<(), Error> {
        let wait: Option<Duration> = if self.opts.get_wait_for_send() {
            self.opts.send_timeout
        } else {
            None
        };
        self.pool.send_msg(msg, wait).await?;
        Ok(())
    }

    /// Batch send client messages
    pub async fn batch_msg(
        &self,
        msgs: Vec<ClientMessage>,
        wait: Option<Duration>,
    ) -> Result<(), Error> {
        self.pool.batch_msg(msgs, wait).await?;
        Ok(())
    }

    /// Send client message to a specific relay
    pub async fn send_msg_to<U>(&self, url: U, msg: ClientMessage) -> Result<(), Error>
    where
        U: TryIntoUrl,
        pool::Error: From<<U as TryIntoUrl>::Err>,
    {
        let wait: Option<Duration> = if self.opts.get_wait_for_send() {
            self.opts.send_timeout
        } else {
            None
        };
        Ok(self.pool.send_msg_to(url, msg, wait).await?)
    }

    /// Send event
    ///
    /// This method will wait for the `OK` message from the relay.
    /// If you not want to wait for the `OK` message, use `send_msg` method instead.
    pub async fn send_event(&self, event: Event) -> Result<EventId, Error> {
        let timeout: Option<Duration> = self.opts.send_timeout;
        let opts = RelaySendOptions::new()
            .skip_disconnected(self.opts.get_skip_disconnected_relays())
            .timeout(timeout);
        Ok(self.pool.send_event(event, opts).await?)
    }

    /// Send multiple [`Event`] at once
    pub async fn batch_event(
        &self,
        events: Vec<Event>,
        opts: RelaySendOptions,
    ) -> Result<(), Error> {
        self.pool.batch_event(events, opts).await?;
        Ok(())
    }

    /// Send event to specific relay
    ///
    /// This method will wait for the `OK` message from the relay.
    /// If you not want to wait for the `OK` message, use `send_msg` method instead.
    pub async fn send_event_to<U>(&self, url: U, event: Event) -> Result<EventId, Error>
    where
        U: TryIntoUrl,
        pool::Error: From<<U as TryIntoUrl>::Err>,
    {
        let timeout: Option<Duration> = self.opts.send_timeout;
        let opts = RelaySendOptions::new()
            .skip_disconnected(self.opts.get_skip_disconnected_relays())
            .timeout(timeout);
        Ok(self.pool.send_event_to(url, event, opts).await?)
    }

    async fn internal_sign_event_builder(&self, builder: EventBuilder) -> Result<Event, Error> {
        match self.signer().await? {
            ClientSigner::Keys(keys) => {
                let difficulty: u8 = self.opts.get_difficulty();
                if difficulty > 0 {
                    Ok(builder.to_pow_event(&keys, difficulty)?)
                } else {
                    Ok(builder.to_event(&keys)?)
                }
            }
            #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
            ClientSigner::NIP07(nip07) => {
                let public_key: XOnlyPublicKey = nip07.get_public_key().await?;
                let unsigned = {
                    let difficulty: u8 = self.opts.get_difficulty();
                    if difficulty > 0 {
                        builder.to_unsigned_pow_event(public_key, difficulty)
                    } else {
                        builder.to_unsigned_event(public_key)
                    }
                };
                Ok(nip07.sign_event(unsigned).await?)
            }
            #[cfg(feature = "nip46")]
            ClientSigner::NIP46(nip46) => {
                let signer_public_key: XOnlyPublicKey = nip46
                    .signer_public_key()
                    .await
                    .ok_or(Error::SignerPublicKeyNotFound)?;
                let unsigned = {
                    let difficulty: u8 = self.opts.get_difficulty();
                    if difficulty > 0 {
                        builder.to_unsigned_pow_event(signer_public_key, difficulty)
                    } else {
                        builder.to_unsigned_event(signer_public_key)
                    }
                };
                let res: Response = self
                    .send_req_to_signer(Request::SignEvent(unsigned), self.opts.nip46_timeout)
                    .await?;
                if let Response::SignEvent(event) = res {
                    Ok(event)
                } else {
                    Err(Error::ResponseNotMatchRequest)
                }
            }
        }
    }

    /// Take an [`EventBuilder`], sign it by using the [`ClientSigner`] and broadcast to all relays.
    ///
    /// Rise an error if the [`ClientSigner`] is not set.
    pub async fn send_event_builder(&self, builder: EventBuilder) -> Result<EventId, Error> {
        let event: Event = self.internal_sign_event_builder(builder).await?;
        self.send_event(event).await
    }

    /// Take an [`EventBuilder`], sign it by using the [`ClientSigner`] and broadcast to specific relays.
    ///
    /// Rise an error if the [`ClientSigner`] is not set.
    pub async fn send_event_builder_to<U>(
        &self,
        url: U,
        builder: EventBuilder,
    ) -> Result<EventId, Error>
    where
        U: TryIntoUrl,
        pool::Error: From<<U as TryIntoUrl>::Err>,
    {
        let event: Event = self.internal_sign_event_builder(builder).await?;
        self.send_event_to(url, event).await
    }

    /// Update metadata
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/01.md>
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// let metadata = Metadata::new()
    ///     .name("username")
    ///     .display_name("My Username")
    ///     .about("Description")
    ///     .picture(Url::parse("https://example.com/avatar.png").unwrap())
    ///     .nip05("username@example.com");
    ///
    /// client.set_metadata(&metadata).await.unwrap();
    /// # }
    /// ```
    pub async fn set_metadata(&self, metadata: &Metadata) -> Result<EventId, Error> {
        let builder = EventBuilder::metadata(metadata);
        self.send_event_builder(builder).await
    }

    /// Publish text note
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/01.md>
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// client
    ///     .publish_text_note("My first text note from Nostr SDK!", [])
    ///     .await
    ///     .unwrap();
    /// # }
    /// ```
    pub async fn publish_text_note<S, I>(&self, content: S, tags: I) -> Result<EventId, Error>
    where
        S: Into<String>,
        I: IntoIterator<Item = Tag>,
    {
        let builder = EventBuilder::text_note(content, tags);
        self.send_event_builder(builder).await
    }

    /// Add recommended relay
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/01.md>
    #[deprecated(since = "0.27.0")]
    pub async fn add_recommended_relay<U>(&self, url: U) -> Result<EventId, Error>
    where
        U: TryIntoUrl,
        Error: From<<U as TryIntoUrl>::Err>,
    {
        let url: Url = url.try_into_url()?;
        #[allow(deprecated)]
        let builder = EventBuilder::add_recommended_relay(&url);
        self.send_event_builder(builder).await
    }

    /// Set contact list
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/02.md>
    pub async fn set_contact_list<I>(&self, list: I) -> Result<EventId, Error>
    where
        I: IntoIterator<Item = Contact>,
    {
        let builder = EventBuilder::contact_list(list);
        self.send_event_builder(builder).await
    }

    async fn get_contact_list_filters(&self) -> Result<Vec<Filter>, Error> {
        let mut filter: Filter = Filter::new().kind(Kind::ContactList).limit(1);

        match self.signer().await? {
            ClientSigner::Keys(keys) => {
                filter = filter.author(keys.public_key());
            }
            #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
            ClientSigner::NIP07(nip07) => {
                let public_key: XOnlyPublicKey = nip07.get_public_key().await?;
                filter = filter.author(public_key);
            }
            #[cfg(feature = "nip46")]
            ClientSigner::NIP46(nip46) => {
                let signer_public_key = nip46
                    .signer_public_key()
                    .await
                    .ok_or(Error::SignerPublicKeyNotFound)?;

                filter = filter.author(signer_public_key);
            }
        };

        Ok(vec![filter])
    }

    /// Get contact list
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/02.md>
    ///
    /// # Example
    /// ```rust,no_run
    /// use std::time::Duration;
    ///
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// let timeout = Duration::from_secs(10);
    /// let _list = client.get_contact_list(Some(timeout)).await.unwrap();
    /// # }
    /// ```
    pub async fn get_contact_list(&self, timeout: Option<Duration>) -> Result<Vec<Contact>, Error> {
        let mut contact_list: Vec<Contact> = Vec::new();
        let filters: Vec<Filter> = self.get_contact_list_filters().await?;
        let events: Vec<Event> = self.get_events_of(filters, timeout).await?;

        for event in events.into_iter() {
            for tag in event.into_iter_tags() {
                if let Tag::PublicKey {
                    public_key,
                    relay_url,
                    alias,
                    uppercase: false,
                } = tag
                {
                    contact_list.push(Contact::new(public_key, relay_url, alias))
                }
            }
        }

        Ok(contact_list)
    }

    /// Get contact list public keys
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/02.md>
    pub async fn get_contact_list_public_keys(
        &self,
        timeout: Option<Duration>,
    ) -> Result<Vec<XOnlyPublicKey>, Error> {
        let mut pubkeys: Vec<XOnlyPublicKey> = Vec::new();
        let filters: Vec<Filter> = self.get_contact_list_filters().await?;
        let events: Vec<Event> = self.get_events_of(filters, timeout).await?;

        for event in events.into_iter() {
            pubkeys.extend(event.public_keys());
        }

        Ok(pubkeys)
    }

    /// Get contact list [`Metadata`]
    pub async fn get_contact_list_metadata(
        &self,
        timeout: Option<Duration>,
    ) -> Result<HashMap<XOnlyPublicKey, Metadata>, Error> {
        let public_keys = self.get_contact_list_public_keys(timeout).await?;
        let mut contacts: HashMap<XOnlyPublicKey, Metadata> =
            public_keys.iter().map(|p| (*p, Metadata::new())).collect();

        let chunk_size: usize = self.opts.get_req_filters_chunk_size();
        for chunk in public_keys.chunks(chunk_size) {
            let mut filters: Vec<Filter> = Vec::new();
            for public_key in chunk.iter() {
                filters.push(
                    Filter::new()
                        .author(*public_key)
                        .kind(Kind::Metadata)
                        .limit(1),
                );
            }
            let events: Vec<Event> = self.get_events_of(filters, timeout).await?;
            for event in events.into_iter() {
                let metadata = Metadata::from_json(event.content())?;
                if let Some(m) = contacts.get_mut(&event.author()) {
                    *m = metadata
                };
            }
        }

        Ok(contacts)
    }

    /// Send encrypted direct message
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/04.md>
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// let alice_pubkey = XOnlyPublicKey::from_bech32(
    ///     "npub14f8usejl26twx0dhuxjh9cas7keav9vr0v8nvtwtrjqx3vycc76qqh9nsy",
    /// )
    /// .unwrap();
    ///
    /// client
    ///     .send_direct_msg(alice_pubkey, "My first DM fro Nostr SDK!", None)
    ///     .await
    ///     .unwrap();
    /// # }
    /// ```
    #[cfg(feature = "nip04")]
    pub async fn send_direct_msg<S>(
        &self,
        receiver: XOnlyPublicKey,
        msg: S,
        reply_to: Option<EventId>,
    ) -> Result<EventId, Error>
    where
        S: Into<String>,
    {
        let builder: EventBuilder = match self.signer().await? {
            ClientSigner::Keys(keys) => {
                EventBuilder::encrypted_direct_msg(&keys, receiver, msg, reply_to)?
            }
            #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
            ClientSigner::NIP07(nip07) => {
                let content: String = nip07.nip04_encrypt(receiver, msg.into()).await?;
                EventBuilder::new(
                    Kind::EncryptedDirectMessage,
                    content,
                    [Tag::public_key(receiver)],
                )
            }
            #[cfg(feature = "nip46")]
            ClientSigner::NIP46(..) => {
                let req = Request::Nip04Encrypt {
                    public_key: receiver,
                    text: msg.into(),
                };
                let res: Response = self
                    .send_req_to_signer(req, self.opts.nip46_timeout)
                    .await?;
                if let Response::Nip04Encrypt(content) = res {
                    EventBuilder::new(
                        Kind::EncryptedDirectMessage,
                        content,
                        [Tag::public_key(receiver)],
                    )
                } else {
                    return Err(Error::ResponseNotMatchRequest);
                }
            }
        };

        self.send_event_builder(builder).await
    }

    /// Repost event
    pub async fn repost_event(
        &self,
        event_id: EventId,
        public_key: XOnlyPublicKey,
    ) -> Result<EventId, Error> {
        let builder = EventBuilder::repost(event_id, public_key);
        self.send_event_builder(builder).await
    }

    /// Delete event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/09.md>
    pub async fn delete_event<T>(&self, id: T) -> Result<EventId, Error>
    where
        T: Into<EventIdOrCoordinate>,
    {
        let builder = EventBuilder::delete([id]);
        self.send_event_builder(builder).await
    }

    /// Like event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/25.md>
    ///
    /// # Example
    /// ```rust,no_run
    /// use std::str::FromStr;
    ///
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// let event_id =
    ///     EventId::from_hex("3aded8d2194dc2fedb1d7b70480b43b6c4deb0a22dcdc9c471d1958485abcf21")
    ///         .unwrap();
    /// let public_key = XOnlyPublicKey::from_str(
    ///     "a8e76c3ace7829f9ee44cf9293309e21a1824bf1e57631d00685a1ed0b0bd8a2",
    /// )
    /// .unwrap();
    ///
    /// client.like(event_id, public_key).await.unwrap();
    /// # }
    /// ```
    pub async fn like(
        &self,
        event_id: EventId,
        public_key: XOnlyPublicKey,
    ) -> Result<EventId, Error> {
        let builder = EventBuilder::reaction(event_id, public_key, "+");
        self.send_event_builder(builder).await
    }

    /// Disike event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/25.md>
    ///
    /// # Example
    /// ```rust,no_run
    /// use std::str::FromStr;
    ///
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// let event_id =
    ///     EventId::from_hex("3aded8d2194dc2fedb1d7b70480b43b6c4deb0a22dcdc9c471d1958485abcf21")
    ///         .unwrap();
    /// let public_key = XOnlyPublicKey::from_str(
    ///     "a8e76c3ace7829f9ee44cf9293309e21a1824bf1e57631d00685a1ed0b0bd8a2",
    /// )
    /// .unwrap();
    ///
    /// client.dislike(event_id, public_key).await.unwrap();
    /// # }
    /// ```
    pub async fn dislike(
        &self,
        event_id: EventId,
        public_key: XOnlyPublicKey,
    ) -> Result<EventId, Error> {
        let builder = EventBuilder::reaction(event_id, public_key, "-");
        self.send_event_builder(builder).await
    }

    /// React to an [`Event`]
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/25.md>
    ///
    /// # Example
    /// ```rust,no_run
    /// use std::str::FromStr;
    ///
    /// use nostr_sdk::prelude::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #   let my_keys = Keys::generate();
    /// #   let client = Client::new(&my_keys);
    /// let event_id =
    ///     EventId::from_hex("3aded8d2194dc2fedb1d7b70480b43b6c4deb0a22dcdc9c471d1958485abcf21")
    ///         .unwrap();
    /// let public_key = XOnlyPublicKey::from_str(
    ///     "a8e76c3ace7829f9ee44cf9293309e21a1824bf1e57631d00685a1ed0b0bd8a2",
    /// )
    /// .unwrap();
    ///
    /// client.reaction(event_id, public_key, "🐻").await.unwrap();
    /// # }
    /// ```
    pub async fn reaction<S>(
        &self,
        event_id: EventId,
        public_key: XOnlyPublicKey,
        content: S,
    ) -> Result<EventId, Error>
    where
        S: Into<String>,
    {
        let builder = EventBuilder::reaction(event_id, public_key, content);
        self.send_event_builder(builder).await
    }

    /// Create new channel
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    pub async fn new_channel(&self, metadata: &Metadata) -> Result<EventId, Error> {
        let builder = EventBuilder::channel(metadata);
        self.send_event_builder(builder).await
    }

    /// Update channel metadata
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    pub async fn set_channel_metadata(
        &self,
        channel_id: EventId,
        relay_url: Option<Url>,
        metadata: &Metadata,
    ) -> Result<EventId, Error> {
        let builder = EventBuilder::channel_metadata(channel_id, relay_url, metadata);
        self.send_event_builder(builder).await
    }

    /// Send message to channel
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    pub async fn send_channel_msg<S>(
        &self,
        channel_id: EventId,
        relay_url: Url,
        msg: S,
    ) -> Result<EventId, Error>
    where
        S: Into<String>,
    {
        let builder = EventBuilder::channel_msg(channel_id, relay_url, msg);
        self.send_event_builder(builder).await
    }

    /// Hide channel message
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    pub async fn hide_channel_msg<S>(
        &self,
        message_id: EventId,
        reason: Option<S>,
    ) -> Result<EventId, Error>
    where
        S: Into<String>,
    {
        let builder = EventBuilder::hide_channel_msg(message_id, reason);
        self.send_event_builder(builder).await
    }

    /// Mute channel user
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    pub async fn mute_channel_user<S>(
        &self,
        pubkey: XOnlyPublicKey,
        reason: Option<S>,
    ) -> Result<EventId, Error>
    where
        S: Into<String>,
    {
        let builder = EventBuilder::mute_channel_user(pubkey, reason);
        self.send_event_builder(builder).await
    }

    /// Create an auth event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/42.md>
    pub async fn auth<S>(&self, challenge: S, relay: Url) -> Result<EventId, Error>
    where
        S: Into<String>,
    {
        let builder = EventBuilder::auth(challenge, relay);
        self.send_event_builder(builder).await
    }

    /// Create zap receipt event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/57.md>
    #[cfg(feature = "nip57")]
    pub async fn zap_receipt<S>(
        &self,
        bolt11: S,
        preimage: Option<S>,
        zap_request: Event,
    ) -> Result<EventId, Error>
    where
        S: Into<String>,
    {
        let builder = EventBuilder::zap_receipt(bolt11, preimage, zap_request);
        self.send_event_builder(builder).await
    }

    /// Create zap receipt event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/57.md>
    #[cfg(feature = "nip57")]
    #[deprecated(since = "0.27.0", note = "Use `zap_receipt` instead")]
    pub async fn new_zap_receipt<S>(
        &self,
        bolt11: S,
        preimage: Option<S>,
        zap_request: Event,
    ) -> Result<EventId, Error>
    where
        S: Into<String>,
    {
        let builder = EventBuilder::zap_receipt(bolt11, preimage, zap_request);
        self.send_event_builder(builder).await
    }

    /// File metadata
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/94.md>
    pub async fn file_metadata<S>(
        &self,
        description: S,
        metadata: FileMetadata,
    ) -> Result<EventId, Error>
    where
        S: Into<String>,
    {
        let builder = EventBuilder::file_metadata(description, metadata);
        self.send_event_builder(builder).await
    }

    /// Negentropy reconciliation
    ///
    /// <https://github.com/hoytech/negentropy>
    pub async fn reconcile(&self, filter: Filter, opts: NegentropyOptions) -> Result<(), Error> {
        Ok(self.pool.reconcile(filter, opts).await?)
    }

    /// Negentropy reconciliation with items
    pub async fn reconcile_with_items(
        &self,
        filter: Filter,
        items: Vec<(EventId, Timestamp)>,
        opts: NegentropyOptions,
    ) -> Result<(), Error> {
        Ok(self.pool.reconcile_with_items(filter, items, opts).await?)
    }

    /// Get a list of channels
    #[deprecated(since = "0.27.0")]
    pub async fn get_channels(&self, timeout: Option<Duration>) -> Result<Vec<Event>, Error> {
        self.get_events_of(vec![Filter::new().kind(Kind::ChannelCreation)], timeout)
            .await
    }

    /// Handle notifications
    pub async fn handle_notifications<F, Fut>(&self, func: F) -> Result<(), Error>
    where
        F: Fn(RelayPoolNotification) -> Fut,
        Fut: Future<Output = Result<bool>>,
    {
        let mut notifications = self.notifications();
        while let Ok(notification) = notifications.recv().await {
            let stop: bool = RelayPoolNotification::Stop == notification;
            let shutdown: bool = RelayPoolNotification::Shutdown == notification;
            let exit: bool = func(notification)
                .await
                .map_err(|e| Error::Handler(e.to_string()))?;
            if exit || stop || shutdown {
                break;
            }
        }
        Ok(())
    }
}
