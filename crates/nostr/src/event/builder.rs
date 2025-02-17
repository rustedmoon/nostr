// Copyright (c) 2022-2023 Yuki Kishimoto
// Copyright (c) 2023-2024 Rust Nostr Developers
// Distributed under the MIT software license

//! Event builder

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::ops::Range;

#[cfg(feature = "std")]
use bitcoin::secp256k1::rand;
use bitcoin::secp256k1::rand::{CryptoRng, Rng};
use bitcoin::secp256k1::{self, Secp256k1, Signing, XOnlyPublicKey};
use serde_json::{json, Value};
use url_fork::Url;

use super::kind::{Kind, NIP90_JOB_REQUEST_RANGE, NIP90_JOB_RESULT_RANGE};
use super::tag::ImageDimensions;
use super::{Event, EventId, Marker, Tag, TagKind, UnsignedEvent};
use crate::key::{self, Keys};
#[cfg(feature = "nip04")]
use crate::nips::nip04;
use crate::nips::nip15::{ProductData, StallData};
#[cfg(all(feature = "std", feature = "nip46"))]
use crate::nips::nip46::Message as NostrConnectMessage;
use crate::nips::nip53::LiveEvent;
#[cfg(feature = "nip57")]
use crate::nips::nip57::ZapRequestData;
use crate::nips::nip58::Error as Nip58Error;
use crate::nips::nip90::DataVendingMachineStatus;
use crate::nips::nip94::FileMetadata;
use crate::nips::nip98::HttpData;
use crate::nips::{nip13, nip58};
#[cfg(feature = "std")]
use crate::types::time::Instant;
use crate::types::time::TimeSupplier;
use crate::types::{Contact, Metadata, Timestamp};
use crate::util::EventIdOrCoordinate;
#[cfg(feature = "std")]
use crate::SECP256K1;
use crate::{JsonUtil, RelayMetadata, UncheckedUrl};

/// Wrong kind error
#[derive(Debug)]
pub enum WrongKindError {
    /// Singe kind
    Single(Kind),
    /// Range
    Range(Range<u64>),
}

impl fmt::Display for WrongKindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Single(k) => write!(f, "{k}"),
            Self::Range(range) => write!(f, "'{} <= k <= {}'", range.start, range.end),
        }
    }
}

/// [`EventBuilder`] error
#[derive(Debug)]
pub enum Error {
    /// Key error
    Key(key::Error),
    /// JSON error
    Json(serde_json::Error),
    /// Secp256k1 error
    Secp256k1(secp256k1::Error),
    /// Unsigned event error
    Unsigned(super::unsigned::Error),
    /// OpenTimestamps error
    #[cfg(feature = "nip03")]
    OpenTimestamps(nostr_ots::Error),
    /// NIP04 error
    #[cfg(feature = "nip04")]
    NIP04(nip04::Error),
    /// NIP58 error
    NIP58(nip58::Error),
    /// Wrong kind
    WrongKind {
        /// The received wrong kind
        received: Kind,
        /// The expected kind (single or range)
        expected: WrongKindError,
    },
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(e) => write!(f, "Key: {e}"),
            Self::Json(e) => write!(f, "Json: {e}"),
            Self::Secp256k1(e) => write!(f, "Secp256k1: {e}"),
            Self::Unsigned(e) => write!(f, "Unsigned event: {e}"),
            #[cfg(feature = "nip03")]
            Self::OpenTimestamps(e) => write!(f, "NIP03: {e}"),
            #[cfg(feature = "nip04")]
            Self::NIP04(e) => write!(f, "NIP04: {e}"),
            Self::NIP58(e) => write!(f, "NIP58: {e}"),
            Self::WrongKind { received, expected } => {
                write!(f, "Wrong kind: received={received}, expected={expected}")
            }
        }
    }
}

