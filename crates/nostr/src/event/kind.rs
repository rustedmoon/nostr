// Copyright (c) 2022-2023 Yuki Kishimoto
// Copyright (c) 2023-2024 Rust Nostr Developers
// Distributed under the MIT software license

//! Kind

use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::num::ParseIntError;
use core::ops::{Add, Range};
use core::str::FromStr;

use serde::de::{Deserialize, Deserializer, Error, Visitor};
use serde::ser::{Serialize, Serializer};

/// NIP90 - Job request range
pub const NIP90_JOB_REQUEST_RANGE: Range<u64> = 5_000..5_999;
/// NIP90 - Job result range
pub const NIP90_JOB_RESULT_RANGE: Range<u64> = 6_000..6_999;
/// Regular range
pub const REGULAR_RANGE: Range<u64> = 1_000..10_000;
/// Replaceable range
pub const REPLACEABLE_RANGE: Range<u64> = 10_000..20_000;
/// Ephemeral range
pub const EPHEMERAL_RANGE: Range<u64> = 20_000..30_000;
/// Parameterized replaceable range
pub const PARAMETERIZED_REPLACEABLE_RANGE: Range<u64> = 30_000..40_000;

/// Event [`Kind`]
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// Metadata (NIP01 and NIP05)
    Metadata,
    /// Short Text Note (NIP01)
    TextNote,
    /// Recommend Relay (NIP01 - deprecated)
    RecommendRelay,
    /// Contacts (NIP02)
    ContactList,
    /// OpenTimestamps Attestations (NIP03)
    OpenTimestamps,
    /// Encrypted Direct Messages (NIP04)
    EncryptedDirectMessage,
    /// Event Deletion (NIP09)
    EventDeletion,
    /// Repost (NIP18)
    Repost,
    /// Generic Repost (NIP18)
    GenericRepost,
    /// Reaction (NIP25)
    Reaction,
    /// Badge Award (NIP58)
    BadgeAward,
    /// Channel Creation (NIP28)
    ChannelCreation,
    /// Channel Metadata (NIP28)
    ChannelMetadata,
    /// Channel Message (NIP28)
    ChannelMessage,
    /// Channel Hide Message (NIP28)
    ChannelHideMessage,
    /// Channel Mute User (NIP28)
    ChannelMuteUser,
    /// Public Chat Reserved (NIP28)
    PublicChatReserved45,
    /// Public Chat Reserved (NIP28)
    PublicChatReserved46,
    /// Public Chat Reserved (NIP28)
    PublicChatReserved47,
    /// Public Chat Reserved (NIP28)
    PublicChatReserved48,
    /// Public Chat Reserved (NIP28)
    PublicChatReserved49,
    /// Wallet Service Info (NIP47)
    WalletConnectInfo,
    /// Reporting (NIP56)
    Reporting,
    /// Zap Private Message (NIP57)
    ZapPrivateMessage,
    /// Zap Request (NIP57)
    ZapRequest,
    /// Zap Receipt (NIP57)
    ZapReceipt,
    /// Mute List
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    MuteList,
    /// Pin List
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    PinList,
    /// Bookmarks
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    Bookmarks,
    /// Communities
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    Communities,
    /// Public Chats
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    PublicChats,
    /// Blocked Relays
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    BlockedRelays,
    /// Search Relays
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    SearchRelays,
    /// Simple Groups
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    SimpleGroups,
    /// Interests
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    Interests,
    /// Emojis
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    Emojis,
    /// Follow Sets
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    FollowSets,
    /// Relay Sets
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    RelaySets,
    /// Bookmark Sets
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    BookmarkSets,
    /// Articles Curation Sets
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    ArticlesCurationSets,
    /// Videos Curation Sets
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    VideosCurationSets,
    /// Interest Sets
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    InterestSets,
    /// Emoji Sets
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    EmojiSets,
    /// Release Artifact Sets
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/51.md>
    ReleaseArtifactSets,
    /// Relay List Metadata (NIP65)
    RelayList,
    /// Client Authentication (NIP42)
    Authentication,
    /// Wallet Connect Request (NIP47)
    WalletConnectRequest,
    /// Wallet Connect Response (NIP47)
    WalletConnectResponse,
    /// Nostr Connect (NIP46)
    NostrConnect,
    /// Live Event (NIP53)
    LiveEvent,
    /// Live Event Message (NIP53)
    LiveEventMessage,
    /// Profile Badges (NIP58)
    ProfileBadges,
    /// Badge Definition (NIP58)
    BadgeDefinition,
    /// Seal (NIP59)
    Seal,
    /// Gift Wrap (NIP59)
    GiftWrap,
    /// GiftWrapped Sealed Direct message
    SealedDirect,
    /// Long-form Text Note (NIP23)
    LongFormTextNote,
    /// Application-specific Data (NIP78)
    ApplicationSpecificData,
    /// File Metadata (NIP94)
    FileMetadata,
    /// HTTP Auth (NIP98)
    HttpAuth,
    /// Set stall (NIP15)
    SetStall,
    /// Set product (NIP15)
    SetProduct,
    /// Job Feedback (NIP90)
    JobFeedback,
    /// Regular Events (must be between 5000 and <=5999)
    JobRequest(u16),
    /// Regular Events (must be between 6000 and <=6999)
    JobResult(u16),
    /// Regular Events (must be between 1000 and <=9999)
    Regular(u16),
    /// Replaceable event (must be between 10000 and <20000)
    Replaceable(u16),
    /// Ephemeral event (must be between 20000 and <30000)
    Ephemeral(u16),
    /// Parameterized replaceable event (must be between 30000 and <40000)
    ParameterizedReplaceable(u16),
    /// Custom
    Custom(u64),
}

