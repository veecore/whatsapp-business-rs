use axum::{
    Router,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Deserializer};
use sha2::Sha256;
use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    future::{Ready, ready},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{net::TcpListener, sync::Notify};

use crate::{
    Identity, IdentityMetadata, IdentityRef, IdentityType, Timestamp, Waba,
    client::AppSecret,
    error::Error,
    message::{Content, Context as MessageContext, Message, MessageRef, MessageStatus},
    server::{
        ConversationOrigin, ErrorContext, Event, EventContext, Handler, IncomingMessage,
        MessageUpdate, MessageUpdateContext, PartnerAdded, Server, WabaEvent,
    },
};

use super::{FromResponse, client::ContentResponse};

#[cfg(feature = "incoming_message_ext")]
use crate::client::Client;

#[derive(thiserror::Error, Debug)]
#[error("{0}")]
struct RawError(String);

macro_rules! error {
    ($state:expr => $($tt:tt)*) => {{
        tokio::spawn(async move {
            $state
                .handler
                .handle_error(ErrorContext {_priv: ()}, Box::new(RawError(format!($($tt)*))))
                .await;
        })
    }}
}

/// Internal shared state for the webhook logic.
/// This struct is used by both the high-level server and the low-level service.
pub(crate) struct InnerServer<H, V, W> {
    #[cfg(feature = "incoming_message_ext")]
    pub(crate) client: Client,
    pub(crate) handler: H,
    pub(crate) app_secret: V,
    pub(crate) verify_token: W,
}

impl Server {
    pub(crate) async fn serve_inner<H>(
        self,
        handler: H,
        #[cfg(feature = "incoming_message_ext")] client: Client,
        notify: Option<Arc<Notify>>,
    ) -> Result<(), Error>
    where
        H: Handler + 'static,
    {
        let listener = TcpListener::bind(&self.config.endpoint)
            .await
            .map_err(|err| Error::Network(err.into()))?;

        macro_rules! run {
            (match ($($b:tt)*) {$(
                $app_secret:tt, $verify_token:tt => $route:expr
            ),*}) => {
                match ($($b)*) {
                    $(
                        (run!(@option $app_secret), run!(@option $verify_token))
                        => {
                            let app = Router::new()
                                .route(&self.config.route_path, $route)
                                .with_state(
                                    InnerServer {
                                        #[cfg(feature = "incoming_message_ext")]
                                        client,
                                        handler,
                                        app_secret: $app_secret,
                                        verify_token: $verify_token,
                                    }
                                    .into(),
                                );
                            if let Some(notify) = notify {
                                tokio::spawn(async move {
                                    // Give Axum a moment to fully stand up
                                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                                    notify.notify_one();
                                });
                            };

                            if let Some(shutdown) = self.config.shutdown {
                                axum::serve(listener, app)
                                    .with_graceful_shutdown(shutdown)
                                    .await
                            } else {
                                axum::serve(listener, app).await
                            }
                        }
                    )*
                }
            };
            {
                @option ()
            } => {None};
            {
                @option $value:tt
            } => {Some($value)}
        }

        let r = run! {
            match (self.config.app_secret, self.config.verify_token) {
                (), () => post(handle_webhook),
                (), verify_token => post(handle_webhook).get(handle_verification),
                app_secret, () => post(handle_webhook_verify_payload),
                app_secret, verify_token => post(handle_webhook_verify_payload).get(handle_verification)
            }
        };

        r.map_err(|err| Error::Network(err.into()))
    }
}

