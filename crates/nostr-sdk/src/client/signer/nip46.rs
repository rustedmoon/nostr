// Copyright (c) 2022-2023 Yuki Kishimoto
// Copyright (c) 2023-2024 Rust Nostr Developers
// Distributed under the MIT software license

//! Nostr Connect Client (NIP46)
//!
//! <https://github.com/nostr-protocol/nips/blob/master/46.md>

use std::sync::Arc;
use std::time::Duration;

use async_utility::time;
use nostr::nips::nip04;
use nostr::nips::nip46::{Message, NostrConnectMetadata, NostrConnectURI, Request, Response};
use nostr::secp256k1::XOnlyPublicKey;
use nostr::{
    serde_json, ClientMessage, EventBuilder, Filter, JsonUtil, Keys, Kind, SubscriptionId,
    Timestamp, Url,
};
use tokio::sync::Mutex;

#[cfg(feature = "blocking")]
use crate::client::blocking::Client as BlockingClient;
use crate::client::{Client, Error};
use crate::relay::RelayPoolNotification;
#[cfg(feature = "blocking")]
use crate::RUNTIME;

/// NIP46 Signer
#[derive(Debug, Clone)]
pub struct Nip46Signer {
    relay_url: Url,
    app_keys: Keys,
    signer_public_key: Arc<Mutex<Option<XOnlyPublicKey>>>,
}

impl Nip46Signer {
    /// New NIP46 remote signer
    pub fn new(relay_url: Url, app_keys: Keys, signer_public_key: Option<XOnlyPublicKey>) -> Self {
        Self {
            relay_url,
            app_keys,
            signer_public_key: Arc::new(Mutex::new(signer_public_key)),
        }
    }

    /// Get signer relay [`Url`]
    pub fn relay_url(&self) -> Url {
        self.relay_url.clone()
    }

    /// Get signer [`XOnlyPublicKey`]
    pub async fn signer_public_key(&self) -> Option<XOnlyPublicKey> {
        let pubkey = self.signer_public_key.lock().await;
        *pubkey
    }

    pub(crate) async fn set_signer_public_key(&self, public_key: XOnlyPublicKey) {
        let mut pubkey = self.signer_public_key.lock().await;
        *pubkey = Some(public_key);
    }

    /// Compose Nostr Connect URI
    pub fn nostr_connect_uri(&self, metadata: NostrConnectMetadata) -> NostrConnectURI {
        NostrConnectURI::with_metadata(self.app_keys.public_key(), self.relay_url(), metadata)
    }
}

impl Client {
    /// Request the [`XOnlyPublicKey`] of the signer (sent with `Connect` request)
    ///
    /// Call not required if you already added in `Client::with_remote_signer`.
    ///
    /// # Example
    /// ```rust,no_run
    /// use std::time::Duration;
    ///
    /// use nostr_sdk::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let app_keys = Keys::generate();
    ///     let relay_url = Url::parse("wss://relay.example.com").unwrap();
    ///     let signer = Nip46Signer::new(relay_url, app_keys, None);
    ///     let client = Client::new(signer);
    ///
    ///     // Signer public key MUST be requested in this case
    ///     client
    ///         .req_signer_public_key(Some(Duration::from_secs(180)))
    ///         .await
    ///         .unwrap();
    /// }
    /// ```
    ///
    /// # Example
    /// ```rust,no_run
    /// use std::str::FromStr;
    ///
    /// use nostr_sdk::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let app_keys = Keys::generate();
    ///     let relay_url = Url::parse("wss://relay.example.com").unwrap();
    ///     let signer_public_key = XOnlyPublicKey::from_str(
    ///         "b2d670de53b27691c0c3400225b65c35a26d06093bcc41f48ffc71e0907f9d4a",
    ///     )
    ///     .unwrap();
    ///     let signer = Nip46Signer::new(relay_url, app_keys, Some(signer_public_key));
    ///
    ///     // Signer public key request isn't needed since we already added in client constructor
    ///     let _client = Client::new(signer);
    /// }
    /// ```
    pub async fn req_signer_public_key(&self, timeout: Option<Duration>) -> Result<(), Error> {
        let signer: Nip46Signer = self.signer().await?.try_into()?;

        if signer.signer_public_key().await.is_none() {
            let public_key = signer.app_keys.public_key();
            let secret_key = signer.app_keys.secret_key()?;

            let id = SubscriptionId::generate();
            let filter = Filter::new()
                .pubkey(public_key)
                .kind(Kind::NostrConnect)
                .since(Timestamp::now());

            // Subscribe
            self.send_msg_to(
                signer.relay_url(),
                ClientMessage::req(id.clone(), vec![filter]),
            )
            .await?;

            let mut notifications = self.notifications();
            time::timeout(timeout, async {
                while let Ok(notification) = notifications.recv().await {
                    if let RelayPoolNotification::Event { event, .. } = notification {
                        if event.kind() == Kind::NostrConnect {
                            let msg: String =
                                nip04::decrypt(&secret_key, event.author_ref(), event.content())?;
                            let msg = Message::from_json(msg)?;
                            if let Ok(Request::Connect(pk)) = msg.to_request() {
                                signer.set_signer_public_key(pk).await;
                                break;
                            }
                        }
                    }
                }

                Ok::<(), Error>(())
            })
            .await
            .ok_or(Error::Timeout)??;

            // Unsubscribe
            self.send_msg_to(signer.relay_url(), ClientMessage::close(id))
                .await?;
        }

        Ok(())
    }