impl PartialEq<Kind> for Kind {
    fn eq(&self, other: &Kind) -> bool {
        self.as_u64() == other.as_u64()
    }
}

impl Eq for Kind {}

impl PartialOrd for Kind {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Kind {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_u64().cmp(&other.as_u64())
    }
}

impl Hash for Kind {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.as_u64().hash(state);
    }
}

impl Kind {
    /// Get [`Kind`] as `u32`
    pub fn as_u32(&self) -> u32 {
        self.as_u64() as u32
    }

    /// Get [`Kind`] as `u64`
    pub fn as_u64(&self) -> u64 {
        (*self).into()
    }

    /// Get [`Kind`] as `f64`
    pub fn as_f64(&self) -> f64 {
        self.as_u64() as f64
    }

    /// Check if [`Kind`] is a NIP90 job request
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/90.md>
    #[inline]
    pub fn is_job_request(&self) -> bool {
        NIP90_JOB_REQUEST_RANGE.contains(&self.as_u64())
    }

    /// Check if [`Kind`] is a NIP90 job result
    ///
    /// <https://github.com/nostr-protocol/nips/blob/master/90.md>
    #[inline]
    pub fn is_job_result(&self) -> bool {
        NIP90_JOB_RESULT_RANGE.contains(&self.as_u64())
    }

    /// Check if [`Kind`] is `Regular`
    #[inline]
    pub fn is_regular(&self) -> bool {
        REGULAR_RANGE.contains(&self.as_u64())
    }

    /// Check if [`Kind`] is `Replaceable`
    #[inline]
    pub fn is_replaceable(&self) -> bool {
        matches!(self, Kind::Metadata)
            || matches!(self, Kind::ContactList)
            || matches!(self, Kind::ChannelMetadata)
            || REPLACEABLE_RANGE.contains(&self.as_u64())
    }

    /// Check if [`Kind`] is `Ephemeral`
    #[inline]
    pub fn is_ephemeral(&self) -> bool {
        EPHEMERAL_RANGE.contains(&self.as_u64())
    }

    /// Check if [`Kind`] is `Parameterized replaceable`
    #[inline]
    pub fn is_parameterized_replaceable(&self) -> bool {
        PARAMETERIZED_REPLACEABLE_RANGE.contains(&self.as_u64())
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_u64())
    }
}