// Verification handler
pub(crate) fn handle_verification<V, H>(
    State(state): State<Arc<InnerServer<H, V, String>>>,
    query: Query<HashMap<String, String>>,
) -> Ready<(StatusCode, Cow<'static, str>)>
where
    V: Send + Sync + 'static,
    H: Handler + 'static,
{
    let challenge = WebhookChallenge {
        _hub_mode: query.get("hub.mode").cloned().unwrap_or_default(),
        hub_challenge: query.get("hub.challenge").cloned().unwrap_or_default(),
        hub_verify_token: query.get("hub.verify_token").cloned().unwrap_or_default(),
    };

    // Verify the token matches our secret and echo
    if challenge.hub_verify_token == *state.verify_token {
        ready((StatusCode::OK, challenge.hub_challenge.into()))
    } else {
        error!(state =>
            "Invalid verification token. Expected: '{}', Received: '{}'",
            state.verify_token, challenge.hub_verify_token
        );
        ready((StatusCode::FORBIDDEN, "Invalid verification token".into()))
    }
}

pub(crate) fn handle_webhook_verify_payload<W, H>(
    state: State<Arc<InnerServer<H, AppSecret, W>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Ready<(StatusCode, Cow<'static, str>)>
where
    W: Send + Sync + 'static,
    H: Handler + 'static,
{
    if let Err(e) = verify_signature(&state.app_secret, &headers, &body) {
        error!(state => "Signature verification failed: {e:#?}");
        return ready((
            StatusCode::UNAUTHORIZED,
            "Signature verification failed".into(),
        ));
    }

    handle_webhook(state, body)
}

// Webhook handler
#[inline]
pub(crate) fn handle_webhook<V, W, H>(
    State(state): State<Arc<InnerServer<H, V, W>>>,
    body: Bytes,
) -> Ready<(StatusCode, Cow<'static, str>)>
where
    V: Send + Sync + 'static,
    W: Send + Sync + 'static,
    H: Handler + 'static,
{
    let payload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            error!(state => "JSON parsing failed: {:?}", e);
            return ready((
                StatusCode::BAD_REQUEST,
                "Invalid JSON payload.\
         Please ensure the body is valid JSON."
                    .into(),
            ));
        }
    };

    if let Err(err) = process_events(
        payload,
        |ctx, event| {
            let state_clone = state.clone();
            tokio::spawn(async move { state_clone.handler.handle(ctx, event).await });
        },
        #[cfg(feature = "incoming_message_ext")]
        &state.client,
    ) {
        error!(state => "Event processing failed: {err:#?}");
        ready((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to process webhook event \
             due to an internal server error. Check server logs for details."
                .into(),
        ))
    } else {
        ready((StatusCode::OK, "".into()))
    }
}

// Signature verification
fn verify_signature(secret: &AppSecret, headers: &HeaderMap, body: &[u8]) -> Result<(), String> {
    let signature = headers
        .get("x-hub-signature-256")
        .ok_or_else(|| "Missing X-Hub-Signature-256 header".to_owned())?
        .to_str()
        .map_err(|_| "Invalid signature header".to_owned())?;

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.0.as_bytes())
        .map_err(|_| "Invalid webhook secret".to_owned())?;

    mac.update(body);
    let expected_signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    // Use constant-time comparison to prevent timing attacks
    if subtle::ConstantTimeEq::ct_eq(signature.as_bytes(), expected_signature.as_bytes()).into() {
        Ok(())
    } else {
        Err(format!(
            "Signature mismatch. Received: {signature}, Expected: {expected_signature}.\
         This usually indicates an incorrect webhook secret or a tampered payload.",
        ))
    }
}

/// Webhook payload structures
#[derive(Deserialize, Debug)]
#[serde(bound = "'de: 'a")]
pub(crate) struct WebhookPayload<'a> {
    /// The specific webhook a business is subscribed to. The webhook is whatsapp_business_account.    
    pub object: Cow<'a, str>,

    /// An array containing an object describing the changes.
    /// Multiple changes from different objects that are of the
    /// same type may be batched together.
    #[serde(rename = "entry")]
    pub entries: Vec<WebhookEntry<'a>>,
}

/// Webhook entry container
#[derive(Deserialize, Debug)]
#[serde(bound = "'de: 'a")]
pub(crate) struct WebhookEntry<'a> {
    /// actually WHATSAPP_BUSINESS_ACCOUNT_ID or BUSINESS_PORTFOLIO_ID
    /// not entry id.
    pub id: Cow<'a, str>,

    /// A UNIX timestamp indicating when the Event Notification was sent
    /// (not when the change that triggered the notification occurred).
    #[serde(default)]
    pub time: Option<Timestamp>,

    /// An array containing an object describing the changed fields and
    /// their new values.
    ///
    /// Only included if you enable the Include Values setting when
    /// configuring the Webhooks product in your app's App Dashboard.
    pub changes: Vec<WebhookEntryChange<'a>>,
}

