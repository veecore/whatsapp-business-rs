//! WhatsApp App Management
//!
//! This module provides access to features that are scoped at the application level — not tied to a single business account.
//!
//! These include:
//! - Webhook configuration for event subscriptions
//! - Staged onboarding flows for connecting a business to your app
//! - App-level subscription management
//!
//! The core entry point is [`AppManager`], which you typically access through [`Client::app`].
//!
//! # Example – Configuring a Webhook
//! ```rust,no_run
//! use whatsapp_business_rs::{
//!     app::{SubscriptionField, WebhookConfig},
//!     client::Client,
//!     App,
//! };
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let app = App::new("123456");
//! let config = WebhookConfig {
//!     webhook_url: "https://example.com/webhook".to_string(),
//!     verify_token: "very_secret".into(),
//! };
//!
//! let client = Client::new("ACCESS_TOKEN").await?;
//! client
//!     .app(app)
//!     .configure_webhook(&config)
//!     .events(
//!         [
//!             SubscriptionField::Messages,
//!             SubscriptionField::MessageTemplateStatusUpdate,
//!         ]
//!         .into(),
//!     )
//!     .await?;
//! # Ok(()) }
//! ```
//!
//! # Example – Onboarding a Business
//! ```rust,no_run
//! use whatsapp_business_rs::{client::Client, App, Business};
//!
//! # async fn onboarding_example() -> Result<(), Box<dyn std::error::Error>> {
//! let app = App::new("123456");
//! let business = Business::new("WABA_ID", "PHONE_ID");
//!
//! let client = Client::new("ACCESS_TOKEN").await?;
//! let flow = client.app(app).start_onboarding(&business);
//!
//! // You can now:
//! // - .exchange_code(...)
//! // - .subscribe()
//! // - .register_phone_number(...)
//! // - .share_credit_line(...)
//! // - .finalize()
//! # Ok(()) }
//! ```
//!
//! > ⚠️ Onboarding flows are type-state driven. You must follow the correct sequence.
//!
//! See also:
//! - [`SubscriptionField`] — event types you can subscribe to
//! - [`WebhookConfig`] — how to configure verification
//! - [`OnboardingFlow`] — guided flow for connecting businesses

use crate::{rest::client::deserialize_str_opt, Endpoint};
use std::{borrow::Cow, fmt::Display, marker::PhantomData};

use serde::{Deserialize, Serialize};

use crate::{
    client::{AppSecret, Client, TokenAuth},
    derive,
    error::Error,
    flow, fut_net_op,
    rest::client::{
        AccessTokenRequest, RegisterPhoneRequest, ShareCreditLineRequest, ShareCreditLineResponse,
    },
    to_value, App, Auth, Business, Fields, FieldsTrait, SimpleOutput, Timestamp, ToValue,
};

/// Provides app-scoped access to WhatsApp API features
///
/// The `AppManager` allows you to manage things that are tied to a WhatsApp App, not an individual business account.
///
/// Typical use cases include:
/// - Registering webhook URLs
/// - Starting onboarding flows for business accounts
/// - Unsubscribing an app from a business
///
/// Obtain an `AppManager` by calling [`Client::app`] with an [`App`] struct.
///
/// # Example – Registering a Webhook
/// ```rust,no_run
/// use whatsapp_business_rs::{
///     app::{SubscriptionField, WebhookConfig},
///     App, Client,
/// };
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = WebhookConfig {
///     webhook_url: "https://example.com/webhook".to_string(),
///     verify_token: "very_secret_token".into(),
/// };
///
/// let client = Client::new("ACCESS_TOKEN").await?;
/// client
///     .app("123456") // your app ID
///     .configure_webhook(&config)
///     .events(
///         [
///             SubscriptionField::Messages,
///             SubscriptionField::MessageTemplateStatusUpdate,
///         ]
///         .into(),
///     )
///     .await?;
/// # Ok(()) }
/// ```
///
/// # Example – Onboarding a Business
/// ```rust,no_run
/// use whatsapp_business_rs::{client::Client, App, Business};
///
/// # async fn onboarding_example() -> Result<(), Box<dyn std::error::Error>> {
/// let business = Business::new("WABA_ID", "PHONE_ID");
///
/// let client = Client::new("ACCESS_TOKEN").await?;
/// let flow = client.app("123456").start_onboarding(&business);
///
/// // exchange token, subscribe, register, etc...
/// # Ok(()) }
/// ```
#[derive(Debug)]
pub struct AppManager<'i> {
    client: Client,
    app: Cow<'i, App>,
}