    /// Send NIP46 [`Request`] to signer
    pub async fn send_req_to_signer(
        &self,
        req: Request,
        timeout: Option<Duration>,
    ) -> Result<Response, Error> {
        let signer: Nip46Signer = self.signer().await?.try_into()?;

        let signer_pubkey = signer
            .signer_public_key()
            .await
            .ok_or(Error::SignerPublicKeyNotFound)?;

        let msg = Message::request(req.clone());
        let req_id = msg.id();

        let public_key = signer.app_keys.public_key();
        let secret_key = signer.app_keys.secret_key()?;

        // Build request
        let event = EventBuilder::nostr_connect(&signer.app_keys, signer_pubkey, msg)?
            .to_event(&signer.app_keys)?;

        // Send request to signer
        self.send_event_to(signer.relay_url(), event).await?;

        let sub_id = SubscriptionId::generate();
        let filter = Filter::new()
            .pubkey(public_key)
            .kind(Kind::NostrConnect)
            .since(Timestamp::now());

        // Subscribe
        self.send_msg_to(
            signer.relay_url(),
            ClientMessage::req(sub_id.clone(), vec![filter]),
        )
        .await?;

        let mut notifications = self.notifications();
        let future = async {
            while let Ok(notification) = notifications.recv().await {
                if let RelayPoolNotification::Event { event, .. } = notification {
                    if event.kind() == Kind::NostrConnect {
                        let msg = nip04::decrypt(&secret_key, event.author_ref(), event.content())?;
                        let msg = Message::from_json(msg)?;

                        tracing::debug!("New message received: {msg:?}");

                        if let Message::Response { id, result, error } = &msg {
                            if &req_id == id {
                                if let Some(result) = result {
                                    let res = match req {
                                        Request::Describe => Response::Describe(
                                            serde_json::from_value(result.to_owned())?,
                                        ),
                                        Request::GetPublicKey => {
                                            let pubkey = serde_json::from_value(result.to_owned())?;
                                            Response::GetPublicKey(pubkey)
                                        }
                                        Request::SignEvent(_) => {
                                            let sig = serde_json::from_value(result.to_owned())?;
                                            Response::SignEvent(sig)
                                        }
                                        Request::Delegate { .. } => Response::Delegate(
                                            serde_json::from_value(result.to_owned())?,
                                        ),
                                        Request::Nip04Encrypt { .. } => Response::Nip04Encrypt(
                                            serde_json::from_value(result.to_owned())?,
                                        ),
                                        Request::Nip04Decrypt { .. } => Response::Nip04Decrypt(
                                            serde_json::from_value(result.to_owned())?,
                                        ),
                                        Request::SignSchnorr { .. } => Response::SignSchnorr(
                                            serde_json::from_value(result.to_owned())?,
                                        ),
                                        _ => break,
                                    };

                                    // Unsubscribe
                                    self.send_msg_to(
                                        signer.relay_url(),
                                        ClientMessage::close(sub_id.clone()),
                                    )
                                    .await?;
                                    return Ok(res);
                                }

                                if let Some(error) = error {
                                    // Unsubscribe
                                    self.send_msg_to(
                                        signer.relay_url(),
                                        ClientMessage::close(sub_id.clone()),
                                    )
                                    .await?;
                                    return Err(Error::Response(error.to_owned()));
                                }

                                break;
                            }
                        }
                    }
                }
            }

            Err(Error::Generic)
        };

        let res: Result<Response, Error> =
            time::timeout(timeout, future).await.ok_or(Error::Timeout)?;

        // Unsubscribe
        self.send_msg_to(signer.relay_url(), ClientMessage::close(sub_id))
            .await?;

        res
    }
}

#[cfg(feature = "blocking")]
impl BlockingClient {
    #[allow(missing_docs)]
    pub fn req_signer_public_key(&self, timeout: Option<Duration>) -> Result<(), Error> {
        RUNTIME.block_on(async { self.client.req_signer_public_key(timeout).await })
    }

    #[allow(missing_docs)]
    pub fn send_req_to_signer(
        &self,
        req: Request,
        timeout: Option<Duration>,
    ) -> Result<Response, Error> {
        RUNTIME.block_on(async { self.client.send_req_to_signer(req, timeout).await })
    }
}