#[derive(Deserialize, Debug)]
#[serde(bound = "'de: 'a")]
pub(crate) struct WebhookEntryChange<'a> {
    #[serde(rename = "field")]
    pub _field: Cow<'a, str>,

    pub value: WebhookChangeValue<'a>,
}

#[derive(Deserialize, Debug)]
#[serde(bound = "'de: 'a")]
pub(crate) struct WebhookChangeValue<'a> {
    #[serde(flatten, default)]
    pub context: Option<Context<'a>>,
    #[serde(flatten)]
    pub event: WebhookEvent<'a>,
}

// NOTE: Should be flattened when used
// I wonder why whatsapp won't just put these in the message
// body.
#[derive(Deserialize, Debug)]
#[serde(bound = "'de: 'a")]
pub(crate) struct Context<'a> {
    pub messaging_product: Cow<'a, str>,

    #[serde(rename = "contacts", default)]
    pub user: Option<VecDeque<UserContactContext>>,

    #[serde(rename = "metadata")]
    pub us: BusinessContactContext<'a>,
}

/// Struct with information for the customer who sent
/// a message to the busines
#[derive(Deserialize, Debug)]
pub(crate) struct UserContactContext {
    /// The customer's WhatsApp ID.
    ///
    /// A business can respond to a customer using this ID.
    ///
    /// This ID may not match the customer's phone number,
    /// which is returned by the API as input when sending
    /// a message to the customer.
    pub wa_id: String,

    // Additional unique, alphanumeric identifier for a WhatsApp user.
    #[serde(default, rename = "user_id")]
    pub _user_id: Option<String>,

    pub profile: UserProfileContext,
}

/// A customer profile object
#[doc(alias = "contacts")]
#[derive(Deserialize, Debug)]
pub(crate) struct UserProfileContext {
    /// The customer's name.
    pub name: String,
}

/// A metadata object describing the business subscribed to the webhook.
#[doc(alias = "profile")]
#[derive(Deserialize, Debug)]
#[serde(bound = "'de: 'a")]
pub(crate) struct BusinessContactContext<'a> {
    pub display_phone_number: Cow<'a, str>,

    /// ID for the phone number.
    ///
    /// A business can respond to a message using this ID.    
    pub phone_number_id: Cow<'a, str>,
}

// NOTE: Should be flattened when used
#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
#[serde(bound = "'de: 'a")]
pub(crate) enum WebhookEvent<'a> {
    Messages(Vec<MessageEvent<'a>>),
    Statuses(Vec<MessageStatusEvent>),
    WabaInfo(PartnerAdded),
}

#[derive(Deserialize, Debug)]
#[serde(bound = "'de: 'a")]
pub(crate) struct MessageEvent<'a> {
    /// The ID for the message that was received by the business.
    ///
    /// You could use messages endpoint to mark this specific message as read.
    pub from: String,
    pub id: String,
    pub timestamp: Timestamp,
    #[serde(default)]
    pub context: MessageContext,

    // We have to include system messages - I'm tired
    #[serde(flatten)]
    pub content: ContentResponse<'a>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct MessageStatusEvent {
    /// For a status to be read, it must have been delivered. In some scenarios, such as when a user is
    /// in the chat screen and a message arrives, the message is delivered and read almost simultaneously.
    /// In this or other similar scenarios, the delivered notification will not be sent back, as it is implied
    /// that a message has been delivered if it has been read. The reason for this behavior is internal optimization.
    pub status: MessageStatus,

    pub id: String,

    /// Date for the status message
    #[serde(default)]
    pub timestamp: Option<Timestamp>,

    /// The customer's WhatsApp ID. A business can respond to a customer using this ID.
    /// This ID may not match the customer's phone number, which is returned by the API as
    /// input when sending a message to the customer.
    #[serde(default)]
    pub recipient_id: Option<String>,

    #[serde(flatten)]
    pub context: MessageUpdateContext,
}