impl<'i> AppManager<'i> {
    /// Create a new message manager
    pub(crate) fn new(app: Cow<'i, App>, client: &Client) -> Self {
        Self {
            client: client.clone(),
            app,
        }
    }

    /// Updates (exchanges) any access token into a long-lived one.
    ///
    /// This method is typically used after a user completes an onboarding flow
    /// which often provides a short-lived access token. Meta's Graph API allows this
    /// short-lived token to be exchanged for a longer-lived token (approximately
    /// 60 days) by leveraging your Meta App's secret.
    ///
    /// # Parameters
    /// - `token`: The short-lived access token to be exchanged. This can be any
    ///   type that can be converted into a [`TokenAuth`] (e.g., `&str`, `String`).
    /// - `app_secret`: Your Meta App's secret key, required to authorize the
    ///   token exchange. This can be any type that can be converted
    ///   into an [`AppSecret`].
    ///
    /// # Returns
    /// A `Result` which, on success, contains a [`Token`] struct with the
    /// new long-lived access token and its expiration details. On failure,
    /// it returns an `Error`.
    ///
    /// # Important Notes
    /// - This method performs an external API call to Meta's Graph API.
    /// - It does **not** mutate the `Client` instance or replace any existing
    ///   authentication within the client. It is your responsibility to store
    ///   and reuse the returned long-lived [`Token`] appropriately for
    ///   subsequent API calls (e.g., by initializing a new `Client` (not recommended)
    ///   or using `.with_auth(...)`).
    /// - Requires a valid app secret that matches the `app_id` associated with
    ///   the `AppManager` to authorize the exchange.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use whatsapp_business_rs::Client;
    /// # use whatsapp_business_rs::error::Error;
    /// # async fn example_update_token(client: Client, app_id: &str, short_token: &str, app_secret: &str) -> Result<(), Error> {
    /// println!("Attempting to exchange short-lived token...");
    /// let long_lived_token_response = client
    ///     .app(app_id)
    ///     .update_token(short_token, app_secret)
    ///     .await?;
    ///
    /// println!("Successfully exchanged token!");
    /// println!("New long-lived token: {}", long_lived_token_response.access_token());
    /// println!("Expires in (seconds): {:?}", long_lived_token_response.expires_in());
    ///
    /// // You can now use `long_lived_token_response` for future API calls.
    /// // For example, to set it as the default auth for a new client:
    /// // let new_client = Client::new(long_lived_token_response).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// For more details, refer to the [Graph API Token Exchange Documentation](https://developers.facebook.com/docs/facebook-login/access-tokens/refreshing/)
    pub async fn update_token<'s, 't, T, A>(&self, token: T, app_secret: A) -> Result<Token, Error>
    where
        T: ToValue<'t, TokenAuth>,
        A: ToValue<'s, AppSecret>,
    {
        let request = AccessTokenRequest {
            client_id: &self.app.id,
            client_secret: &app_secret.to_value().0,
            grant_type: "fb_exchange_token",
            fb_exchange_token: &token.to_value(),
            ..Default::default()
        };

        // TODO: Convert timestamp to unix so we get has_expired?
        self.client.get_access_token(request).await
    }