impl From<u64> for Kind {
    #[inline]
    fn from(u: u64) -> Self {
        match u {
            0 => Self::Metadata,
            1 => Self::TextNote,
            2 => Self::RecommendRelay,
            3 => Self::ContactList,
            1040 => Self::OpenTimestamps,
            4 => Self::EncryptedDirectMessage,
            5 => Self::EventDeletion,
            6 => Self::Repost,
            16 => Self::GenericRepost,
            7 => Self::Reaction,
            8 => Self::BadgeAward,
            40 => Self::ChannelCreation,
            41 => Self::ChannelMetadata,
            42 => Self::ChannelMessage,
            43 => Self::ChannelHideMessage,
            44 => Self::ChannelMuteUser,
            45 => Self::PublicChatReserved45,
            46 => Self::PublicChatReserved46,
            47 => Self::PublicChatReserved47,
            48 => Self::PublicChatReserved48,
            49 => Self::PublicChatReserved49,
            13194 => Self::WalletConnectInfo,
            1984 => Self::Reporting,
            9733 => Self::ZapPrivateMessage,
            9734 => Self::ZapRequest,
            9735 => Self::ZapReceipt,
            10000 => Self::MuteList,
            10001 => Self::PinList,
            10003 => Self::Bookmarks,
            10004 => Self::Communities,
            10005 => Self::PublicChats,
            10006 => Self::BlockedRelays,
            10007 => Self::SearchRelays,
            10009 => Self::SimpleGroups,
            10015 => Self::Interests,
            10030 => Self::Emojis,
            10002 => Self::RelayList,
            22242 => Self::Authentication,
            23194 => Self::WalletConnectRequest,
            23195 => Self::WalletConnectResponse,
            24133 => Self::NostrConnect,
            30000 => Self::FollowSets,
            30002 => Self::RelaySets,
            30003 => Self::BookmarkSets,
            30004 => Self::ArticlesCurationSets,
            30005 => Self::VideosCurationSets,
            30015 => Self::InterestSets,
            30030 => Self::EmojiSets,
            30063 => Self::ReleaseArtifactSets,
            30311 => Self::LiveEvent,
            1311 => Self::LiveEventMessage,
            30008 => Self::ProfileBadges,
            30009 => Self::BadgeDefinition,
            13 => Self::Seal,
            1059 => Self::GiftWrap,
            14 => Self::SealedDirect,
            30017 => Self::SetStall,
            30018 => Self::SetProduct,
            30023 => Self::LongFormTextNote,
            30078 => Self::ApplicationSpecificData,
            1063 => Self::FileMetadata,
            27235 => Self::HttpAuth,
            7000 => Self::JobFeedback,
            x if (NIP90_JOB_REQUEST_RANGE).contains(&x) => Self::JobRequest(x as u16),
            x if (NIP90_JOB_RESULT_RANGE).contains(&x) => Self::JobResult(x as u16),
            x if (REGULAR_RANGE).contains(&x) => Self::Regular(x as u16),
            x if (REPLACEABLE_RANGE).contains(&x) => Self::Replaceable(x as u16),
            x if (EPHEMERAL_RANGE).contains(&x) => Self::Ephemeral(x as u16),
            x if (PARAMETERIZED_REPLACEABLE_RANGE).contains(&x) => {
                Self::ParameterizedReplaceable(x as u16)
            }
            x => Self::Custom(x),
        }
    }
}