fn process_events<'a>(
    payload: WebhookPayload<'a>,
    new_event: impl Fn(EventContext, Event),
    #[cfg(feature = "incoming_message_ext")] client: &Client,
) -> Result<(), String> {
    if payload.object != "whatsapp_business_account" {
        return Err(format!(
            "Invalid object type '{}'. Expected 'whatsapp_business_account'.",
            payload.object
        ));
    }

    for entry in payload.entries {
        let webhook_received_at = entry.time.unwrap_or_else(|| Timestamp {
            inner: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        });

        let add_event = |event: Event| {
            new_event(
                EventContext {
                    event_received_at: webhook_received_at,
                    waba: if let Event::Waba(WabaEvent::PartnerAdded(ref info)) = event {
                        // The entry.id for waba is called portfolio id
                        Waba::new(&info.waba_id)
                    } else {
                        Waba::new(&*entry.id)
                    },
                },
                event,
            )
        };

        for change in entry.changes {
            // let messaging_product = change.value.context.messaging_product.to_owned();
            match change.value.event {
                WebhookEvent::Messages(messages) => {
                    messages.build_events(
                        change.value.context.ok_or_else(|| {
                            "Missing 'context' field for Messages event in\
                             webhook payload. This field is required to build message events."
                                .to_owned()
                        })?,
                        webhook_received_at,
                        add_event,
                        #[cfg(feature = "incoming_message_ext")]
                        client.clone(),
                    )?
                    // .await?
                }
                WebhookEvent::Statuses(update) => {
                    update.build_events(
                        change.value.context.ok_or_else(|| {
                            "Missing 'context' field for Statuses event in\
                             webhook payload. This field is required to build\
                              status update events."
                                .to_owned()
                        })?,
                        webhook_received_at,
                        add_event,
                        #[cfg(feature = "incoming_message_ext")]
                        client.clone(),
                    )?
                    // .await?
                }
                WebhookEvent::WabaInfo(waba_info) => {
                    add_event(Event::Waba(WabaEvent::PartnerAdded(waba_info)))
                }
            };
        }
    }

    Ok(())
}

/// Event Parser Trait
trait EventBuilder {
    /*async */
    fn build_events(
        self,
        context: Context<'_>,
        received_at: Timestamp,
        add_event: impl Fn(Event), // take Result<Event, String> here for granular control?
        #[cfg(feature = "incoming_message_ext")] client: Client,
    ) -> Result<(), String>;
}

impl EventBuilder for Vec<MessageEvent<'_>> {
    /*async */
    fn build_events(
        self,
        context: Context<'_>,
        received_at: Timestamp,
        add_event: impl Fn(Event),
        #[cfg(feature = "incoming_message_ext")] client: Client,
    ) -> Result<(), String> {
        if context.messaging_product != "whatsapp" {
            return Err(format!(
                "Unsupported 'messaging_product': '{}'.\
                 Only 'whatsapp' is currently supported for this webhook event.",
                context.messaging_product
            ));
        }

        let recipient_contact = context.us;

        let mut senders = context.user.ok_or_else(|| {
            "Missing 'user' field in context for message events.\
             This field should contain sender contact information."
                .to_owned()
        })?;

        for message in self {
            let sender_contact = senders
                .pop_front()
                .ok_or_else(|| "Sender contact not included in payload for message.".to_owned())?;

            let sender = Identity {
                phone_id: sender_contact.wa_id,
                identity_type: IdentityType::User,
                metadata: IdentityMetadata {
                    name: Some(sender_contact.profile.name),
                    phone_number: Some(message.from),
                },
            };

            let recipient = Identity {
                phone_id: recipient_contact.phone_number_id.to_string(),
                identity_type: IdentityType::Business,
                metadata: IdentityMetadata {
                    phone_number: Some(recipient_contact.display_phone_number.to_string()),
                    ..Default::default()
                },
            };

            let content =
                Content::from_response(message.content).map_err(|err| format!("{err}"))?;

            let msg = Message {
                content,
                context: message.context,
                sender,
                recipient,
                id: message.id,
                timestamp: message.timestamp,
                message_status: MessageStatus::Delivered,
            };

            // FIXME: Have message manager here... we'd prolly have sender cloned
            // to avoid life issue
            add_event(Event::IncomingMessage(IncomingMessage {
                message: msg,
                timestamp: Some(received_at),
                #[cfg(feature = "incoming_message_ext")]
                client: client.clone(),
            }));
        }

        Ok(())
    }
}