    /// Retrieves detailed metadata and information about a given access token.
    ///
    /// This method is highly useful for introspection, debugging, and verifying
    /// the current status, permissions, and scopes associated with any access token.
    /// You can use it to check if a token is valid, when it expires, which app
    /// it belongs to, and what permissions it grants.
    ///
    /// # Parameters
    /// - `token`: The access token (short-lived or long-lived) for which you
    ///   want to retrieve debug information. This can be any type that
    ///   can be converted into a [`TokenAuth`] (e.g., `&str`, `String`).
    ///
    /// # Returns
    /// A `Result` which, on success, contains a [`TokenDebug`] struct with
    /// various details about the token. On failure, it returns an `Error`.
    ///
    /// # API Endpoint
    /// This method makes a GET request to the `/debug_token` Graph API endpoint.
    ///
    /// # Current Limitations
    /// At present, this implementation does **not** support field filtering for
    /// the debug token response. The full response structure returned by Meta's
    /// API will be deserialized into [`TokenDebug`].
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use whatsapp_business_rs::Client;
    /// use whatsapp_business_rs::error::Error;
    ///
    /// # async fn example_debug_token(client: Client, app_id: &str) -> Result<(), Error> {
    /// let user_token = "Ez......";
    ///
    /// println!("Debugging token...");
    /// let token_info = client
    ///     .app(app_id)
    ///     .debug_token(user_token)
    ///     .await?;
    ///
    /// println!("Token Debug Info:");
    /// println!("  App ID: {}", token_info.app_id);
    /// println!("  User ID: {:?}", token_info.user_id); // Optional, might not be present for app tokens
    /// println!("  Is Valid: {}", token_info.is_valid);
    /// println!("  Expires At: {:?}", token_info.expires_at);
    /// println!("  Scopes: {:?}", token_info.scopes);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn debug_token<'t, T>(&self, token: T) -> Result<TokenDebug, Error>
    where
        T: ToValue<'t, TokenAuth>,
    {
        let token: &str = &token.to_value();

        let request = self
            .client
            .get(self.client.endpoint().join("debug_token"))
            .query(&[("input_token", token)]);

        fut_net_op(request).await
    }

    /// Retrieves a long-lived access token for this Meta App.
    ///
    /// A long-lived token is crucial for applications that need to
    /// maintain continuous access without frequent re-authentication.
    ///
    /// # Parameters
    /// - `app_secret`: The secret key associated with your Meta App. This can be
    ///   anything that can be converted into an [`AppSecret`] type.
    ///
    /// # Returns
    /// A `Result` which is:
    /// - `Ok(Token)`: On success, a long-lived access token.
    /// - `Err(Error)`: If the request fails, for example, due to an invalid `app_secret`,
    ///   network issues, or Meta API errors.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::client::AppSecret;
    ///
    /// # async fn example(app_manager: whatsapp_business_rs::app::AppManager<'_>)
    /// # -> Result<(), Box<dyn std::error::Error>> {
    /// let app_secret = AppSecret("YOUR_META_APP_SECRET".to_string());
    ///
    /// match app_manager.get_access_token(app_secret).await {
    ///     Ok(auth_token) => {
    ///         println!("Successfully obtained long-lived access token!");
    ///         // You can now use `auth_token` for future API calls
    ///     },
    ///     Err(e) => eprintln!("Failed to get access token: {}", e),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`AppSecret`]: crate::AppSecret
    /// [`Error`]: crate::error::Error
    pub async fn get_access_token<'s>(
        &self,
        app_secret: impl ToValue<'s, AppSecret>,
    ) -> Result<Token, Error> {
        let request = AccessTokenRequest {
            client_id: &self.app.id,
            client_secret: &app_secret.to_value().0,
            grant_type: "client_credentials",
            ..Default::default()
        };

        self.client.get_access_token(request).await
    }

    /// This method creates a `ConfigureWebhook` builder that, when awaited,
    /// sends a `POST /{app-id}/subscriptions` request to Meta’s Graph API.
    /// It allows you to fluently select which [`SubscriptionField`] events the webhook should listen for.
    ///
    /// This method does **not** start a server or begin listening for events. It returns a builder
    /// that you can either `await` directly to register the webhook, or pass to
    /// [`Serve::configure_webhook`] to perform registration as part of the server startup.
    ///
    /// # Parameters
    /// - `config`: The webhook configuration, including the URL, and verify token.
    ///
    /// # Returns
    /// A `ConfigureWebhook` builder.
    ///
    /// # Examples
    ///
    /// ## Registering a webhook immediately:
    /// ```rust
    /// use whatsapp_business_rs::{
    ///     app::{SubscriptionField, WebhookConfig},
    ///     Fields,
    /// };
    ///
    /// # async fn example(app_manager: whatsapp_business_rs::app::AppManager<'_>)
    /// # -> Result<(), Box<dyn std::error::Error>> {
    /// let config = WebhookConfig {
    ///     verify_token: "choosen_secret".into(),
    ///     webhook_url: "https://example.com/whatsapp_webhook".into(),
    /// };
    ///
    /// app_manager
    ///     .configure_webhook(&config)
    ///     .events(Fields::all())
    ///     .await?; // Await to perform the registration
    /// # Ok(())}
    /// ```
    ///
    /// ## Preparing a webhook registration for server startup:
    /// (See [`Serve::configure_webhook`] for a full example)
    /// ```rust
    /// use whatsapp_business_rs::{Fields, app::{SubscriptionField, WebhookConfig}};
    ///
    /// # fn example_prep(app_manager: whatsapp_business_rs::app::AppManager<'_>, config: &WebhookConfig) {
    /// let pending_registration = app_manager
    ///      .configure_webhook(config)
    ///      .events(Fields::all());
    ///
    /// // This `pending_registration` can now be passed to `Serve::configure_webhook`.
    /// // DO NOT await it here if you intend to pass it to `Serve::configure_webhook`.
    /// # }
    /// ```
    ///
    /// [`Fields`]: crate::Fields
    /// [`SubscriptionField`]: crate::app::SubscriptionField
    /// [`Serve::configure_webhook`]: crate::server::Serve::configure_webhook
    pub fn configure_webhook<'c, C>(&self, config: C) -> ConfigureWebhook<'c>
    where
        C: ToValue<'c, WebhookConfig>,
    {
        let request = self
            .client
            .post(self.endpoint("subscriptions"))
            .query(&config.to_value().to_request());

        ConfigureWebhook {
            request,
            _marker: PhantomData,
        }
    }

    /// Starts a new onboarding flow for a WhatsApp Business.
    ///
    /// # Arguments
    /// - `business`: The target `Business` instance to onboard.
    ///
    /// # Returns
    /// An [`OnboardingFlow`] that enforces the sequence of onboarding steps.
    pub fn start_onboarding<'a, 'b, B>(&'a self, business: B) -> OnboardingFlow<'a, 'b>
    where
        B: ToValue<'b, Business>,
    {
        OnboardingFlow::new(self, business.to_value())
    }

    /// Removes the app's subscription from the specified business.
    ///
    /// # Arguments
    /// - `business`: The target business whose app subscription should be revoked.
    ///
    /// # Returns
    /// An empty `Result` if successful, otherwise an error.
    pub async fn unsubscribe(&self, business: &Business) -> Result<(), Error> {
        let request = self.client.delete(
            self.client
                .a_node(&business.account_id)
                .join("subscribed_apps"),
        );

        fut_net_op(request).await
    }

    Endpoint! {app.id}
}