impl From<key::Error> for Error {
    fn from(e: key::Error) -> Self {
        Self::Key(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<secp256k1::Error> for Error {
    fn from(e: secp256k1::Error) -> Self {
        Self::Secp256k1(e)
    }
}

impl From<super::unsigned::Error> for Error {
    fn from(e: super::unsigned::Error) -> Self {
        Self::Unsigned(e)
    }
}

#[cfg(feature = "nip03")]
impl From<nostr_ots::Error> for Error {
    fn from(e: nostr_ots::Error) -> Self {
        Self::OpenTimestamps(e)
    }
}

#[cfg(feature = "nip04")]
impl From<nip04::Error> for Error {
    fn from(e: nip04::Error) -> Self {
        Self::NIP04(e)
    }
}

impl From<nip58::Error> for Error {
    fn from(e: nip58::Error) -> Self {
        Self::NIP58(e)
    }
}

/// [`Event`] builder
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EventBuilder {
    kind: Kind,
    tags: Vec<Tag>,
    content: String,
    custom_created_at: Option<Timestamp>,
}

impl EventBuilder {
    /// New [`EventBuilder`]
    pub fn new<S, I>(kind: Kind, content: S, tags: I) -> Self
    where
        S: Into<String>,
        I: IntoIterator<Item = Tag>,
    {
        Self {
            kind,
            tags: tags.into_iter().collect(),
            content: content.into(),
            custom_created_at: None,
        }
    }

    /// Set a custom `created_at` UNIX timestamp
    pub fn custom_created_at(mut self, created_at: Timestamp) -> Self {
        self.custom_created_at = Some(created_at);
        self
    }

    /// Build [`Event`]
    pub fn to_event_with_ctx<C, R, T>(
        self,
        secp: &Secp256k1<C>,
        rng: &mut R,
        supplier: &T,
        keys: &Keys,
    ) -> Result<Event, Error>
    where
        C: Signing,
        R: Rng + CryptoRng,
        T: TimeSupplier,
    {
        let pubkey: XOnlyPublicKey = keys.public_key();
        Ok(self
            .to_unsigned_event_with_supplier(supplier, pubkey)
            .sign_with_ctx(secp, rng, keys)?)
    }

    /// Build [`UnsignedEvent`]
    pub fn to_unsigned_event_with_supplier<T>(
        self,
        supplier: &T,
        pubkey: XOnlyPublicKey,
    ) -> UnsignedEvent
    where
        T: TimeSupplier,
    {
        let created_at: Timestamp = self
            .custom_created_at
            .unwrap_or_else(|| Timestamp::now_with_supplier(supplier));
        let id = EventId::new(&pubkey, created_at, &self.kind, &self.tags, &self.content);
        UnsignedEvent {
            id,
            pubkey,
            created_at,
            kind: self.kind,
            tags: self.tags,
            content: self.content,
        }
    }

    /// Build POW [`Event`]
    pub fn to_pow_event_with_ctx<C, R, T>(
        self,
        secp: &Secp256k1<C>,
        rng: &mut R,
        supplier: &T,
        keys: &Keys,
        difficulty: u8,
    ) -> Result<Event, Error>
    where
        C: Signing,
        R: Rng + CryptoRng,
        T: TimeSupplier,
    {
        let pubkey: XOnlyPublicKey = keys.public_key();
        Ok(self
            .to_unsigned_pow_event_with_supplier(supplier, pubkey, difficulty)
            .sign_with_ctx(secp, rng, keys)?)
    }

    /// Build unsigned POW [`Event`]
    pub fn to_unsigned_pow_event_with_supplier<T>(
        self,
        supplier: &T,
        pubkey: XOnlyPublicKey,
        difficulty: u8,
    ) -> UnsignedEvent
    where
        T: TimeSupplier,
    {
        let mut nonce: u128 = 0;
        let mut tags: Vec<Tag> = self.tags;

        #[cfg(feature = "std")]
        let now = Instant::now();

        loop {
            nonce += 1;

            tags.push(Tag::POW { nonce, difficulty });

            let created_at: Timestamp = self
                .custom_created_at
                .unwrap_or_else(|| Timestamp::now_with_supplier(supplier));
            let id = EventId::new(&pubkey, created_at, &self.kind, &tags, &self.content);

            if nip13::get_leading_zero_bits(id.inner()) >= difficulty {
                #[cfg(feature = "std")]
                tracing::debug!(
                    "{} iterations in {} ms. Avg rate {} hashes/second",
                    nonce,
                    now.elapsed().as_millis(),
                    nonce * 1000 / std::cmp::max(1, now.elapsed().as_millis())
                );

                return UnsignedEvent {
                    id,
                    pubkey,
                    created_at,
                    kind: self.kind,
                    tags,
                    content: self.content,
                };
            }

            tags.pop();
        }
    }
}

impl EventBuilder {
    /// Build [`Event`]
    #[cfg(feature = "std")]
    pub fn to_event(self, keys: &Keys) -> Result<Event, Error> {
        self.to_event_with_ctx(&SECP256K1, &mut rand::thread_rng(), &Instant::now(), keys)
    }

    /// Build [`UnsignedEvent`]
    #[cfg(feature = "std")]
    pub fn to_unsigned_event(self, pubkey: XOnlyPublicKey) -> UnsignedEvent {
        self.to_unsigned_event_with_supplier(&Instant::now(), pubkey)
    }

    /// Build POW [`Event`]
    #[cfg(feature = "std")]
    pub fn to_pow_event(self, keys: &Keys, difficulty: u8) -> Result<Event, Error> {
        self.to_pow_event_with_ctx(
            &SECP256K1,
            &mut rand::thread_rng(),
            &Instant::now(),
            keys,
            difficulty,
        )
    }

    /// Build unsigned POW [`Event`]
    #[cfg(feature = "std")]
    pub fn to_unsigned_pow_event(self, pubkey: XOnlyPublicKey, difficulty: u8) -> UnsignedEvent {
        self.to_unsigned_pow_event_with_supplier(&Instant::now(), pubkey, difficulty)
    }
}

impl EventBuilder {
    /// Profile metadata
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/01.md>
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr::url::Url;
    /// use nostr::{EventBuilder, Metadata};
    ///
    /// let metadata = Metadata::new()
    ///     .name("username")
    ///     .display_name("My Username")
    ///     .about("Description")
    ///     .picture(Url::parse("https://example.com/avatar.png").unwrap())
    ///     .nip05("username@example.com")
    ///     .lud16("yuki@getalby.com");
    ///
    /// let builder = EventBuilder::metadata(&metadata);
    /// ```
    pub fn metadata(metadata: &Metadata) -> Self {
        Self::new(Kind::Metadata, metadata.as_json(), [])
    }

    /// Set metadata
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/01.md>
    #[deprecated(since = "0.27.0", note = "Use `metadata` instead")]
    pub fn set_metadata(metadata: &Metadata) -> Self {
        Self::metadata(metadata)
    }

    /// Add recommended relay
    #[deprecated(since = "0.27.0")]
    pub fn add_recommended_relay(url: &Url) -> Self {
        Self::new(Kind::RecommendRelay, url.as_ref(), [])
    }

    /// Relay list metadata (NIP65)
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/65.md>
    pub fn relay_list<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = (UncheckedUrl, Option<RelayMetadata>)>,
    {
        let tags = iter
            .into_iter()
            .map(|(url, metadata)| Tag::RelayMetadata(url, metadata));
        Self::new(Kind::RelayList, "", tags)
    }

    /// Text note
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/01.md>
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr::EventBuilder;
    ///
    /// let builder = EventBuilder::text_note("My first text note from Nostr SDK!", []);
    /// ```
    pub fn text_note<S, I>(content: S, tags: I) -> Self
    where
        S: Into<String>,
        I: IntoIterator<Item = Tag>,
    {
        Self::new(Kind::TextNote, content, tags)
    }

    /// Text note
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/01.md>
    #[deprecated(since = "0.27.0", note = "Use `text_note` instead")]
    pub fn new_text_note<S, I>(content: S, tags: I) -> Self
    where
        S: Into<String>,
        I: IntoIterator<Item = Tag>,
    {
        Self::text_note(content, tags)
    }

    /// Long-form text note (generally referred to as "articles" or "blog posts").
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/23.md>
    ///
    /// # Example
    /// ```rust,no_run
    /// use std::str::FromStr;
    ///
    /// use nostr::{EventBuilder, Tag, Timestamp, EventId, UncheckedUrl};
    ///
    /// let event_id = EventId::from_hex("b3e392b11f5d4f28321cedd09303a748acfd0487aea5a7450b3481c60b6e4f87").unwrap();
    /// let content: &str = "Lorem [ipsum][4] dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.\n\nRead more at #[3].";
    /// let tags = &[
    ///     Tag::Identifier("lorem-ipsum".to_string()),
    ///     Tag::Title("Lorem Ipsum".to_string()),
    ///     Tag::PublishedAt(Timestamp::from(1296962229)),
    ///     Tag::Hashtag("placeholder".to_string()),
    ///     Tag::event(event_id),
    /// ];
    /// let builder = EventBuilder::long_form_text_note("My first text note from Nostr SDK!", []);
    /// ```
    pub fn long_form_text_note<S, I>(content: S, tags: I) -> Self
    where
        S: Into<String>,
        I: IntoIterator<Item = Tag>,
    {
        Self::new(Kind::LongFormTextNote, content, tags)
    }

    /// Contact list
    pub fn contact_list<I>(contacts: I) -> Self
    where
        I: IntoIterator<Item = Contact>,
    {
        let tags = contacts.into_iter().map(|contact| Tag::PublicKey {
            public_key: contact.pk,
            relay_url: contact.relay_url,
            alias: contact.alias,
            uppercase: false,
        });
        Self::new(Kind::ContactList, "", tags)
    }

    /// Set contact list
    #[deprecated(since = "0.27.0", note = "Use `contact_list` instead")]
    pub fn set_contact_list<I>(contacts: I) -> Self
    where
        I: IntoIterator<Item = Contact>,
    {
        Self::contact_list(contacts)
    }

    /// OpenTimestamps Attestations for Events
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/03.md>
    #[cfg(feature = "nip03")]
    pub fn opentimestamps(
        event_id: EventId,
        relay_url: Option<UncheckedUrl>,
    ) -> Result<Self, Error> {
        let ots: String = nostr_ots::timestamp_event(&event_id.to_hex())?;
        Ok(Self::new(
            Kind::OpenTimestamps,
            ots,
            [Tag::Event {
                event_id,
                relay_url,
                marker: None,
            }],
        ))
    }

    /// Create encrypted direct msg event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/04.md>
    #[cfg(all(feature = "std", feature = "nip04"))]
    pub fn encrypted_direct_msg<S>(
        sender_keys: &Keys,
        receiver_pubkey: XOnlyPublicKey,
        content: S,
        reply_to: Option<EventId>,
    ) -> Result<Self, Error>
    where
        S: Into<String>,
    {
        let mut tags: Vec<Tag> = vec![Tag::public_key(receiver_pubkey)];
        if let Some(reply_to) = reply_to {
            tags.push(Tag::event(reply_to));
        }
        Ok(Self::new(
            Kind::EncryptedDirectMessage,
            nip04::encrypt(&sender_keys.secret_key()?, &receiver_pubkey, content.into())?,
            tags,
        ))
    }

    /// Create encrypted direct msg event
    #[cfg(all(feature = "std", feature = "nip04"))]
    #[deprecated(since = "0.27.0", note = "Use `encrypted_direct_msg` instead")]
    pub fn new_encrypted_direct_msg<S>(
        sender_keys: &Keys,
        receiver_pubkey: XOnlyPublicKey,
        content: S,
        reply_to: Option<EventId>,
    ) -> Result<Self, Error>
    where
        S: Into<String>,
    {
        Self::encrypted_direct_msg(sender_keys, receiver_pubkey, content, reply_to)
    }

    /// Repost event
    pub fn repost(event_id: EventId, public_key: XOnlyPublicKey) -> Self {
        Self::new(
            Kind::Repost,
            String::new(),
            [Tag::event(event_id), Tag::public_key(public_key)],
        )
    }

    /// Create delete event
    pub fn delete<I, T>(ids: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<EventIdOrCoordinate>,
    {
        Self::delete_with_reason(ids, "")
    }

    /// Create delete event with reason
    pub fn delete_with_reason<I, T, S>(ids: I, reason: S) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<EventIdOrCoordinate>,
        S: Into<String>,
    {
        let tags = ids.into_iter().map(|t| {
            let middle: EventIdOrCoordinate = t.into();
            middle.into()
        });
        Self::new(Kind::EventDeletion, reason.into(), tags)
    }

    /// Add reaction (like/upvote, dislike/downvote or emoji) to an event
    pub fn reaction<S>(event_id: EventId, public_key: XOnlyPublicKey, content: S) -> Self
    where
        S: Into<String>,
    {
        Self::new(
            Kind::Reaction,
            content,
            [Tag::event(event_id), Tag::public_key(public_key)],
        )
    }

    /// Add reaction (like/upvote, dislike/downvote or emoji) to an event
    #[deprecated(since = "0.27.0", note = "Use `reaction` instead")]
    pub fn new_reaction<S>(event_id: EventId, public_key: XOnlyPublicKey, content: S) -> Self
    where
        S: Into<String>,
    {
        Self::reaction(event_id, public_key, content)
    }

    /// Create new channel
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    pub fn channel(metadata: &Metadata) -> Self {
        Self::new(Kind::ChannelCreation, metadata.as_json(), [])
    }

    /// Create new channel
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    #[deprecated(since = "0.27.0", note = "Use `channel` instead")]
    pub fn new_channel(metadata: &Metadata) -> Self {
        Self::channel(metadata)
    }

    /// Channel metadata
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    pub fn channel_metadata(
        channel_id: EventId,
        relay_url: Option<Url>,
        metadata: &Metadata,
    ) -> Self {
        Self::new(
            Kind::ChannelMetadata,
            metadata.as_json(),
            [Tag::Event {
                event_id: channel_id,
                relay_url: relay_url.map(|u| u.into()),
                marker: None,
            }],
        )
    }

    /// Set channel metadata
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    #[deprecated(since = "0.27.0", note = "Use `channel_metadata` instead")]
    pub fn set_channel_metadata(
        channel_id: EventId,
        relay_url: Option<Url>,
        metadata: &Metadata,
    ) -> Self {
        Self::channel_metadata(channel_id, relay_url, metadata)
    }

    /// Channel message
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    pub fn channel_msg<S>(channel_id: EventId, relay_url: Url, content: S) -> Self
    where
        S: Into<String>,
    {
        Self::new(
            Kind::ChannelMessage,
            content,
            [Tag::Event {
                event_id: channel_id,
                relay_url: Some(relay_url.into()),
                marker: Some(Marker::Root),
            }],
        )
    }

    /// Channel message
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    #[deprecated(since = "0.27.0", note = "Use `channel_msg` instead")]
    pub fn new_channel_msg<S>(channel_id: EventId, relay_url: Url, content: S) -> Self
    where
        S: Into<String>,
    {
        Self::channel_msg(channel_id, relay_url, content)
    }

    /// Hide message
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    pub fn hide_channel_msg<S>(
        message_id: EventId, // event id of kind 42
        reason: Option<S>,
    ) -> Self
    where
        S: Into<String>,
    {
        let content: Value = json!({
            "reason": reason.map(|s| s.into()).unwrap_or_default(),
        });

        Self::new(
            Kind::ChannelHideMessage,
            content.to_string(),
            [Tag::event(message_id)],
        )
    }

    /// Mute channel user
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/28.md>
    pub fn mute_channel_user<S>(public_key: XOnlyPublicKey, reason: Option<S>) -> Self
    where
        S: Into<String>,
    {
        let content: Value = json!({
            "reason": reason.map(|s| s.into()).unwrap_or_default(),
        });

        Self::new(
            Kind::ChannelMuteUser,
            content.to_string(),
            [Tag::public_key(public_key)],
        )
    }

    /// Create an auth event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/42.md>
    pub fn auth<S>(challenge: S, relay: Url) -> Self
    where
        S: Into<String>,
    {
        Self::new(
            Kind::Authentication,
            "",
            [Tag::Challenge(challenge.into()), Tag::Relay(relay.into())],
        )
    }

    /// Nostr Connect
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/46.md>
    #[cfg(all(feature = "std", feature = "nip04", feature = "nip46"))]
    pub fn nostr_connect(
        sender_keys: &Keys,
        receiver_pubkey: XOnlyPublicKey,
        msg: NostrConnectMessage,
    ) -> Result<Self, Error> {
        Ok(Self::new(
            Kind::NostrConnect,
            nip04::encrypt(&sender_keys.secret_key()?, &receiver_pubkey, msg.as_json())?,
            [Tag::public_key(receiver_pubkey)],
        ))
    }

    /// Live Event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/53.md>
    pub fn live_event(live_event: LiveEvent) -> Self {
        let tags: Vec<Tag> = live_event.into();
        Self::new(Kind::LiveEvent, "", tags)
    }

    /// Live Event Message
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/53.md>
    pub fn live_event_msg<S>(
        live_event_id: S,
        live_event_host: XOnlyPublicKey,
        content: S,
        relay_url: Option<Url>,
        mut tags: Vec<Tag>,
    ) -> Self
    where
        S: Into<String>,
    {
        tags.push(Tag::A {
            kind: Kind::LiveEvent,
            public_key: live_event_host,
            identifier: live_event_id.into(),
            relay_url: relay_url.map(|u| u.into()),
        });
        Self::new(Kind::LiveEventMessage, content, tags)
    }

    /// Create report event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/56.md>
    pub fn report<I, S>(tags: I, content: S) -> Self
    where
        I: IntoIterator<Item = Tag>,
        S: Into<String>,
    {
        Self::new(Kind::Reporting, content, tags)
    }

    /// Create public zap request event
    ///
    /// **This event MUST NOT be broadcasted to relays**, instead must be sent to a recipient's LNURL pay callback url.
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/57.md>
    #[cfg(feature = "nip57")]
    #[deprecated(since = "0.27.0", note = "Use `public_zap_request` instead")]
    pub fn new_zap_request(data: ZapRequestData) -> Self {
        let message: String = data.message.clone();
        let tags: Vec<Tag> = data.into();
        Self::new(Kind::ZapRequest, message, tags)
    }

    /// Create **public** zap request event
    ///
    /// **This event MUST NOT be broadcasted to relays**, instead must be sent to a recipient's LNURL pay callback url.
    ///
    /// To build a **private** or **anonymous** zap request, use:
    ///
    /// ```rust,no_run
    /// use nostr::prelude::*;
    ///
    /// # #[cfg(all(feature = "std", feature = "nip57"))]
    /// # fn main() {
    /// # let keys = Keys::generate();
    /// # let public_key = XOnlyPublicKey::from_bech32(
    /// # "npub14f8usejl26twx0dhuxjh9cas7keav9vr0v8nvtwtrjqx3vycc76qqh9nsy",
    /// # ).unwrap();
    /// # let relays = [UncheckedUrl::from("wss://relay.damus.io")];
    /// let data = ZapRequestData::new(public_key, relays).message("Zap!");
    ///
    /// let anon_zap: Event = nip57::anonymous_zap_request(data.clone()).unwrap();
    /// println!("Anonymous zap request: {anon_zap:#?}");
    ///
    /// let private_zap: Event = nip57::private_zap_request(data, &keys).unwrap();
    /// println!("Private zap request: {private_zap:#?}");
    /// # }
    ///
    /// # #[cfg(not(all(feature = "std", feature = "nip57")))]
    /// # fn main() {}
    /// ```
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/57.md>
    #[cfg(feature = "nip57")]
    pub fn public_zap_request(data: ZapRequestData) -> Self {
        let message: String = data.message.clone();
        let tags: Vec<Tag> = data.into();
        Self::new(Kind::ZapRequest, message, tags)
    }

    /// Create zap receipt event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/57.md>
    #[cfg(feature = "nip57")]
    pub fn zap_receipt<S>(bolt11: S, preimage: Option<S>, zap_request: Event) -> Self
    where
        S: Into<String>,
    {
        let mut tags = vec![
            Tag::Bolt11(bolt11.into()),
            Tag::Description(zap_request.as_json()),
        ];

        // add preimage tag if provided
        if let Some(pre_image_tag) = preimage {
            tags.push(Tag::Preimage(pre_image_tag.into()))
        }

        // add e tag
        if let Some(tag) = zap_request
            .iter_tags()
            .find(|t| t.kind() == TagKind::E)
            .cloned()
        {
            tags.push(tag);
        }

        // add p tag
        if let Some(tag) = zap_request
            .iter_tags()
            .find(|t| t.kind() == TagKind::P)
            .cloned()
        {
            tags.push(tag);
        }

        // add P tag
        tags.push(Tag::PublicKey {
            public_key: zap_request.author(),
            relay_url: None,
            alias: None,
            uppercase: true,
        });

        Self::new(Kind::ZapReceipt, "", tags)
    }

    /// Create zap receipt event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/57.md>
    #[cfg(feature = "nip57")]
    #[deprecated(since = "0.27.0", note = "Use `zap_receipt` instead")]
    pub fn new_zap_receipt<S>(bolt11: S, preimage: Option<S>, zap_request: Event) -> Self
    where
        S: Into<String>,
    {
        Self::zap_receipt(bolt11, preimage, zap_request)
    }

    /// Create a badge definition event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/58.md>
    ///
    /// # Example
    /// ```rust,no_run
    /// use nostr::{EventBuilder, ImageDimensions, UncheckedUrl};
    ///
    /// let badge_id = String::from("nostr-sdk-test-badge");
    /// let name = Some(String::from("Nostr SDK test badge"));
    /// let description = Some(String::from("This is a test badge"));
    /// let image_url = Some(UncheckedUrl::from("https://nostr.build/someimage/1337"));
    /// let image_size = Some(ImageDimensions::new(1024, 1024));
    /// let thumbs = vec![(
    ///     UncheckedUrl::from("https://nostr.build/somethumbnail/1337"),
    ///     Some(ImageDimensions::new(256, 256)),
    /// )];
    ///
    /// let event_builder =
    ///     EventBuilder::define_badge(badge_id, name, description, image_url, image_size, thumbs);
    /// ```
    pub fn define_badge<S>(
        badge_id: S,
        name: Option<S>,
        description: Option<S>,
        image: Option<UncheckedUrl>,
        image_dimensions: Option<ImageDimensions>,
        thumbnails: Vec<(UncheckedUrl, Option<ImageDimensions>)>,
    ) -> Self
    where
        S: Into<String>,
    {
        let mut tags: Vec<Tag> = Vec::new();

        // Set identifier tag
        tags.push(Tag::Identifier(badge_id.into()));

        // Set name tag
        if let Some(name) = name {
            tags.push(Tag::Name(name.into()));
        }

        // Set description tag
        if let Some(description) = description {
            tags.push(Tag::Description(description.into()));
        }

        // Set image tag
        if let Some(image) = image {
            let image_tag = if let Some(dimensions) = image_dimensions {
                Tag::Image(image, Some(dimensions))
            } else {
                Tag::Image(image, None)
            };
            tags.push(image_tag);
        }

        // Set thumbnail tags
        for (thumb, dimensions) in thumbnails.into_iter() {
            let thumb_tag = if let Some(dimensions) = dimensions {
                Tag::Thumb(thumb, Some(dimensions))
            } else {
                Tag::Thumb(thumb, None)
            };
            tags.push(thumb_tag);
        }

        Self::new(Kind::BadgeDefinition, "", tags)
    }

    /// Create a badge award event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/58.md>
    pub fn award_badge<I>(badge_definition: &Event, awarded_pubkeys: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = Tag>,
    {
        let mut tags = Vec::new();

        let badge_id = badge_definition
            .iter_tags()
            .find_map(|t| match t {
                Tag::Identifier(id) => Some(id),
                _ => None,
            })
            .ok_or(Error::NIP58(nip58::Error::IdentifierTagNotFound))?;

        // Add identity tag
        tags.push(Tag::A {
            kind: Kind::BadgeDefinition,
            public_key: badge_definition.author(),
            identifier: badge_id.clone(),
            relay_url: None,
        });

        // Add awarded pubkeys
        let ptags = awarded_pubkeys
            .into_iter()
            .filter(|p| matches!(p, Tag::PublicKey { .. }));
        tags.extend(ptags);

        // Build event
        Ok(Self::new(Kind::BadgeAward, "", tags))
    }

    /// Create a profile badges event
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/58.md>
    pub fn profile_badges(
        badge_definitions: Vec<Event>,
        badge_awards: Vec<Event>,
        pubkey_awarded: &XOnlyPublicKey,
    ) -> Result<Self, Error> {
        if badge_definitions.len() != badge_awards.len() {
            return Err(Error::NIP58(nip58::Error::InvalidLength));
        }

        let badge_awards: Vec<Event> = nip58::filter_for_kind(badge_awards, &Kind::BadgeAward);
        if badge_awards.is_empty() {
            return Err(Error::NIP58(Nip58Error::InvalidKind));
        }

        for award in badge_awards.iter() {
            if !award.iter_tags().any(|t| match t {
                Tag::PublicKey { public_key, .. } => public_key == pubkey_awarded,
                _ => false,
            }) {
                return Err(Error::NIP58(Nip58Error::BadgeAwardsLackAwardedPublicKey));
            }
        }

        let badge_definitions: Vec<Event> =
            nip58::filter_for_kind(badge_definitions, &Kind::BadgeDefinition);
        if badge_definitions.is_empty() {
            return Err(Error::NIP58(Nip58Error::InvalidKind));
        }

        // Add identifier `d` tag
        let id_tag: Tag = Tag::Identifier("profile_badges".to_string());
        let mut tags: Vec<Tag> = vec![id_tag];

        let badge_definitions_identifiers = badge_definitions.into_iter().filter_map(|event| {
            let id: String = event.identifier()?.to_string();
            Some((event, id))
        });

        let badge_awards_identifiers = badge_awards.into_iter().filter_map(|event| {
            let (_, relay_url) = nip58::extract_awarded_public_key(event.tags(), pubkey_awarded)?;
            let (id, a_tag) = event.iter_tags().find_map(|t| match t {
                Tag::A { identifier, .. } => Some((identifier.clone(), t.clone())),
                _ => None,
            })?;
            Some((event, id, a_tag, relay_url))
        });

        // This collection has been filtered for the needed tags
        let users_badges = core::iter::zip(badge_definitions_identifiers, badge_awards_identifiers);

        for (badge_definition, badge_award) in users_badges {
            match (badge_definition, badge_award) {
                ((_, identifier), (_, badge_id, ..)) if badge_id != identifier => {
                    return Err(Error::NIP58(Nip58Error::MismatchedBadgeDefinitionOrAward));
                }
                ((_, identifier), (badge_award_event, badge_id, a_tag, relay_url))
                    if badge_id == identifier =>
                {
                    let badge_definition_event_tag: Tag = a_tag;
                    let badge_award_event_tag: Tag = Tag::Event {
                        event_id: badge_award_event.id(),
                        relay_url,
                        marker: None,
                    };
                    tags.extend_from_slice(&[badge_definition_event_tag, badge_award_event_tag]);
                }
                _ => {}
            }
        }

        Ok(EventBuilder::new(Kind::ProfileBadges, "", tags))
    }

    /// Data Vending Machine - Job Request
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/90.md>
    pub fn job_request<I>(kind: Kind, tags: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = Tag>,
    {
        if kind.is_job_request() {
            Ok(Self::new(kind, "", tags))
        } else {
            Err(Error::WrongKind {
                received: kind,
                expected: WrongKindError::Range(NIP90_JOB_REQUEST_RANGE),
            })
        }
    }

    /// Data Vending Machine - Job Result
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/90.md>
    pub fn job_result(
        job_request: Event,
        amount_millisats: u64,
        bolt11: Option<String>,
    ) -> Result<Self, Error> {
        let kind: Kind = job_request.kind() + 1000;
        if kind.is_job_result() {
            let mut tags: Vec<Tag> = job_request
                .iter_tags()
                .filter_map(|t| {
                    if t.kind() == TagKind::I {
                        Some(t.clone())
                    } else {
                        None
                    }
                })
                .collect();
            tags.extend_from_slice(&[
                Tag::event(job_request.id()),
                Tag::public_key(job_request.author()),
                Tag::Request(job_request),
                Tag::Amount {
                    millisats: amount_millisats,
                    bolt11,
                },
            ]);
            Ok(Self::new(kind, "", tags))
        } else {
            Err(Error::WrongKind {
                received: kind,
                expected: WrongKindError::Range(NIP90_JOB_RESULT_RANGE),
            })
        }
    }

    /// Data Vending Machine - Job Feedback
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/90.md>
    pub fn job_feedback(
        job_request: &Event,
        status: DataVendingMachineStatus,
        extra_info: Option<String>,
        amount_millisats: u64,
        bolt11: Option<String>,
        payload: Option<String>,
    ) -> Self {
        let tags = [
            Tag::DataVendingMachineStatus { status, extra_info },
            Tag::event(job_request.id()),
            Tag::public_key(job_request.author()),
            Tag::Amount {
                millisats: amount_millisats,
                bolt11,
            },
        ];
        Self::new(Kind::JobFeedback, payload.unwrap_or_default(), tags)
    }

    /// File metadata
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/94.md>
    pub fn file_metadata<S>(description: S, metadata: FileMetadata) -> Self
    where
        S: Into<String>,
    {
        let tags: Vec<Tag> = metadata.into();
        Self::new(Kind::FileMetadata, description.into(), tags)
    }

    /// HTTP Auth
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/98.md>
    pub fn http_auth(data: HttpData) -> Self {
        let tags: Vec<Tag> = data.into();
        Self::new(Kind::HttpAuth, "", tags)
    }

    /// Set stall data
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/15.md>
    pub fn stall_data(data: StallData) -> Self {
        let tags: Vec<Tag> = data.clone().into();
        Self::new(Kind::SetStall, data, tags)
    }

    /// Set stall data
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/15.md>
    #[deprecated(since = "0.27.0", note = "Use `stall_data` instead")]
    pub fn new_stall_data(data: StallData) -> Self {
        Self::stall_data(data)
    }

    /// Set product data
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/15.md>
    pub fn product_data(data: ProductData) -> Self {
        let tags: Vec<Tag> = data.clone().into();
        Self::new(Kind::SetProduct, data, tags)
    }

    /// Set product data
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/15.md>
    #[deprecated(since = "0.27.0", note = "Use `product_data` instead")]
    pub fn new_product_data(data: ProductData) -> Self {
        Self::product_data(data)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "std")]
    use core::str::FromStr;

    #[cfg(feature = "std")]
    use bitcoin::secp256k1::SecretKey;

    use super::*;

    #[test]
    #[cfg(feature = "std")]
    fn round_trip() {
        let keys = Keys::new(
            SecretKey::from_str("6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e")
                .unwrap(),
        );

        let event = EventBuilder::text_note("hello", [])
            .to_event(&keys)
            .unwrap();

        let serialized = event.as_json();
        let deserialized = Event::from_json(serialized).unwrap();

        assert_eq!(event, deserialized);
    }

    #[test]
    #[cfg(all(feature = "std", feature = "nip04"))]
    fn test_encrypted_direct_msg() {
        let sender_keys = Keys::new(
            SecretKey::from_str("6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e")
                .unwrap(),
        );
        let receiver_keys = Keys::new(
            SecretKey::from_str("7b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e")
                .unwrap(),
        );

        let content = "Mercury, the Winged Messenger";
        let event = EventBuilder::encrypted_direct_msg(
            &sender_keys,
            receiver_keys.public_key(),
            content,
            None,
        )
        .unwrap()
        .to_event(&sender_keys)
        .unwrap();

        event.verify().unwrap();
    }

    #[test]
    #[cfg(feature = "nip57")]
    fn test_zap_event_builder() {
        let bolt11 = String::from("lnbc10u1p3unwfusp5t9r3yymhpfqculx78u027lxspgxcr2n2987mx2j55nnfs95nxnzqpp5jmrh92pfld78spqs78v9euf2385t83uvpwk9ldrlvf6ch7tpascqhp5zvkrmemgth3tufcvflmzjzfvjt023nazlhljz2n9hattj4f8jq8qxqyjw5qcqpjrzjqtc4fc44feggv7065fqe5m4ytjarg3repr5j9el35xhmtfexc42yczarjuqqfzqqqqqqqqlgqqqqqqgq9q9qxpqysgq079nkq507a5tw7xgttmj4u990j7wfggtrasah5gd4ywfr2pjcn29383tphp4t48gquelz9z78p4cq7ml3nrrphw5w6eckhjwmhezhnqpy6gyf0");
        let preimage = Some(String::from(
            "5d006d2cf1e73c7148e7519a4c68adc81642ce0e25a432b2434c99f97344c15f",
        ));
        let zap_request_json = String::from("{\"pubkey\":\"32e1827635450ebb3c5a7d12c1f8e7b2b514439ac10a67eef3d9fd9c5c68e245\",\"content\":\"\",\"id\":\"d9cc14d50fcb8c27539aacf776882942c1a11ea4472f8cdec1dea82fab66279d\",\"created_at\":1674164539,\"sig\":\"77127f636577e9029276be060332ea565deaf89ff215a494ccff16ae3f757065e2bc59b2e8c113dd407917a010b3abd36c8d7ad84c0e3ab7dab3a0b0caa9835d\",\"kind\":9734,\"tags\":[[\"e\",\"3624762a1274dd9636e0c552b53086d70bc88c165bc4dc0f9e836a1eaf86c3b8\"],[\"p\",\"32e1827635450ebb3c5a7d12c1f8e7b2b514439ac10a67eef3d9fd9c5c68e245\"],[\"relays\",\"wss://relay.damus.io\",\"wss://nostr-relay.wlvs.space\",\"wss://nostr.fmt.wiz.biz\",\"wss://relay.nostr.bg\",\"wss://nostr.oxtr.dev\",\"wss://nostr.v0l.io\",\"wss://brb.io\",\"wss://nostr.bitcoiner.social\",\"ws://monad.jb55.com:8080\",\"wss://relay.snort.social\"]]}");
        let zap_request_event: Event = Event::from_json(zap_request_json).unwrap();
        let event_builder = EventBuilder::zap_receipt(bolt11, preimage, zap_request_event);

        assert_eq!(6, event_builder.tags.len());

        let has_preimage_tag = event_builder
            .tags
            .clone()
            .iter()
            .find(|t| matches!(t, Tag::Preimage(_)))
            .is_some();

        assert_eq!(true, has_preimage_tag);
    }

    #[test]
    #[cfg(feature = "nip57")]
    fn test_zap_event_builder_without_preimage() {
        let bolt11 = String::from("lnbc10u1p3unwfusp5t9r3yymhpfqculx78u027lxspgxcr2n2987mx2j55nnfs95nxnzqpp5jmrh92pfld78spqs78v9euf2385t83uvpwk9ldrlvf6ch7tpascqhp5zvkrmemgth3tufcvflmzjzfvjt023nazlhljz2n9hattj4f8jq8qxqyjw5qcqpjrzjqtc4fc44feggv7065fqe5m4ytjarg3repr5j9el35xhmtfexc42yczarjuqqfzqqqqqqqqlgqqqqqqgq9q9qxpqysgq079nkq507a5tw7xgttmj4u990j7wfggtrasah5gd4ywfr2pjcn29383tphp4t48gquelz9z78p4cq7ml3nrrphw5w6eckhjwmhezhnqpy6gyf0");
        let preimage = None;
        let zap_request_json = String::from("{\"pubkey\":\"32e1827635450ebb3c5a7d12c1f8e7b2b514439ac10a67eef3d9fd9c5c68e245\",\"content\":\"\",\"id\":\"d9cc14d50fcb8c27539aacf776882942c1a11ea4472f8cdec1dea82fab66279d\",\"created_at\":1674164539,\"sig\":\"77127f636577e9029276be060332ea565deaf89ff215a494ccff16ae3f757065e2bc59b2e8c113dd407917a010b3abd36c8d7ad84c0e3ab7dab3a0b0caa9835d\",\"kind\":9734,\"tags\":[[\"e\",\"3624762a1274dd9636e0c552b53086d70bc88c165bc4dc0f9e836a1eaf86c3b8\"],[\"p\",\"32e1827635450ebb3c5a7d12c1f8e7b2b514439ac10a67eef3d9fd9c5c68e245\"],[\"relays\",\"wss://relay.damus.io\",\"wss://nostr-relay.wlvs.space\",\"wss://nostr.fmt.wiz.biz\",\"wss://relay.nostr.bg\",\"wss://nostr.oxtr.dev\",\"wss://nostr.v0l.io\",\"wss://brb.io\",\"wss://nostr.bitcoiner.social\",\"ws://monad.jb55.com:8080\",\"wss://relay.snort.social\"]]}");
        let zap_request_event = Event::from_json(zap_request_json).unwrap();
        let event_builder = EventBuilder::zap_receipt(bolt11, preimage, zap_request_event);

        assert_eq!(5, event_builder.tags.len());
        let has_preimage_tag = event_builder
            .tags
            .clone()
            .iter()
            .find(|t| matches!(t, Tag::Preimage(_)))
            .is_some();

        assert_eq!(false, has_preimage_tag);
    }

    #[test]
    fn test_badge_definition_event_builder_badge_id_only() {
        let badge_id = String::from("bravery");
        let event_builder =
            EventBuilder::define_badge(badge_id, None, None, None, None, Vec::new());

        let has_id = event_builder
            .tags
            .clone()
            .iter()
            .find(|t| matches!(t, Tag::Identifier(_)))
            .is_some();
        assert_eq!(true, has_id);

        assert_eq!(Kind::BadgeDefinition, event_builder.kind);
    }

    #[test]
    fn test_badge_definition_event_builder_full() {
        let badge_id = String::from("bravery");
        let name = Some(String::from("Bravery"));
        let description = Some(String::from("Brave pubkey"));
        let image_url = Some(UncheckedUrl::from("https://nostr.build/someimage/1337"));
        let image_size = Some(ImageDimensions::new(1024, 1024));
        let thumbs = vec![(
            UncheckedUrl::from("https://nostr.build/somethumbnail/1337"),
            Some(ImageDimensions::new(256, 256)),
        )];

        let event_builder =
            EventBuilder::define_badge(badge_id, name, description, image_url, image_size, thumbs);

        let has_id = event_builder
            .tags
            .clone()
            .iter()
            .find(|t| matches!(t, Tag::Identifier(_)))
            .is_some();
        assert_eq!(true, has_id);

        assert_eq!(Kind::BadgeDefinition, event_builder.kind);
    }

    #[test]
    #[cfg(feature = "std")]
    fn test_badge_award_event_builder() {
        let keys = Keys::generate();
        let pub_key = keys.public_key();

        // Set up badge definition
        let badge_definition_event_json = format!(
            r#"{{
                "id": "4d16822726cefcb45768988c6451b6de5a20b504b8df85efe0808caf346e167c",
                "pubkey": "{}",
                "created_at": 1677921759,
                "kind": 30009,
                "tags": [
                  ["d", "bravery"],
                  ["name", "Bravery"],
                  ["description", "A brave soul"]
                ],
                "content": "",
                "sig": "cf154350a615f0355d165b52c7ecccce563d9a935801181e9016d077f38d31a1dc992a757ef8d652a416885f33d836cf408c79f5d983d6f1f03c966ace946d59"
              }}"#,
            pub_key.to_string()
        );
        let badge_definition_event: Event =
            serde_json::from_str(&badge_definition_event_json).unwrap();

        // Set up goal event
        let example_event_json = format!(
            r#"{{
            "content": "",
            "id": "378f145897eea948952674269945e88612420db35791784abf0616b4fed56ef7",
            "kind": 8,
            "pubkey": "{}",
            "sig": "fd0954de564cae9923c2d8ee9ab2bf35bc19757f8e328a978958a2fcc950eaba0754148a203adec29b7b64080d0cf5a32bebedd768ea6eb421a6b751bb4584a8",
            "created_at": 1671739153,
            "tags": [
                ["a", "30009:{}:bravery"],
                ["p", "32e1827635450ebb3c5a7d12c1f8e7b2b514439ac10a67eef3d9fd9c5c68e245", "wss://nostr.oxtr.dev"],
                ["p", "232a4ba3df82ccc252a35abee7d87d1af8fc3cc749e4002c3691434da692b1df", "wss://nostr.oxtr.dev"]
            ]
            }}"#,
            pub_key.to_string(),
            pub_key.to_string()
        );
        let example_event: Event = serde_json::from_str(&example_event_json).unwrap();

        // Create new event with the event builder
        let awarded_pubkeys = vec![
            Tag::PublicKey {
                public_key: XOnlyPublicKey::from_str(
                    "32e1827635450ebb3c5a7d12c1f8e7b2b514439ac10a67eef3d9fd9c5c68e245",
                )
                .unwrap(),
                relay_url: Some(UncheckedUrl::from_str("wss://nostr.oxtr.dev").unwrap()),
                alias: None,
                uppercase: false,
            },
            Tag::PublicKey {
                public_key: XOnlyPublicKey::from_str(
                    "232a4ba3df82ccc252a35abee7d87d1af8fc3cc749e4002c3691434da692b1df",
                )
                .unwrap(),
                relay_url: Some(UncheckedUrl::from_str("wss://nostr.oxtr.dev").unwrap()),
                alias: None,
                uppercase: false,
            },
        ];
        let event_builder: Event =
            EventBuilder::award_badge(&badge_definition_event, awarded_pubkeys)
                .unwrap()
                .to_event(&keys)
                .unwrap();

        assert_eq!(event_builder.kind(), Kind::BadgeAward);
        assert_eq!(event_builder.content(), "");
        assert_eq!(event_builder.tags(), example_event.tags());
    }

    #[test]
    #[cfg(feature = "std")]
    fn test_profile_badges() {
        // The pubkey used for profile badges event
        let keys = Keys::generate();
        let pub_key = keys.public_key();

        // Create badge 1
        let badge_one_keys = Keys::generate();
        let badge_one_pubkey = badge_one_keys.public_key();
        let relay_url = UncheckedUrl::from_str("wss://nostr.oxtr.dev").unwrap();

        let awarded_pubkeys = vec![
            Tag::PublicKey {
                public_key: pub_key.clone(),
                relay_url: Some(relay_url.clone()),
                alias: None,
                uppercase: false,
            },
            Tag::PublicKey {
                public_key: XOnlyPublicKey::from_str(
                    "232a4ba3df82ccc252a35abee7d87d1af8fc3cc749e4002c3691434da692b1df",
                )
                .unwrap(),
                relay_url: Some(UncheckedUrl::from_str("wss://nostr.oxtr.dev").unwrap()),
                alias: None,
                uppercase: false,
            },
        ];
        let bravery_badge_event =
            self::EventBuilder::define_badge("bravery", None, None, None, None, Vec::new())
                .to_event(&badge_one_keys)
                .unwrap();
        let bravery_badge_award =
            self::EventBuilder::award_badge(&bravery_badge_event, awarded_pubkeys.clone())
                .unwrap()
                .to_event(&badge_one_keys)
                .unwrap();

        // Badge 2
        let badge_two_keys = Keys::generate();
        let badge_two_pubkey = badge_two_keys.public_key();

        let honor_badge_event =
            self::EventBuilder::define_badge("honor", None, None, None, None, Vec::new())
                .to_event(&badge_two_keys)
                .unwrap();
        let honor_badge_award =
            self::EventBuilder::award_badge(&honor_badge_event, awarded_pubkeys.clone())
                .unwrap()
                .to_event(&badge_two_keys)
                .unwrap();

        let example_event_json = format!(
            r#"{{
            "content":"",
            "id": "378f145897eea948952674269945e88612420db35791784abf0616b4fed56ef7",
            "kind": 30008,
            "pubkey": "{}",
            "sig":"fd0954de564cae9923c2d8ee9ab2bf35bc19757f8e328a978958a2fcc950eaba0754148a203adec29b7b64080d0cf5a32bebedd768ea6eb421a6b751bb4584a8",
            "created_at":1671739153,
            "tags":[
                ["d", "profile_badges"],
                ["a", "30009:{}:bravery"],
                ["e", "{}", "wss://nostr.oxtr.dev"],
                ["a", "30009:{}:honor"],
                ["e", "{}", "wss://nostr.oxtr.dev"]
            ]
            }}"#,
            pub_key.to_string(),
            badge_one_pubkey.to_string(),
            bravery_badge_award.id().to_string(),
            badge_two_pubkey.to_string(),
            honor_badge_award.id().to_string(),
        );
        let example_event: Event = serde_json::from_str(&example_event_json).unwrap();

        let badge_definitions = vec![bravery_badge_event, honor_badge_event];
        let badge_awards = vec![bravery_badge_award, honor_badge_award];
        let profile_badges =
            EventBuilder::profile_badges(badge_definitions, badge_awards, &pub_key)
                .unwrap()
                .to_event(&keys)
                .unwrap();

        assert_eq!(profile_badges.kind(), Kind::ProfileBadges);
        assert_eq!(profile_badges.tags(), example_event.tags());
    }
}