impl From<Kind> for u64 {
    fn from(e: Kind) -> u64 {
        match e {
            Kind::Metadata => 0,
            Kind::TextNote => 1,
            Kind::RecommendRelay => 2,
            Kind::ContactList => 3,
            Kind::OpenTimestamps => 1040,
            Kind::EncryptedDirectMessage => 4,
            Kind::EventDeletion => 5,
            Kind::Repost => 6,
            Kind::GenericRepost => 16,
            Kind::Reaction => 7,
            Kind::BadgeAward => 8,
            Kind::ChannelCreation => 40,
            Kind::ChannelMetadata => 41,
            Kind::ChannelMessage => 42,
            Kind::ChannelHideMessage => 43,
            Kind::ChannelMuteUser => 44,
            Kind::PublicChatReserved45 => 45,
            Kind::PublicChatReserved46 => 46,
            Kind::PublicChatReserved47 => 47,
            Kind::PublicChatReserved48 => 48,
            Kind::PublicChatReserved49 => 49,
            Kind::WalletConnectInfo => 13194,
            Kind::Reporting => 1984,
            Kind::ZapPrivateMessage => 9733,
            Kind::ZapRequest => 9734,
            Kind::ZapReceipt => 9735,
            Kind::MuteList => 10000,
            Kind::PinList => 10001,
            Kind::Bookmarks => 10003,
            Kind::Communities => 10004,
            Kind::PublicChats => 10005,
            Kind::BlockedRelays => 10006,
            Kind::SearchRelays => 10007,
            Kind::SimpleGroups => 10009,
            Kind::Interests => 10015,
            Kind::Emojis => 10030,
            Kind::RelayList => 10002,
            Kind::Authentication => 22242,
            Kind::WalletConnectRequest => 23194,
            Kind::WalletConnectResponse => 23195,
            Kind::NostrConnect => 24133,
            Kind::FollowSets => 30000,
            Kind::RelaySets => 30002,
            Kind::BookmarkSets => 30003,
            Kind::ArticlesCurationSets => 30004,
            Kind::VideosCurationSets => 30005,
            Kind::InterestSets => 30015,
            Kind::EmojiSets => 30030,
            Kind::ReleaseArtifactSets => 30063,
            Kind::LiveEvent => 30311,
            Kind::LiveEventMessage => 1311,
            Kind::ProfileBadges => 30008,
            Kind::BadgeDefinition => 30009,
            Kind::Seal => 13,
            Kind::GiftWrap => 1059,
            Kind::SealedDirect => 14,
            Kind::SetStall => 30017,
            Kind::SetProduct => 30018,
            Kind::LongFormTextNote => 30023,
            Kind::ApplicationSpecificData => 30078,
            Kind::FileMetadata => 1063,
            Kind::HttpAuth => 27235,
            Kind::JobFeedback => 7000,
            Kind::JobRequest(u) => u as u64,
            Kind::JobResult(u) => u as u64,
            Kind::Regular(u) => u as u64,
            Kind::Replaceable(u) => u as u64,
            Kind::Ephemeral(u) => u as u64,
            Kind::ParameterizedReplaceable(u) => u as u64,
            Kind::Custom(u) => u,
        }
    }
}

impl From<f64> for Kind {
    fn from(kind: f64) -> Self {
        Self::from(kind as u64)
    }
}

impl FromStr for Kind {
    type Err = ParseIntError;

    fn from_str(kind: &str) -> Result<Self, Self::Err> {
        let kind: u64 = kind.parse()?;
        Ok(Self::from(kind))
    }
}

impl Add<u64> for Kind {
    type Output = Self;

    fn add(self, rhs: u64) -> Self::Output {
        let kind = self.as_u64();
        Kind::from(kind + rhs)
    }
}

impl Serialize for Kind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(From::from(*self))
    }
}

impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_u64(KindVisitor)
    }
}

struct KindVisitor;

impl Visitor<'_> for KindVisitor {
    type Value = Kind;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "an unsigned number")
    }

    fn visit_u64<E>(self, v: u64) -> Result<Kind, E>
    where
        E: Error,
    {
        Ok(From::<u64>::from(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equal_kind() {
        assert_eq!(Kind::Custom(20100), Kind::Custom(20100));
        assert_eq!(Kind::Custom(20100), Kind::Ephemeral(20100));
        assert_eq!(Kind::TextNote, Kind::Custom(1));
        assert_eq!(Kind::ParameterizedReplaceable(30017), Kind::SetStall);
        assert_eq!(Kind::ParameterizedReplaceable(30018), Kind::SetProduct);
    }

    #[test]
    fn test_not_equal_kind() {
        assert_ne!(Kind::Custom(20100), Kind::Custom(2000));
        assert_ne!(Kind::Authentication, Kind::EncryptedDirectMessage);
        assert_ne!(Kind::TextNote, Kind::Custom(2));
    }

    #[test]
    fn test_kind_is_parameterized_replaceable() {
        assert!(Kind::ParameterizedReplaceable(32122).is_parameterized_replaceable());
        assert!(!Kind::ParameterizedReplaceable(1).is_parameterized_replaceable());
    }
}