/// Represents an OAuth access token issued by the Graph API.
///
/// > Note: While this type lives under `crate::app`, the token itself may belong to
/// > a variety of identities — including app users, system users, page subscribers,
/// > or even the app itself.
///
/// The `expires_in` field indicates how long (in seconds) the token will remain valid
/// from the time of issuance. This is **not guaranteed** to be present.
///
/// If you need precise expiration data (e.g., `expires_at`, `data_access_expires_at`),
/// call `debug_token(...)` to inspect a token more thoroughly.
#[derive(Deserialize, Debug)]
pub struct Token {
    access_token: String,
    token_type: TokenType,

    /// Time-to-live in seconds (relative to time of issuance).
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_str_opt::<i64, __D>")]
    expires_in: Option<i64>,
}

impl Token {
    /// Returns the raw access token string.
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Returns the token type (usually [`TokenType::Bearer`]).
    pub fn token_type(&self) -> TokenType {
        self.token_type
    }

    /// Returns the number of seconds this token is valid for, if available.
    pub fn expires_in(&self) -> Option<i64> {
        self.expires_in
    }

    /// Returns whether the token is considered long-lived (~60 days).
    ///
    /// This is based on a simple heuristic threshold.
    pub fn is_long_lived(&self) -> bool {
        // A long-lived token is typically around 60 days.
        // 60 days * 24 hours/day * 60 minutes/hour * 60 seconds/minute = 5,184,000 seconds
        // We'll use a threshold slightly less than 60 days to be safe, e.g., 58 days.
        // 58 days * 24 * 60 * 60 = 5,011,200 seconds
        const LONG_LIVED_THRESHOLD_SECONDS: i64 = 5_011_200;

        self.expires_in
            .map(|s| s > LONG_LIVED_THRESHOLD_SECONDS)
            .unwrap_or(false)
    }
}