impl EventBuilder for Vec<MessageStatusEvent> {
    /*async */
    fn build_events(
        self,
        context: Context<'_>,
        _received_at: Timestamp,
        add_event: impl Fn(Event),
        #[cfg(feature = "incoming_message_ext")] _client: Client,
    ) -> Result<(), String> {
        if context.messaging_product != "whatsapp" {
            return Err(format!(
                "Unsupported 'messaging_product': '{}'.\
                        Only 'whatsapp' is currently supported for this webhook event.",
                context.messaging_product
            ));
        }

        let sender_contact = context.us;

        for update in self {
            let sender = IdentityRef::business(&*sender_contact.phone_number_id);
            // IdentityRef {
            //     id: sender_contact.phone_number_id.to_owned(),
            //     identity_type: IdentityType::Business,
            //     // Just the phone_number.... When we have more fields sent to us
            //     // we'll put whole Identity in MessageRef... thankfully, we can view
            //     // IdentityRef from Identity so we won't have to deprecate the getters
            //     // metadata: IdentityMetadata {
            //     //     phone_number: Some(sender_contact.display_phone_number.to_owned()),
            //     //     ..Default::default()
            //     // },
            // };

            let recipient = update.recipient_id.map(IdentityRef::user);

            let message = MessageRef {
                message_id: update.id,
                sender: Some(sender),
                recipient,
            };

            let update = MessageUpdate {
                message,
                status: update.status,
                timestamp: update.timestamp,
                context: update.context,
            };
            add_event(Event::MessageUpdate(update));
        }

        Ok(())
    }
}

// Webhook challenge struct
#[derive(Debug, Deserialize)]
pub(crate) struct WebhookChallenge {
    #[serde(rename = "hub.mode")]
    _hub_mode: String,
    #[serde(rename = "hub.challenge")]
    hub_challenge: String,
    #[serde(rename = "hub.verify_token")]
    hub_verify_token: String,
}