/// Represents the type of access token used.
///
/// Meta typically uses "Bearer" tokens, but this enum allows future extension
/// if other token schemes are introduced or detected.
///
/// This is mostly informational — token behavior is dictated by access scopes and origin,
/// not this enum alone.
#[derive(Deserialize, PartialEq, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TokenType {
    /// Standard Bearer token   
    Bearer,
}

impl Display for TokenType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenType::Bearer => write!(f, "Bearer"),
        }
    }
}

impl Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.access_token.fmt(f)
    }
}

impl ToValue<'_, TokenAuth> for Token {
    fn to_value(self) -> Cow<'static, TokenAuth> {
        self.access_token.to_value()
    }
}

impl<'a> ToValue<'a, TokenAuth> for &'a Token {
    fn to_value(self) -> Cow<'a, TokenAuth> {
        (&self.access_token).to_value()
    }
}

impl ToValue<'_, Auth> for Token {
    fn to_value(self) -> Cow<'static, Auth> {
        Cow::Owned(Auth::token(self))
    }
}

impl ToValue<'_, Auth> for &Token {
    fn to_value(self) -> Cow<'static, Auth> {
        Cow::Owned(Auth::token(self))
    }
}

/// Debug token information
#[derive(Deserialize, Clone, Debug)]
#[non_exhaustive]
pub struct TokenDebug {
    pub app_id: String,
    pub application: String,
    pub data_access_expires_at: Timestamp,
    pub expires_at: Timestamp,
    #[serde(default)]
    pub granular_scopes: Vec<GranularScope>,
    pub is_valid: bool,
    // permissions
    #[serde(default)]
    pub scopes: Vec<String>,
    pub r#type: String,
    #[serde(default)]
    pub user_id: Option<String>,
}

/// Granular scope information
#[derive(Deserialize, Clone, Debug)]
#[non_exhaustive]
pub struct GranularScope {
    pub scope: String,
    // account_ids of businesses
    #[serde(default)]
    pub target_ids: Vec<String>,
}

/// Represents the essential configuration for a webhook endpoint.
///
/// This struct bundles the `webhook_url` where your server will receive notifications
/// and the `verify_token` used by WhatsApp to authenticate your endpoint during setup
/// and verify incoming requests.
#[derive(Clone, Debug)]
pub struct WebhookConfig {
    pub verify_token: String,
    pub webhook_url: String,
}

to_value! {
    WebhookConfig
}

impl<V, W> ToValue<'_, WebhookConfig> for (V, W)
where
    V: Into<String> + Send + Sync,
    W: Into<String> + Send + Sync,
{
    fn to_value(self) -> Cow<'static, WebhookConfig> {
        Cow::Owned(WebhookConfig {
            verify_token: self.0.into(),
            webhook_url: self.1.into(),
        })
    }
}

SimpleOutput! {
    ConfigureWebhook<'a> => ()
}

impl<'a> ConfigureWebhook<'a> {
    /// Specifies the webhook events to subscribe to.
    ///
    /// This method allows you to select which types of events (e.g., `messages`, `account_updates`)
    /// your webhook should receive.
    ///
    /// If this method is not called, the webhook might subscribe to a default
    /// set of events (if specified by Meta's API) or no events, meaning you
    /// would not receive notifications. It is highly recommended to explicitly
    /// define the events you need.
    ///
    /// # Parameters
    /// - `events`: A [`Fields`] struct containing the desired [`SubscriptionField`]s
    ///   representing the events you wish to receive.
    ///
    /// # Returns
    /// The updated `ConfigureWebhook` builder instance, allowing for further configuration
    /// or finalization of the webhook setup.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::Fields;
    /// use whatsapp_business_rs::app::{SubscriptionField, WebhookConfig};
    ///
    /// # async fn example(manager: whatsapp_business_rs::app::AppManager<'_>)
    /// # -> Result<(), Box<dyn std::error::Error>> {
    /// manager
    ///     .configure_webhook(WebhookConfig {
    ///         verify_token: "YOUR_SECRET_VERIFY_TOKEN".into(),
    ///         webhook_url: "https:///your.domain/webhook-endpoint".into(),
    ///     })
    ///     .events(
    ///         Fields::new()
    ///             .with(SubscriptionField::Messages)
    ///             .with(SubscriptionField::AccountUpdate),
    ///     )
    ///     .await?;
    ///
    /// println!("Webhook configured successfully!");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`Fields`]: crate::Fields
    /// [`SubscriptionField`]: crate::webhook::SubscriptionField
    pub fn events(mut self, events: Fields<SubscriptionField>) -> Self {
        self.request = events.into_request(self.request, []);
        self
    }
}