pub(crate) fn deserialize_origin<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ConversationOrigin>, D::Error> {
    #[derive(Deserialize)]
    struct Object {
        r#type: ConversationOrigin,
    }

    let helper = <Option<Object>>::deserialize(deserializer)?;
    Ok(helper.map(|o| o.r#type))
}

#[cfg(test)]
mod tests {
    use super::WebhookPayload;

    // We only test for successful deserialization
    macro_rules! test_payload {
        (|$title:ident|: $($payload:tt)*) => {
            #[test]
            fn $title() {
                serde_json::from_str::<WebhookPayload<'_>>(stringify!($($payload)*)).unwrap();
            }
        }
    }

    test_payload! {
        |unknown|: {
          "object": "whatsapp_business_account",
          "entry": [
            {
              "id": "WHATSAPP_BUSINESS_ACCOUNT_ID",
              "changes": [
                {
                  "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                      "display_phone_number": "PHONE_NUMBER",
                      "phone_number_id": "PHONE_NUMBER_ID"
                    },
                    "contacts": [
                      {
                        "profile": {
                          "name": "NAME"
                        },
                        "wa_id": "WHATSAPP_ID"
                      }
                    ],
                    "messages": [
                      {
                        "from": "PHONE_NUMBER",
                        "id": "wamid.ID",
                        "timestamp": "1731617831",
                        "errors": [
                          {
                            "code": 131051,
                            "details": "Message type is not currently supported",
                            "title": "Unsupported message type"
                          }
                        ],
                        "type": "unknown"
                      }
                    ]
                  },
                  "field": "messages"
                }
              ]
            }
          ]
        }
    }

    test_payload! {
        |waba|: {
          "entry": [
            {
              "id": "35602282435505",
              "time": 1731617831,
              "changes": [
                {
                  "value": {
                    "event": "PARTNER_ADDED",
                    "waba_info": {
                      "waba_id": "495709166956424",
                      "owner_business_id": "942647313864044"
                    }
                  },
                  "field": "account_update"
                }
              ]
            }
          ],
          "object": "whatsapp_business_account"
        }
    }

    test_payload! {
        |order_message|: {
          "object": "whatsapp_business_account",
          "entry": [
            {
              "id": "8856996819413533",
              "changes": [
                {
                  "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                      "display_phone_number": "16505553333",
                      "phone_number_id": "phone-number-id"
                    },
                    "contacts": [
                      {
                        "profile": {
                          "name": "Kerry Fisher"
                        },
                        "wa_id": "16315551234"
                      }
                    ],
                    "messages": [
                      {
                        "from": "16315551234",
                        "id": "wamid.ABGGFlCGg0cvAgo6cHbBhfK5760V",
                        "order": {
                          "catalog_id": "the-catalog_id",
                          "product_items": [
                            {
                              "product_retailer_id": "the-product-SKU-identifier",
                              "quantity": 50,
                              "item_price": 308,
                              "currency": "USD"
                            }
                          ],
                          "text": "text-message-sent-along-with-the-order"
                        },
                        "context": {
                          "from": "16315551234",
                          "id": "wamid.gBGGFlaCGg0xcvAdgmZ9plHrf2Mh-o"
                        },
                        "timestamp": 1603069091,
                        "type": "order"
                      }
                    ]
                  },
                  "field": "messages"
                }
              ]
            }
          ]
        }
    }

    test_payload! {
        |contextual_message|: {
          "object": "whatsapp_business_account",
          "entry": [
            {
              "id": "ID",
              "changes": [
                {
                  "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                      "display_phone_number": "PHONE_NUMBER",
                      "phone_number_id": "PHONE_NUMBER_ID"
                    },
                    "contacts": [
                      {
                        "profile": {
                          "name": "NAME"
                        },
                        "wa_id": "PHONE_NUMBER_ID"
                      }
                    ],
                    "messages": [
                      {
                        "from": "PHONE_NUMBER",
                        "id": "wamid.ID",
                        "text": {
                          "body": "MESSAGE_TEXT"
                        },
                        "context": {
                          "from": "PHONE_NUMBER",
                          "id": "wamid.ID",
                          "referred_product": {
                            "catalog_id": "CATALOG_ID",
                            "product_retailer_id": "PRODUCT_ID"
                          }
                        },
                        "timestamp": 1738499404,
                        "type": "text"
                      }
                    ]
                  },
                  "field": "messages"
                }
              ]
            }
          ]
        }
    }

    test_payload! {
        |interactive_button_reply|: {
          "object": "whatsapp_business_account",
          "entry": [
            {
              "id": "WHATSAPP_BUSINESS_ACCOUNT_ID",
              "changes": [
                {
                  "value": {
                      "messaging_product": "whatsapp",
                      "metadata": {
                           "display_phone_number": "PHONE_NUMBER",
                           "phone_number_id": "PHONE_NUMBER_ID"
                      },
                      "contacts": [
                        {
                          "profile": {
                            "name": "NAME"
                          },
                          "wa_id": "PHONE_NUMBER_ID"
                        }
                      ],
                      "messages": [
                        {
                          "from": "PHONE_NUMBER_ID",
                          "id": "wamid.ID",
                          "timestamp": 17893000,
                          "interactive": {
                            "button_reply": {
                              "id": "unique-button-identifier-here",
                              "title": "button-text"
                            },
                            "type": "button_reply"
                          },
                          "type": "interactive"
                        }
                      ]
                  },
                  "field": "messages"
                }
              ]
            }
          ]
        }
    }

    test_payload! {
        |interactive_list_reply|: {
          "object": "whatsapp_business_account",
          "entry": [
            {
              "id": "WHATSAPP_BUSINESS_ACCOUNT_ID",
              "changes": [
                {
                  "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                      "display_phone_number": "PHONE_NUMBER",
                      "phone_number_id": "PHONE_NUMBER_ID"
                    },
                    "contacts": [
                      {
                        "profile": {
                          "name": "NAME"
                        },
                        "wa_id": "PHONE_NUMBER_ID"
                      }
                    ],
                    "messages": [
                      {
                        "from": "PHONE_NUMBER_ID",
                        "id": "wamid.ID",
                        "timestamp": 178999000,
                        "interactive": {
                          "list_reply": {
                            "id": "list_reply_id",
                            "title": "list_reply_title",
                            "description": "list_reply_description"
                          },
                          "type": "list_reply"
                        },
                        "type": "interactive"
                      }
                    ]
                  },
                  "field": "messages"
                }
              ]
            }
          ]
        }
    }

    // test_payload! {
    //     |interactive_flow_reply|: {
    //         "object": "whatsapp_business_account",
    //         "entry": [
    //         {
    //             "id": "WHATSAPP_BUSINESS_ACCOUNT_ID",
    //             "changes": [
    //             {
    //                 "value":
    //                 {
    //                     "messaging_product": "whatsapp",
    //                     "metadata":
    //                     {
    //                         "display_phone_number": "PHONE_NUMBER",
    //                         "phone_number_id": "PHONE_NUMBER_ID"
    //                     },
    //                     "contacts": [
    //                     {
    //                         "profile":
    //                         {
    //                             "name": "NAME"
    //                         },
    //                         "wa_id": "WHATSAPP_ID"
    //                     }],
    //                     "messages": [
    //                     {
    //                         "context":
    //                         {
    //                             "from": "16315558151",
    //                             "id": "gBGGEiRVVgBPAgm7FUgc73noXjo"
    //                         },
    //                         "from": "<USER_ACCOUNT_NUMBER>",
    //                         "id": "<MESSAGE_ID>",
    //                         "type": "interactive",
    //                         "interactive":
    //                         {
    //                             "type": "nfm_reply",
    //                             "nfm_reply":
    //                             {
    //                                 "name": "flow",
    //                                 "body": "Sent",
    //                                 "response_json": "{\"flow_token\": \"<FLOW_TOKEN>\", \"optional_param1\": \"<value1>\", \"optional_param2\": \"<value2>\"}"
    //                             }
    //                         },
    //                         "timestamp": "<MESSAGE_SEND_TIMESTAMP>"
    //                     }]
    //                 },
    //                 "field": "messages"
    //             }]
    //         }]
    //     }
    // }

    test_payload! {
        |location_message|: {
          "object": "whatsapp_business_account",
          "entry": [
            {
              "id": "WHATSAPP_BUSINESS_ACCOUNT_ID",
              "changes": [
                {
                  "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                      "display_phone_number": "PHONE_NUMBER",
                      "phone_number_id": "PHONE_NUMBER_ID"
                    },
                    "contacts": [
                      {
                        "profile": {
                          "name": "NAME"
                        },
                        "wa_id": "WHATSAPP_ID"
                      }
                    ],
                    "messages": [
                      {
                        "from": "PHONE_NUMBER",
                        "id": "wamid.ID",
                        "timestamp": 1238838484,
                        "location": {
                          "latitude": -233,
                          "longitude": 40,
                          "name": "LOCATION_NAME",
                          "address": "LOCATION_ADDRESS"
                        }
                      }
                    ]
                  },
                  "field": "messages"
                }
              ]
            }
          ]
        }
    }

    test_payload! {
        |media_message_sticker|: {
          "object": "whatsapp_business_account",
          "entry": [
            {
              "id": "ID",
              "changes": [
                {
                  "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                      "display_phone_number": "PHONE_NUMBER",
                      "phone_number_id": "PHONE_NUMBER_ID"
                    },
                    "contacts": [
                      {
                        "profile": {
                          "name": "NAME"
                        },
                        "wa_id": "ID"
                      }
                    ],
                    "messages": [
                      {
                        "from": "SENDER_PHONE_NUMBER",
                        "id": "wamid.ID",
                        "timestamp": 179398488,
                        "type": "sticker",
                        "sticker": {
                          "mime_type": "image/webp",
                          "sha256": "HASH",
                          "id": "ID"
                        }
                      }
                    ]
                  },
                  "field": "messages"
                }
              ]
            }
          ]
        }
    }

    test_payload! {
        |media_message_image|: {
          "object": "whatsapp_business_account",
          "entry": [
            {
              "id": "WHATSAPP_BUSINESS_ACCOUNT_ID",
              "changes": [
                {
                  "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                      "display_phone_number": "PHONE_NUMBER",
                      "phone_number_id": "PHONE_NUMBER_ID"
                    },
                    "contacts": [
                      {
                        "profile": {
                          "name": "NAME"
                        },
                        "wa_id": "WHATSAPP_ID"
                      }
                    ],
                    "messages": [
                      {
                        "from": "PHONE_NUMBER",
                        "id": "wamid.ID",
                        "timestamp": 1293944940,
                        "type": "image",
                        "image": {
                          "caption": "CAPTION",
                          "mime_type": "image/jpeg",
                          "sha256": "IMAGE_HASH",
                          "id": "ID"
                        }
                      }
                    ]
                  },
                  "field": "messages"
                }
              ]
            }
          ]
        }
    }

    test_payload! {
        |reaction_message|: {
            "object": "whatsapp_business_account",
            "entry": [
                {
                    "id": "WHATSAPP_BUSINESS_ACCOUNT_ID",
                    "changes": [
                        {
                            "value": {
                                "messaging_product": "whatsapp",
                                "metadata": {
                                    "display_phone_number": "PHONE_NUMBER",
                                    "phone_number_id": "PHONE_NUMBER_ID"
                                },
                                "contacts": [
                                    {
                                        "profile": {
                                            "name": "NAME"
                                        },
                                        "wa_id": "PHONE_NUMBER"
                                    }
                                ],
                                "messages": [
                                    {
                                        "from": "PHONE_NUMBER",
                                        "id": "wamid.ID",
                                        "timestamp": 17494004003,
                                        "reaction": {
                                            "message_id": "MESSAGE_ID",
                                            "emoji": "😀"
                                        },
                                        "type": "reaction"
                                    }
                                ]
                            },
                            "field": "messages"
                        }
                    ]
                }
            ]
        }
    }

    test_payload! {
        |text_message|: {
          "object": "whatsapp_business_account",
          "entry": [
            {
              "id": "<WHATSAPP_BUSINESS_ACCOUNT_ID>",
              "changes": [
                {
                  "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                      "display_phone_number": "<BUSINESS_DISPLAY_PHONE_NUMBER>",
                      "phone_number_id": "<BUSINESS_PHONE_NUMBER_ID>"
                    },
                    "contacts": [
                      {
                        "profile": {
                          "name": "<WHATSAPP_USER_NAME>"
                        },
                        "wa_id": "<WHATSAPP_USER_ID>"
                      }
                    ],
                    "messages": [
                      {
                        "from": "<WHATSAPP_USER_PHONE_NUMBER>",
                        "id": "<WHATSAPP_MESSAGE_ID>",
                        "timestamp": 1378940040,
                        "text": {
                          "body": "<MESSAGE_BODY_TEXT>"
                        },
                        "type": "text"
                      }
                    ]
                  },
                  "field": "messages"
                }
              ]
            }
          ]
        }
    }

    test_payload! {
        |message_update_failed|: {
          "object": "whatsapp_business_account",
          "entry": [
            {
              "id": "<WHATSAPP_BUSINESS_ACCOUNT_ID>",
              "changes": [
                {
                  "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                      "display_phone_number": "<BUSINESS_PHONE_NUMBER>",
                      "phone_number_id": "<BUSINESS_PHONE_NUMBER_ID>"
                    },
                    "statuses": [
                      {
                        "id": "<WHATSAPP_MESSAGE_ID>",
                        "status": "failed",
                        "timestamp": 12999990,
                        "recipient_id": "<WHATSAPP_USER_PHONE_NUMBER>",
                        "errors": [
                          {
                            "code": 131050,
                            "title": "Unable to deliver the message. This recipient has chosen to stop receiving marketing messages on WhatsApp from your business"
                          }
                        ]
                      }
                    ]
                  },
                  "field": "messages"
                }
              ]
            }
          ]
        }
    }

    test_payload! {
        |message_update_sent|: {
          "object": "whatsapp_business_account",
          "entry": [
            {
              "id": "<WHATSAPP_BUSINESS_ACCOUNT_ID>",
              "changes": [
                {
                  "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                      "display_phone_number": "<BUSINESS_DISPLAY_PHONE_NUMBER>",
                      "phone_number_id": "<BUSINESS_PHONE_NUMBER_ID>"
                    },
                    "statuses": [
                      {
                        "id": "<WHATSAPP_MESSAGE_ID>",
                        "status": "sent",
                        "timestamp": 1289388883,
                        "recipient_id": "<WHATSAPP_USER_ID>",
                        "conversation": {
                          "id": "<CONVERSATION_ID>",
                          "origin": {
                            "type": "<CONVERSATION_CATEGORY>"
                          }
                        },
                        "pricing": {
                          "billable": true,
                          "pricing_model": "CBP",
                          "category": "<CONVERSATION_CATEGORY>"
                        }
                      }
                    ]
                  },
                  "field": "messages"
                }
              ]
            }
          ]
        }
    }
}