impl<'a, 'b> OnboardingFlow<'a, 'b> {
    pub(crate) fn new(manager: &'a AppManager, business: Cow<'b, Business>) -> Self {
        Self {
            manager,
            business,
            access_token: (),
            subscribe: (),
            register: (),
            share_credit_line: (),
        }
    }
}

flow! {
    /// A staged onboarding flow for WhatsApp Business accounts.
    ///
    /// This type enforces correct sequencing of onboarding steps at compile-time using type state.
    ///
    /// ⚠️ This flow is not considered stable — steps or requirements may change.
    #[must_use = "OnboardingFlow's steps need to be completed"]
    pub struct OnboardingFlow<'a, 'b, S1, S2, S3, S4> {
        access_token: S1,
        subscribe: S2,
        register: S3,
        share_credit_line: S4,

        manager: &'a AppManager<'a>,
        business: Cow<'b, Business>,
    }

    /// Step 1: Exchanges an authorization code for an access token.
    ///
    /// This is typically the first step after a user completes Meta's Embedded Signup flow.
    ///
    /// # Arguments
    /// - `code`: The authorization code returned by Meta (typically from the embedded flow).
    /// - `app_secret`: Your app's client secret.
    ///
    /// # Returns
    /// The next step in the flow if successful, or an error.
    pub async fn get_access_token<'s>(
        self,
        code: &str,
        app_secret: impl ToValue<'s, AppSecret>,
    ) -> Token {
        let request = AccessTokenRequest {
            client_id: &self.manager.app.id,
            client_secret: &app_secret.to_value().0,
            code,
            ..Default::default()
        };

        let token = self.manager.client.get_access_token(request).await?;

        (token, self)
    }

    /// Step 2: Subscribes the business account to your WhatsApp application.
    ///
    /// This allows you to receive webhooks and send messages.
    ///
    /// # Returns
    /// The next step in the onboarding flow if successful.
    pub async fn subscribe(
        self,
    ) -> PhantomData<()> {
        // FIXME: This can't be
        let client = &self.manager.client;
        let account_id = self.business.account_id.as_str();

        let url = client.a_node(account_id).join("subscribed_apps");

        let request = client
            .post(url)
            .bearer_auth(&self.access_token);

        (fut_net_op(request).await?, self)
    }

    /// Step 3: Registers the phone number for WhatsApp use.
    ///
    /// This usually involves sending a PIN that was received from Meta to finalize verification.
    ///
    /// # Arguments
    /// - `pin`: The verification PIN.
    ///
    /// # Returns
    /// The next step in the onboarding flow.
    pub async fn register_phone_number(
        self,
        pin: &str,
    ) -> PhantomData<()> {
        let client = &self.manager.client;
        let request = RegisterPhoneRequest::from_pin(pin);

        let url = client.a_node(&self.business.identity.phone_id).join("register");

        let request = client
            .post(url)
            .bearer_auth(&self.access_token)
            .json(&request);

        (fut_net_op(request).await?, self)
    }

    /// Step 4: Optionally shares a credit line with the business account.
    ///
    /// This enables the business to send WhatsApp messages using your WhatsApp Business credit.
    ///
    /// # Arguments
    /// - `credit_line_id`: The ID of your credit line.
    /// - `currency`: Currency in ISO 4217 format (e.g., `"GBP"`, `"USD"`).
    ///
    /// # Returns
    /// The final state in the onboarding flow.
    #![optional] pub async fn share_credit_line(
        self,
        credit_line_id: &str,
        currency: &str,
    ) -> String
    {
        let client = &self.manager.client;
        let waba_id = &self.business.account_id;

        let payload = ShareCreditLineRequest {
            waba_id,
            waba_currency: currency,
        };

        let url = client.a_node(credit_line_id).join("whatsapp_credit_sharing_and_attach");

        let request = client
            .post(url)
            .query(&payload);

        let res: ShareCreditLineResponse = fut_net_op(request).await?;

        (res.allocation_config_id, self)
    }
}

impl<T> OnboardingFlow<'_, '_, Token, PhantomData<()>, PhantomData<()>, T> {
    /// Finalizes the onboarding and returns access credentials and allocation info.
    ///
    /// This can be stored or used to interact with the business account.
    ///
    /// # Returns
    /// A [`BusinessOnboardingResult`] with the access token and credit allocation ID.
    pub fn finalize(self) -> BusinessOnboardingResult<T> {
        BusinessOnboardingResult {
            access_token: self.access_token,
            allocation_config_id: self.share_credit_line,
        }
    }
}

/// Final result of the onboarding process.
///
/// Contains credentials and allocation info required to interact with the onboarded business.
#[derive(Debug)]
#[non_exhaustive]
pub struct BusinessOnboardingResult<T> {
    /// The WhatsApp access token returned by Meta.
    pub access_token: Token,
    /// The credit line allocation ID (if shared).    
    pub allocation_config_id: T,
}

derive! {
    /// Subscription event types available for WhatsApp Business webhooks.
    ///
    /// This enum lists all the possible event types (also known as "fields") that
    /// can be subscribed to when configuring a webhook with [`AppManager::configure_webhook`] or
    /// as part of the server setup with [`Serve::configure_webhook`].
    ///
    /// These fields determine what kinds of notifications your webhook will receive
    /// from Meta’s servers.
    ///
    /// Use it with the `.with(...)`, `.extend(...)`, or `.all()` methods on [`Fields`].
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::{Fields, app::SubscriptionField};
    ///
    /// let events = Fields::new()
    ///      .with(SubscriptionField::Messages)
    ///      .with(SubscriptionField::AccountUpdate);
    /// ```
    ///
    /// [`Fields`]: crate::Fields
    /// [`AppManager::configure_webhook`]: crate::app::AppManager::configure_webhook
    /// [`Serve::configure_webhook`]: crate::server::Serve::configure_webhook
    #[derive(#FieldsTrait, Serialize, PartialEq, Eq, Hash, Clone, Copy, Debug)]
    #[serde(rename_all = "snake_case")]
    pub enum SubscriptionField {
        /// Notifies you of decisions related to Official Business Account status
        /// or rejections of messaging limit increase requests.
        AccountAlerts,

        /// Triggered when your WhatsApp Business Account (WABA) is reviewed by Meta.
        AccountReviewUpdate,

        /// Triggered on any change to your WABA, including phone number updates,
        /// policy violations, bans, and more.
        AccountUpdate,

        /// Notifies you of changes to your business's capabilities, such as:
        /// - Messaging limits
        /// - Max allowed phone numbers
        BusinessCapabilityUpdate,

        /// Triggered when a message template's content is updated,
        /// e.g., title/body edits or button additions.
        MessageTemplateComponentsUpdate,

        /// Triggered when a message template's quality rating changes.
        MessageTemplateQualityUpdate,

        /// Triggered when a message template is approved, rejected, or disabled.
        MessageTemplateStatusUpdate,

        /// Triggered when:
        /// - A customer sends your business a message
        /// - You send/deliver a message to a customer
        /// - A customer reads your message
        Messages,

        /// Triggered when the verified name for a phone number is approved or rejected.
        PhoneNumberNameUpdate,

        /// Triggered when a phone number’s quality rating changes.
        PhoneNumberQualityUpdate,

        /// Triggered when:
        /// - Two-step verification is updated, disabled, or disabled by request
        Security,

        /// Triggered when the category of a message template is changed.
        TemplateCategoryUpdate,
    }
}

impl<'a> ToValue<'a, App> for AppManager<'a> {
    #[inline]
    fn to_value(self) -> Cow<'a, App> {
        self.app
    }
}

impl<'a> ToValue<'a, App> for &'a AppManager<'a> {
    #[inline]
    fn to_value(self) -> Cow<'a, App> {
        Cow::Borrowed(self.app.as_ref())
    }
}
