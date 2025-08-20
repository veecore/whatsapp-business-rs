//! WhatsApp Business API client implementation
//!
//! This module provides the main client used to interact with Meta's WhatsApp Business Graph API.
//! It handles authentication, and scoped access to specialized managers
//! like messages, catalogs, apps, and wabas.
//!
//! # Example – Creating a Client
//!
//! ```rust,no_run
//! use std::time::Duration;
//! use whatsapp_business_rs::client::Client;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::builder()
//!     .timeout(Duration::from_secs(15))
//!     .api_version("v19.0")
//!     .connect("YOUR_ACCESS_TOKEN")
//!     .await?;
//! # Ok(()) }
//! ```
//!
//! # Example – Sending a Message
//!
//! ```rust,no_run
//! use whatsapp_business_rs::client::Client;
//! use whatsapp_business_rs::{message::Draft, IdentityRef};
//!
//! # async fn run(client: Client) -> Result<(), Box<dyn std::error::Error>> {
//! let business = IdentityRef::business("1234567890");
//! let user = IdentityRef::user("9876543210");
//!
//! client
//!     .message(business)
//!     .send(user, Draft::text("Hello from Rust!"))
//!     .await?;
//! # Ok(()) }
//! ```

use super::error::Error;
use crate::{
    add_auth_to_request,
    app::{AppManager, Token},
    catalog::CatalogManager,
    message::{IntoDraft, MediaSource, MediaType, MessageCreate, MessageRef},
    rest::{
        client::{AccessTokenRequest, MessageRequestOutput, SendMessageResponse},
        fut_net_op, IntoMessageRequestOutput,
    },
    to_value,
    waba::WabaManager,
    App, CatalogRef, Endpoint, IdentityRef, IntoFuture, NullableUnit, SimpleOutput, ToValue, Waba,
};

use reqwest::{
    header::{HeaderMap, HeaderValue},
    Client as HttpClient, ClientBuilder as HttpClientBuilder, IntoUrl, RequestBuilder,
};
use std::{
    borrow::Cow, fmt::Display, marker::PhantomData, mem::transmute, ops::Deref, sync::Arc,
    time::Duration,
};

#[cfg(feature = "batch")]
use crate::{batch::Batch, reference, requests_batch_include};

/// Default API version for WhatsApp Business API
const DEFAULT_API_VERSION: &str = "22.0";
/// Default user agent for the client
const USER_AGENT: &str = "whatsapp-business-rs/0.1 (Rust)";

/// The primary entry point for interacting with the **WhatsApp Business API**.
///
/// This `Client` provides a strongly-typed and user-friendly wrapper around
/// Meta's Graph API for WhatsApp. It simplifies making API calls by managing
/// bearer token injection automatically.
///
/// You can create a new `Client` instance using either [`Client::new`] for
/// a quick setup with an access token, or [`Client::builder`] for more
/// advanced configuration.
///
/// # Key Capabilities (Sub-Managers)
///
/// Access specific API functionalities through dedicated sub-managers:
/// - `.message(identity_ref)`: For sending, and managing messages.
/// - `.catalog(catalog_ref)`: To manage your product catalogs, including adding or updating items.
/// - `.app(app)`: For configuring webhooks, managing app settings, and handling onboarding flows.
/// - `.waba(business)`: To manage your WhatsApp Business Accounts (WBAs)
///
/// # Authentication Design (App-Scoped Client)
///
/// This client is designed as an **app-scoped client**. While you can
/// initialize it with a default [`Auth`] token (ideal for simple, single-tenant
/// applications), most operations also support **per-request authentication**
/// using the `.with_auth(...)` method. This flexibility is crucial for
/// multi-tenant applications or scenarios where different operations require
/// different access tokens (e.g., user-specific tokens).
///
/// # Example
///
/// Initialize the client with your access token:
/// ```rust,no_run
/// use whatsapp_business_rs::Client;
///
/// # async fn example() {
/// let client = Client::new("YOUR_WHATSAPP_ACCESS_TOKEN").await.unwrap();
/// // Now you can use the client to interact with the API, e.g., to send messages:
/// // client.message("phone_number_id").send("recipient_number", "Hello from Rust!").await;
/// # }
/// ```
///
/// # Important Authentication Note ⚠️
///
/// `Client` does **not** enforce authentication capabilities at compile time.
/// It's your responsibility to ensure the correct authentication token (e.g.,
/// a user-level token for user-specific operations, or an app-level token for
/// app settings) is used for each API call to prevent runtime authorization errors.
#[derive(Clone, Debug)]
pub struct Client {
    inner: Arc<InnerClient>,
}

/// Represents the authentication credentials used to interact with the
/// **WhatsApp Business API** via Meta's Graph API.
///
/// This enum encapsulates the different types of tokens supported:
/// direct access tokens (`Token`) or app ID/secret pairs (`Secret`).
///
/// # Design Considerations
///
/// The `Client` does **not** allow updating its internal authentication once
/// it's been initialized. This design choice simplifies the API and accounts
/// for the typical 60-day expiration of long-lived WhatsApp tokens.
///
/// For long-term or dynamic authentication needs, consider these strategies:
///
/// * **Initialize with `Auth::Secret`**: For app-level endpoints,
///   initializing the client with your `app_id` and `app_secret` is
///   recommended.
/// * **Per-Operation Authentication**: For user- or system-level operations,
///   use the `.with_auth(...)` method available on most operations. This
///   allows you to inject the appropriate token for a specific request.
/// * **Manual Token Refresh**: If an access token expires, you can manually
///   refresh or upgrade it. For instance, to extend an existing token,
///   you might use:
///```rust
/// // Assuming 'client' is your initialized Client and 'app_id' is your Meta App ID
/// // 'old_token' is the expired or soon-to-expire token, 'app_secret' is your app's secret
/// // This call returns a new token (usually long-lived) and its expiration details.
/// # use whatsapp_business_rs::Client;
/// # async fn example_(client: Client) -> Result<(), Box<dyn std::error::Error>> {
/// let new_token_response = client
///     .app("your_app_id")
///     .update_token("old_token", "your_app_secret")
///     .await
///     .unwrap();
///
/// let refreshed_token = new_token_response.access_token();
/// let expires_in = new_token_response.expires_in();
/// // Store or use 'refreshed_token' for future API calls
/// # Ok(())}
///```
///
/// # Authentication Scopes and Best Practices
///
/// Think of the `Client` as primarily an **app-scoped client**. While it can
/// be used for user-level interactions, be mindful that:
///
/// -   There are no compile-time checks to prevent using a token outside
///     its intended scope (e.g., using an app token for a user-specific
///     message).
/// -   Using an incorrect token for an operation will result in runtime
///     authorization errors from Meta's API.
///
/// Meta's authentication mechanisms (like `app_secret` behavior) can change.
/// We are actively monitoring these changes. Contributions and insights are
/// highly valued!
#[derive(PartialEq, Clone, Debug)]
pub enum Auth {
    /// Represents a temporary or long-lived **access token**.
    ///
    /// This variant holds the raw access token string. It's suitable for
    /// direct authentication when you already have a valid token.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::Auth;
    ///
    /// let auth_token = Auth::token("YOUR_ACCESS_TOKEN_STRING");
    /// ```
    Token(TokenAuth),

    /// Represents authentication using your **Meta App ID** and **App Secret**.
    ///
    /// The `app_id` and `app_secret` are combined into a `{app_id}|{app_secret}` string
    /// for direct use with app-level API endpoints.
    ///
    /// # Fields
    /// - `app_id`: The unique identifier for your Meta App.
    /// - `app_secret`: The secret key associated with your Meta App.
    ///
    /// # Important Note
    /// This variant is primarily valid for **app-level operations** (e.g., configuring
    /// webhooks, managing app settings). For operations that require user-specific or
    /// system user tokens (like sending messages on behalf of a specific WABA), you
    /// should typically use the `.with_auth(...)` method on the relevant operation
    /// to inject the appropriate `Auth::Token` at the operation level.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::Auth;
    ///
    /// let auth_secret = Auth::secret(("YOUR_APP_ID", "YOUR_APP_SECRET"));
    /// ```
    Secret {
        app_id: String,
        app_secret: AppSecret,
    },

    /// Represents a pre-parsed and optimized authentication token.
    ///
    /// This variant is backed by the `bytes::Bytes` type, making it highly efficient for cloning and sharing.
    /// It is ideal for situations where authentication credentials are used frequently in-memory, as it helps
    /// to reduce memory usage and allocation overhead.
    ///
    /// #### **Creation**
    ///
    /// You cannot create a `Parsed` variant directly. Instead, you must first create an `Auth::Token` or `Auth::Secret`
    /// variant and then convert it to `Auth::Parsed`.
    ///
    /// #### **Example**
    ///
    /// ```rust
    /// use whatsapp_business_rs::Auth;
    ///
    /// // Example 1: Creating a Parsed token from a direct token
    /// let parsed_auth_token = Auth::token("YOUR_ACCESS_TOKEN_STRING").parsed();
    ///
    /// // Example 2: Creating a Parsed token from an App ID and Secret
    /// let app_id = "YOUR_APP_ID";
    /// let app_secret = "YOUR_APP_SECRET";
    /// let parsed_auth_secret = Auth::secret((app_id, app_secret)).parsed();
    ///
    /// // You can now use parsed_auth_token or parsed_auth_secret with minimal memory overhead
    /// ```
    Parsed(ParsedAuth),
}

impl Auth {
    /// Creates an `Auth::Token` variant from a value that can be converted
    /// into a `TokenAuth`.
    ///
    /// This is a convenient constructor for creating authentication using
    /// a direct access token string.
    ///
    /// # Arguments
    /// - `token`: A value (e.g., `&str` or `String`) that can be converted
    ///   into a `TokenAuth` instance.
    ///
    /// # Example
    ///
    /// ```rust
    /// use whatsapp_business_rs::Auth;
    ///
    /// let auth = Auth::token("EAA...your_long_lived_token...AAA");
    /// ```
    pub fn token<'a>(token: impl ToValue<'a, TokenAuth>) -> Self {
        Self::Token(token.to_value().into_owned())
    }

    /// Creates an `Auth::Secret` variant from an app ID and app secret pair.
    ///
    /// This is a convenient constructor for creating authentication using
    /// your Meta App's credentials.
    ///
    /// # Arguments
    /// - `secret`: A tuple `(app_id, app_secret)` where `app_id` can be
    ///   converted into a `String` and `app_secret` can be
    ///   converted into an `AppSecret`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use whatsapp_business_rs::Auth;
    ///
    /// let auth = Auth::secret(("123456789012345", "your_meta_app_secret_string"));
    /// ```
    pub fn secret<'s, I, S>(secret: (I, S)) -> Self
    where
        I: Into<String>,
        S: ToValue<'s, AppSecret>,
    {
        Self::Secret {
            app_id: secret.0.into(),
            app_secret: secret.1.to_value().into_owned(),
        }
    }

    /// This method converts the current Auth variant (either Token or Secret) into
    /// an optimized `Auth::Parsed` variant.
    ///
    /// This conversion process prepares the authentication data for efficient use,
    /// particularly for in-memory operations. It's especially useful for reducing
    /// memory overhead and allocation costs when the authentication data needs to be
    /// frequently cloned or shared.
    ///
    /// If the conversion fails, it returns an AuthParsedError, which provides details
    /// about the underlying reason for the failure.
    pub fn parsed(self) -> Result<Self, AuthParsedError> {
        // Our try_into adds Bearer
        let parsed = self
            .try_into()
            .map_err(|err| AuthParsedError { inner: err })?;

        Ok(Self::Parsed(ParsedAuth { header: parsed }))
    }
}

/// A wrapper struct for a **WhatsApp Business API access token** string.
///
/// This struct holds the raw string value of an access token, providing
/// type safety and clarity when passing tokens around within the crate.
/// It is typically used as the payload for the `Auth::Token` enum variant.
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::client::TokenAuth;
///
/// let access_token = TokenAuth("EAAG...your_token_string...FGA".to_string());
#[derive(PartialEq, Clone, Debug)]
pub struct TokenAuth(pub String);

/// A wrapper struct for the **Meta App Secret** string.
///
/// This struct encapsulates the secret key associated with your Meta App.
/// It is primarily used within the `Auth::Secret` enum variant to provide
/// credentials for app-level authentication.
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::client::AppSecret;
///
/// let app_secret = AppSecret("YOUR_APP_SECRET_STRING_HERE".to_string());
#[derive(PartialEq, Clone, Debug)]
pub struct AppSecret(pub String);

/// A parsed representation of the authentication token for an API client.
///
/// This struct holds the authentication token data in a format suitable for
/// use in HTTP request headers.
#[derive(PartialEq, Clone, Debug)]
pub struct ParsedAuth {
    /// The authentication token data, formatted for an HTTP header.
    header: reqwest::header::HeaderValue,
}

impl Client {
    /// Creates a new client with default configuration.
    ///
    /// This is the simplest way to get started — uses default timeouts and latest API version.
    ///
    /// # Arguments
    /// * `auth` - The access token used to authenticate with the WhatsApp API
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::Client;
    ///
    /// # async fn example() {
    /// let client = Client::new("your_access_token").await.unwrap();
    /// # }
    /// ```
    pub async fn new<'a, A: ToValue<'a, Auth>>(auth: A) -> Result<Self, Error> {
        Self::builder().connect(auth).await
    }

    /// Starts building a new WhatsApp client with custom settings.
    ///
    /// Allows setting timeouts, API version, etc.
    ///
    /// # Example
    /// ```rust,no_run
    /// use std::time::Duration;
    /// use whatsapp_business_rs::Client;
    ///
    /// # async fn example() {
    /// let client = Client::builder()
    ///     .timeout(Duration::from_secs(10))
    ///     .api_version("v19.0")
    ///     .connect("your_token")
    ///     .await
    ///     .unwrap();
    /// # }
    /// ```
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    /// Returns a message manager for sending messages as a given sender.
    ///
    /// This is the main entry point for interacting with the `/messages` endpoint.
    ///
    /// # Arguments
    /// * `from` - The sender identity (typically your business's phone number)
    ///
    /// # Example
    /// ```rust,no_run
    /// # use whatsapp_business_rs::IdentityRef;
    /// # async fn example(client: whatsapp_business_rs::Client) {
    /// let sender = IdentityRef::business("1234567890");
    /// let manager = client.message(sender);
    /// # }
    /// ```
    pub fn message<'i, I>(&self, from: I) -> MessageManager<'i>
    where
        I: ToValue<'i, IdentityRef>,
    {
        MessageManager::new(from.to_value(), self)
    }

    /// Returns a catalog manager scoped to the given business.
    ///
    /// Useful for managing product catalogs owned by a business account.
    ///
    /// # Arguments
    /// * `c` - A reference to the catalog.
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn example(client: whatsapp_business_rs::Client) {
    /// let catalog = client.catalog("1234567890");
    /// # }
    /// ```
    pub fn catalog<'c, C>(&self, c: C) -> CatalogManager<'c>
    where
        C: ToValue<'c, CatalogRef>,
    {
        CatalogManager::new(c.to_value(), self)
    }

    /// Returns a WABA manager for performing account-level operations.
    ///
    /// Includes access to phone numbers, subscribed apps, and number registration flows.
    ///
    /// # Arguments
    /// * `w` - Business ID or [`Waba`]
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn example(client: whatsapp_business_rs::Client) {
    /// let waba = client.waba("1234567890");
    /// # }
    /// ```
    pub fn waba<'w, W>(&self, w: W) -> WabaManager<'w>
    where
        W: ToValue<'w, Waba>,
    {
        WabaManager::new(w.to_value(), self)
    }

    /// Returns an App manager for checking app integrations with a business.
    ///
    /// Lets you fetch info about which apps are connected to the WhatsApp account.
    ///
    /// # Arguments
    /// * `a` - An [`App`] or app ID
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn example(client: whatsapp_business_rs::Client) {
    /// let app = client.app("987654321");
    /// # }
    /// ```
    pub fn app<'p, A>(&self, a: A) -> AppManager<'p>
    where
        A: ToValue<'p, App>,
    {
        AppManager::new(a.to_value(), self)
    }

    /// Create a new [`Batch`] for grouping multiple requests into a single network call.
    ///
    /// Batching reduces network overhead and enables advanced workflows,
    /// like uploading media once and reusing it across multiple messages.
    /// This is especially useful for **bulk messaging scenarios**, where you want to send
    /// several messages in one go while staying within Meta's platform policies.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use whatsapp_business_rs::Client;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = Client::new("YOUR-API-KEY").await?;
    ///
    /// // ⚠️ Always ensure your bulk messaging complies with Meta’s rules.
    /// // Sending unsolicited or spammy messages can lead to account restrictions.
    /// let recipients = ["+1111111111", "+2222222222", "+3333333333"];
    ///
    /// let output = client
    ///     .batch()
    ///     .include_iter(
    ///         recipients.into_iter().map(|r| client.message("SENDER").send(r, "Promo: 20% off today!"))
    ///     )
    ///     .execute()
    ///     .await?;
    ///
    /// // Flatten results for easy handling
    /// for (i, result) in output.result()?.into_iter().enumerate() {
    ///     match result {
    ///         Ok(res) => println!("Message {i} sent: {}", res.message_id()),
    ///         Err(e) => eprintln!("Failed to send message {i}: {e:#}"),
    ///     }
    /// }
    /// # Ok(()) }
    /// ```
    ///
    /// See [`Batch`] for more details on adding requests and combining results.
    #[cfg(feature = "batch")]
    pub fn batch(&self) -> Batch {
        Batch::new(self)
    }
}

/// Creates a new [`Client`] with default settings.
///
/// The latest supported WhatsApp API version is used.
///
/// # Example
/// ```rust,no_run
/// use whatsapp_business_rs::client::ClientBuilder;
///
/// let builder = ClientBuilder::new();
/// ```
#[derive(Debug)]
pub struct ClientBuilder {
    http: HttpClientBuilder,
    api_version: &'static str,
    error: Option<Error>,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            http: HttpClientBuilder::new(),
            api_version: DEFAULT_API_VERSION,
            error: None,
        }
    }
}

impl ClientBuilder {
    /// Create a new builder with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the request timeout for all WhatsApp API calls.
    ///
    /// If a request takes longer than this, it will error with a timeout.
    ///
    /// # Arguments
    /// * `duration` - Timeout duration
    ///
    /// # Example
    /// ```rust,no_run
    /// # use whatsapp_business_rs::client::ClientBuilder;
    /// # use std::time::Duration;
    /// # let builder = ClientBuilder::new();
    /// let builder = builder.timeout(Duration::from_secs(20));
    /// ```
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.http = self.http.timeout(duration);
        self
    }

    /// Sets the WhatsApp API version to use (e.g. `"19.0"`).
    ///
    /// If you add the `"v"` prefix, it will be removed.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use whatsapp_business_rs::client::ClientBuilder;
    /// # let builder = ClientBuilder::new();
    /// let builder = builder.api_version("19.0");
    /// ```
    pub fn api_version(mut self, version: &'static str) -> Self {
        // we can have static from trimming but not prefixing.
        self.api_version = version;
        self
    }

    /// This method configures the client to use a specific authentication token
    /// for subsequent API requests. It consumes the builder and returns a new one
    /// with the authentication settings applied.
    ///
    /// # Examples
    /// ```rust
    /// use whatsapp_business_rs::client::{ClientBuilder, Auth};
    ///
    /// // Create a builder and add an authentication token
    /// let builder = ClientBuilder::new()
    ///     .auth("your_access_token");
    /// ```
    ///
    /// [`Auth`]: crate::client::Auth
    pub fn auth<'a, A: ToValue<'a, Auth>>(mut self, auth: A) -> Self {
        fn add_auth(headers: &mut HeaderMap, auth: Cow<'_, Auth>) -> Result<(), Error> {
            match auth {
                Cow::Owned(auth) => {
                    headers.insert(reqwest::header::AUTHORIZATION, auth.try_into()?);
                    Ok(())
                }
                Cow::Borrowed(auth) => {
                    headers.insert(reqwest::header::AUTHORIZATION, auth.try_into()?);
                    Ok(())
                }
            }
        }

        let mut headers = HeaderMap::new();
        if let Err(err) = add_auth(&mut headers, auth.to_value()) {
            self.error = Some(err);
            return self;
        }

        self.http = self.http.default_headers(headers);
        self
    }

    /// Finishes building the client and creates a new, unauthenticated or authenticated
    /// instance depending on whether auth was called.
    ///
    /// This method concludes the client configuration and returns a `Client`
    /// instance. It is typically used after setting up other client properties
    /// like the timeout or user agent, but without providing an authentication
    /// token directly.
    ///
    /// To build a client with authentication, you can use the `auth` method
    /// before calling `build`, or use the convenience method `connect`.
    ///
    /// # Returns
    /// A `Result` which is:
    /// - `Ok(Client)`: A new, configured `Client` instance.
    /// - `Err(Error)`: If any of the builder's methods failed or if there are
    ///   other issues during client creation.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::client::ClientBuilder;
    ///
    /// let client = ClientBuilder::new()
    ///     .auth("your_access_token")
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// [`Client`]: crate::client::Client
    /// [`Error`]: crate::error::Error
    pub fn build(self) -> Result<Client, Error> {
        if let Some(err) = self.error {
            return Err(err);
        }

        let http_client = self.http.user_agent(USER_AGENT).build()?;
        Ok(Client {
            inner: Arc::new(InnerClient {
                http_client,
                endpoint: Endpoint::new(self.api_version),
            }),
        })
    }

    /// Finishes building and connects the client using the given credentials.
    ///
    /// This asynchronous method attempts to establish a connection to the WhatsApp
    /// Business API.
    ///
    /// # Arguments
    /// * `auth` - An access token or anything convertible into [`Auth`].
    ///
    /// # Returns
    /// A `Result` which is:
    /// - `Ok(Client)`: On successful connection, a fully configured [`Client`] instance.
    /// - `Err(Error)`: If the connection fails, an internal API call fails, or due to
    ///   other issues.
    ///
    /// # Example
    /// ```rust,no_run
    ///  use whatsapp_business_rs::client::{ClientBuilder, Auth};
    /// # use std::time::Duration;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Connect using a direct token
    /// let client_token = ClientBuilder::new()
    ///      .timeout(Duration::from_secs(10))
    ///      .connect("your_token")
    ///      .await?;
    ///
    /// // Connect using App Secret for permanent token
    /// let client_secret = ClientBuilder::new()
    ///      .connect(Auth::Secret {
    ///          app_id: "YOUR_APP_ID".to_string(),
    ///          app_secret: "YOUR_APP_SECRET".into(),
    ///      })
    ///      .await?;
    /// # Ok(())}
    /// ```
    /// [`Auth`]: crate::client::Auth
    /// [`Client`]: crate::client::Client
    /// [`Error`]: crate::error::Error
    pub async fn connect<'a, A: ToValue<'a, Auth>>(self, auth: A) -> Result<Client, Error> {
        self.auth(auth).build()
    }
}

/// Manager for sending messages, updating conversation status, and managing media.
///
/// This manager provides methods to interact with the WhatsApp Business Cloud API
/// for sending various message types, marking messages as read or "replying" (typing),
/// and handling media uploads and deletions.
///
/// A `MessageManager` instance is typically obtained via [`Client::message()`].
///
/// Note: WhatsApp Business Cloud API **does not** allow retrieving historical messages.
///
/// # Example
/// ```rust,no_run
/// use whatsapp_business_rs::{client::Client, IdentityRef};
///
/// # async fn example_message_manager(client: &Client) {
/// let business_phone_number = IdentityRef::business("1234567890");
/// let message_manager = client.message(business_phone_number);
///
/// let user_phone_number = IdentityRef::user("9876543210");
///
/// // Send a simple text message
/// let metadata = message_manager
///     .send(user_phone_number, "Hello from Rust!")
///     .await
///     .unwrap();
/// println!("Message sent with ID: {}", metadata.message_id());
/// # }
/// ```
#[derive(Debug)]
pub struct MessageManager<'i> {
    pub(crate) client: Client,
    pub(crate) from: Cow<'i, IdentityRef>,
}

impl<'i> MessageManager<'i> {
    /// Create a new message manager
    fn new(from: Cow<'i, IdentityRef>, client: &Client) -> Self {
        Self {
            client: client.clone(),
            from,
        }
    }

    /// Prepares to send a message to a WhatsApp user.
    ///
    /// This method returns a [`SendMessage`] builder. The message is sent when
    /// the returned `SendMessage` instance is `.await`ed.
    ///
    /// Accepts anything that can convert into a `Draft` — such as plain strings, and more.
    ///
    /// # Arguments
    /// * `to` - Recipient identity (e.g., user phone number).
    /// * `draft` - A message or content convertible into a WhatsApp draft (e.g., `String`, `Draft`).
    ///
    /// # Returns
    /// A [`SendMessage`] builder, which can be `.await`ed to send the message.
    ///
    /// # Examples
    ///
    /// ## Sending a simple text message:
    /// ```rust,no_run
    /// use whatsapp_business_rs::{IdentityRef, Draft};
    ///
    /// # async fn example_send_text(manager: whatsapp_business_rs::client::MessageManager<'_>)
    /// # -> Result<(), Box<dyn std::error::Error>> {
    /// let recipient = IdentityRef::user("9876543210");
    ///
    /// // Send text message (awaiting the SendMessage builder directly)
    /// let metadata = manager.send(&recipient, "Hello, how are you?").await?;
    /// println!("Text message sent, ID: {}", metadata.message_id());
    /// # Ok(())}
    /// ```
    ///
    /// ## Sending a rich media message:
    /// ```rust,no_run
    /// use whatsapp_business_rs::{IdentityRef, Draft, message::Media};
    ///
    /// # async fn example_send_media(manager: whatsapp_business_rs::client::MessageManager<'_>)
    /// # -> Result<(), Box<dyn std::error::Error>> {
    /// let recipient = IdentityRef::user("9876543210");
    ///
    /// let image = Media::from_path("path/to/your/image.png").await?;
    /// let draft = Draft::media(image).with_caption("Look at this nice picture!");
    ///
    /// let metadata = manager.send(recipient, draft).await?;
    /// println!("Media message sent, ID: {}", metadata.message_id());
    /// # Ok(())}
    /// ```
    pub fn send<'t, I, D>(&self, to: I, draft: D) -> SendMessage<'t>
    where
        I: ToValue<'t, IdentityRef>,
        D: IntoDraft,
    {
        let body = draft.into_draft().into_request(self, to.to_value());
        SendMessage {
            body,
            request: self.client.post(self.base_url()),
        }
    }

    /// Prepares to mark the chat as "replying..." (typing indicator).
    ///
    /// This notifies the user that your business is replying to their message.
    /// Typically used to improve perceived responsiveness while composing a response.
    ///
    /// This method returns a [`SetReplying`] builder. The action is performed when
    /// the returned `SetReplying` instance is `.await`ed.
    ///
    /// # Arguments
    /// * `to` - A message reference you're replying to (e.g., from an incoming message).
    ///
    /// # Returns
    /// A [`SetReplying`] builder, which can be `.await`ed to set the typing indicator.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::message::MessageRef;
    ///
    /// # async fn example_set_replying(manager: whatsapp_business_rs::client::MessageManager<'_>)
    /// # -> Result<(), Box<dyn std::error::Error>> {
    /// let incoming_message_id = "wamid.ID"; // Replace with an actual incoming message ID
    /// let message_ref = MessageRef::from(incoming_message_id);
    ///
    /// // Mark as replying (typing...)
    /// manager.set_replying(&message_ref).await?;
    /// println!("Typing indicator set.");
    /// # Ok(())}
    /// ```
    #[inline]
    pub fn set_replying<'t, T>(&self, to: T) -> SetReplying<'t>
    where
        T: ToValue<'t, MessageRef>,
    {
        let to = to.to_value();
        SetReplying {
            request: self.client.post(self.base_url()).json(&to.set_typing()),
            _marker: PhantomData,
        }
    }

    /// Prepares to mark a conversation as read.
    ///
    /// This updates the chat to indicate that the user's message has been seen.
    /// This is important for WhatsApp's internal delivery/read tracking.
    ///
    /// This method returns a [`SetRead`] builder. The action is performed when
    /// the returned `SetRead` instance is `.await`ed.
    ///
    /// # Arguments
    /// * `to` - A message reference you are marking as read (e.g., the ID of the last incoming message).
    ///
    /// # Returns
    /// A [`SetRead`] builder, which can be `.await`ed to mark the conversation as read.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::message::MessageRef;
    ///
    /// # async fn example_set_read(manager: whatsapp_business_rs::client::MessageManager<'_>)
    /// # -> Result<(), Box<dyn std::error::Error>> {
    /// let incoming_message_id = "wamid.ID"; // Replace with an actual incoming message ID
    /// let message_ref = MessageRef::from(incoming_message_id);
    ///
    /// // Mark the message as read
    /// manager.set_read(message_ref).await?;
    /// println!("Conversation marked as read.");
    /// # Ok(())}
    /// ```
    #[inline]
    pub fn set_read<'t, T>(&self, to: T) -> SetRead<'t>
    where
        T: ToValue<'t, MessageRef>,
    {
        let to = to.to_value();
        SetRead {
            request: self.client.post(self.base_url()).json(&to.set_read()),
            _marker: PhantomData,
        }
    }

    /// Prepares to upload media (image, audio, video, document) to WhatsApp.
    ///
    /// This method returns an [`UploadMedia`] builder. The upload is performed when
    /// the returned `UploadMedia` instance is `.await`ed.
    ///
    /// # Arguments
    /// * `media` - The raw bytes of the media content to be uploaded.
    /// * `media_type` - The specific type of media (e.g., `MediaType::Image`, `MediaType::Video`).
    ///
    /// # Returns
    /// An `UploadMedia` builder, which can be `.await`ed to perform the upload.
    /// The result of awaiting is the ID of the uploaded media, which can then be
    /// used in messages (e.g., via `Draft::media`).
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn example_upload_media(manager: whatsapp_business_rs::client::MessageManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
    /// # let image_bytes = vec![0; 1024];
    /// let media_id = manager
    ///     .upload_media(image_bytes, "image/png".parse().unwrap(), "image.jpeg")
    ///     .await?;
    /// println!("Media uploaded with ID: {}", media_id);
    /// # Ok(())}
    /// ```
    #[inline]
    pub fn upload_media(
        &self,
        media: Vec<u8>,
        media_type: MediaType,
        filename: impl Into<Cow<'static, str>>,
    ) -> UploadMedia {
        self.upload_media_inner(media, media_type.mime_type(), filename)
    }

    /// Prepares to delete previously uploaded media from WhatsApp servers.
    ///
    /// This method returns a [`DeleteMedia`] builder. The deletion is performed when
    /// the returned `DeleteMedia` instance is `.await`ed.
    ///
    /// Useful for cleaning up storage or invalidating content you no longer want accessible.
    ///
    /// # Arguments
    /// * `media_id` - The ID of the media to delete (must have been uploaded previously).
    ///
    /// # Returns
    /// A `DeleteMedia` builder, which can be `.await`ed to perform the deletion.
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn example_delete_media(manager: whatsapp_business_rs::client::MessageManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
    /// let uploaded_media_id = "your_media_id_to_delete";
    /// manager.delete_media(uploaded_media_id).await?;
    /// println!("Media ID {} deleted.", uploaded_media_id);
    /// # Ok(())}
    /// ```
    #[inline]
    pub fn delete_media(&self, media_id: &str) -> DeleteMedia {
        self.client.delete_media(media_id)
    }

    // To the identity not message
    #[inline(always)]
    pub(crate) fn base_url(&self) -> Endpoint<'_, 2> {
        self.endpoint("messages")
    }

    Endpoint! {from.phone_id}
}

/// A builder for sending a message.
///
/// This struct is returned by [`MessageManager::send`]. It does not perform
/// the network request until it is `.await`ed (due to its `IntoFuture` implementation)
/// or its `execute().await` method is called.
#[must_use = "SendMessage does nothing unless you `.await` or `.execute().await` it"]
pub struct SendMessage<'t> {
    body: MessageRequestOutput<'t>,
    request: RequestBuilder,
}

impl SendMessage<'_> {
    /// Specifies the authentication token to use for sending this message.
    ///
    /// This is especially helpful for applications managing messages across multiple
    /// WhatsApp Business Accounts (WBAs) or different `Client` configurations.
    /// You can reuse your `MessageManager` and apply the correct token
    /// for each message send operation dynamically.
    ///
    /// If not called, the request will use the authentication configured with the `Client`
    /// used to create this `MessageManager`.
    ///
    /// # Parameters
    /// - `auth`: [`Auth`] token to use for this specific request.
    ///
    /// [`Auth`]: crate::client::Auth
    pub fn with_auth<'a>(mut self, auth: impl ToValue<'a, Auth>) -> Self {
        let auth = auth.to_value();
        self.body = self.body.with_auth(Cow::Borrowed(&*auth));
        self.request = add_auth_to_request!(auth => self.request);
        self
    }
}

IntoFuture! {
    impl<'t> SendMessage<'t> {
        /// Executes the message send request.
        ///
        /// This method performs the network operation. Because `SendMessage`
        /// implements `IntoFuture`, you can also simply `.await` the
        /// `SendMessage` instance directly, which will call this method internally.
        ///
        /// # Returns
        /// `Ok(MessageCreate)` on success, containing metadata about the sent message.
        /// `Err(Error)` if the request fails.
        ///
        /// # Example
        /// ```rust,no_run
        /// use whatsapp_business_rs::{client::MessageManager, IdentityRef, Draft};
        ///
        /// # async fn example_execute_send(manager: MessageManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
        /// let recipient = IdentityRef::user("9876543210");
        /// let draft = Draft::text("Hello!");
        ///
        /// // Preferred: Await directly
        /// manager.send(&recipient, draft.clone()).await?;
        ///
        /// // Alternative: Call .execute().await
        /// manager.send(&recipient, draft).execute().await?;
        /// # Ok(()) }
        /// ```
        pub async fn execute(self) -> Result<MessageCreate, Error> {
            let body = self.body.execute().await?;
            let request = self.request.json(&body);
            fut_net_op(request).await
        }
    }
}

#[cfg(feature = "batch")]
impl<'a> crate::batch::Requests for SendMessage<'a> {
    type BatchHandler = SendMessageHandler<'a>;

    type ResponseReference = SendMessageResponseReference;

    fn into_batch_ref(
        self,
        f: &mut crate::batch::Formatter,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), crate::batch::FormatError> {
        let (h, body) = self.body.into_batch_ref(f)?;
        let mut request = f.add_request(self.request.json(&body))?;
        let reference = SendMessageResponseReference {
            reference_id: request.get_name(),
        };

        let _debug = request.finish()?;
        let handler = SendMessageHandler {
            #[cfg(debug_assertions)]
            endpoint: _debug.url.into(),
            dep_handler: h,
        };

        Ok((handler, reference))
    }

    fn into_batch(
        self,
        f: &mut crate::batch::Formatter,
    ) -> Result<Self::BatchHandler, crate::batch::FormatError>
    where
        Self: Sized,
    {
        let (h, body) = self.body.into_batch_ref(f)?;
        let request = f.add_request(self.request.json(&body))?;
        let _debug = request.finish()?;

        Ok(SendMessageHandler {
            #[cfg(debug_assertions)]
            endpoint: _debug.url.into(),
            dep_handler: h,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (1 + self.body.size_hint().0, None)
    }

    requests_batch_include! {}
}

/// A symbolic reference to the response of a `SendMessage` request.
///
/// This type can be used directly in other requests that expect a
/// `MessageRef`. This enables you to compose dependent requests
/// within a batch without needing the concrete ID or field value upfront.
///
/// ## Notes
/// - Still only valid inside a batch context.
/// - Should not be inspected at runtime (no field access).
/// - Implements `Clone` so it can be reused in multiple dependents.
/// - Implements [`ToValue`] so it can stand in for `MessageRef`.
#[derive(Clone)]
#[cfg(feature = "batch")]
pub struct SendMessageResponseReference {
    reference_id: Cow<'static, str>,
}

#[cfg(feature = "batch")]
impl ToValue<'_, MessageRef> for SendMessageResponseReference {
    fn to_value(self) -> Cow<'static, MessageRef> {
        let id = self.reference_id;
        let ref_message_id = reference!(id => SendMessageResponse => [messages[0]] [id]);
        Cow::Owned(MessageRef::from(ref_message_id))
    }
}

#[cfg(feature = "batch")]
impl<'a> ToValue<'a, MessageRef> for &'a SendMessageResponseReference {
    fn to_value(self) -> Cow<'static, MessageRef> {
        let id = &self.reference_id;
        let ref_message_id = reference!(id => SendMessageResponse => [messages[0]] [id]);
        Cow::Owned(MessageRef::from(ref_message_id))
    }
}

#[cfg(feature = "batch")]
pub struct SendMessageHandler<'a> {
    #[cfg(debug_assertions)]
    endpoint: Cow<'static, str>,
    dep_handler: <MessageRequestOutput<'a> as crate::batch::Requests>::BatchHandler,
}

#[cfg(feature = "batch")]
impl crate::batch::Handler for SendMessageHandler<'_> {
    type Responses = Result<MessageCreate, Error>;

    fn from_batch(
        self,
        response: &mut crate::batch::BatchResponse,
    ) -> Result<Self::Responses, crate::batch::FromResponseError> {
        // First read the possible dep requests... all we're interested
        // in is the error and reading past
        self.dep_handler.from_batch(response)?;

        response.handle_next_typical(
            #[cfg(debug_assertions)]
            self.endpoint,
        )
    }
}

// The internals of SendMessage are okay being nulled
#[cfg(feature = "batch")]
NullableUnit! {SendMessage<'a,>}

SimpleOutput! {
    SetReplying<'a> => ()
}

/// A symbolic reference to the response of a `SetReplying` request.
///
/// This type is a placeholder for the eventual response and cannot be
/// inspected directly. It is only valid within a batch context, where it
/// can be passed to `.then()` or `.then_nullable()` to compose dependent
/// requests.
///
/// ## Notes
/// - Not usable outside of batch building.
/// - Cannot be read or matched against at runtime.
/// - Opaque: only pass it to request builders.
/// - Implements `Clone` so it can be reused across multiple dependents.
#[derive(Clone)]
#[cfg(feature = "batch")]
pub struct SetReplyingResponseReference {
    _priv: (),
}

#[cfg(feature = "batch")]
impl crate::batch::IntoResponseReference for SetReplying<'_> {
    type ResponseReference = SetReplyingResponseReference;

    fn into_response_reference(_: Cow<'static, str>) -> Self::ResponseReference {
        Self::ResponseReference { _priv: () }
    }
}

SimpleOutput! {
    SetRead<'a> => ()
}

/// A symbolic reference to the response of a `SetRead` request.
///
/// This type is a placeholder for the eventual response and cannot be
/// inspected directly. It is only valid within a batch context, where it
/// can be passed to `.then()` or `.then_nullable()` to compose dependent
/// requests.
///
/// ## Notes
/// - Not usable outside of batch building.
/// - Cannot be read or matched against at runtime.
/// - Opaque: only pass it to request builders.
/// - Implements `Clone` so it can be reused across multiple dependents.
#[derive(Clone)]
#[cfg(feature = "batch")]
pub struct SetReadResponseReference {
    _priv: (),
}

#[cfg(feature = "batch")]
impl crate::batch::IntoResponseReference for SetRead<'_> {
    type ResponseReference = SetReadResponseReference;

    fn into_response_reference(_: Cow<'static, str>) -> Self::ResponseReference {
        Self::ResponseReference { _priv: () }
    }
}

/// A builder for uploading media to WhatsApp.
///
/// This struct is returned by [`MessageManager::upload_media`]. It does not perform
/// the network request until it is `.await`ed (due to its `IntoFuture` implementation)
/// or its `execute().await` method is called.
#[must_use = "UploadMedia does nothing unless you `.await` or `.execute().await` it"]
#[derive(Debug)]
pub struct UploadMedia {
    pub(crate) request: RequestBuilder,
    pub(crate) media: reqwest::multipart::Part,
}

impl UploadMedia {
    /// Specifies the authentication token to use for this media upload request.
    ///
    /// This is useful for applications handling media across different WhatsApp Business Accounts (WBAs)
    /// or with varying permissions. You can reuse an existing `MessageManager` and
    /// provide the specific token required for this upload operation.
    ///
    /// If not called, the request will use the authentication configured with the `Client`
    /// used to create this `MessageManager`.
    ///
    /// # Parameters
    /// - `auth`: [`Auth`] token to use for this specific request.
    ///
    /// [`Auth`]: crate::client::Auth
    #[inline]
    pub fn with_auth<'a>(self, auth: impl ToValue<'a, Auth>) -> Self {
        <UploadMedia as crate::rest::IntoMessageRequestOutput>::with_auth(self, auth.to_value())
    }
}

IntoFuture! {
    impl UploadMedia {
        /// Executes the media upload request.
        ///
        /// This method performs the network operation. Because `UploadMedia`
        /// implements `IntoFuture`, you can also simply `.await` the
        /// `UploadMedia` instance directly, which will call this method internally.
        ///
        /// # Returns
        /// `Ok(String)` on success, containing the ID of the newly uploaded media.
        /// `Err(Error)` if the request fails.
        ///
        /// # Example
        /// ```rust,no_run
        /// # use whatsapp_business_rs::client::MessageManager;
        /// # async fn example_execute_upload(manager: MessageManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
        /// # let video_bytes = vec![0; 1024];
        /// // Preferred: Await directly
        /// let media_id_1 = manager
        ///     .upload_media(video_bytes.clone(), "video/mp4".parse().unwrap(), "video.mp4")
        ///     .await?;
        ///
        /// // Alternative: Call .execute().await
        /// let media_id_2 = manager
        ///     .upload_media(video_bytes, "video/mp4".parse().unwrap(), "video.mp4")
        ///     .execute()
        ///     .await?;
        /// # Ok(()) }
        /// ```
        #[inline]
        pub async fn execute(self) -> Result<String, Error> {
            <UploadMedia as crate::rest::IntoMessageRequestOutput>::execute(self).await
        }
    }
}

/// A symbolic reference to the response of a `UploadMedia` request.
///
/// This type is a placeholder for the eventual response and cannot be
/// inspected directly. It is only valid within a batch context, where it
/// can be passed to `.then()` or `.then_nullable()` to compose dependent
/// requests.
///
/// ## Notes
/// - Not usable outside of batch building.
/// - Cannot be read or matched against at runtime.
/// - Opaque: only pass it to request builders.
/// - Implements `Clone` so it can be reused across multiple dependents.
#[derive(Clone, Debug)]
#[cfg(feature = "batch")]
pub struct UploadMediaResponseReference(pub(crate) String);

#[cfg(feature = "batch")]
impl From<UploadMediaResponseReference> for MediaSource {
    fn from(value: UploadMediaResponseReference) -> Self {
        Self::Id(value.0)
    }
}

#[cfg(feature = "batch")]
NullableUnit! {UploadMedia <>}

SimpleOutput! {
    DeleteMedia => ()
}

/// A symbolic reference to the response of a `DeleteMedia` request.
///
/// This type is a placeholder for the eventual response and cannot be
/// inspected directly. It is only valid within a batch context, where it
/// can be passed to `.then()` or `.then_nullable()` to compose dependent
/// requests.
///
/// ## Notes
/// - Not usable outside of batch building.
/// - Cannot be read or matched against at runtime.
/// - Opaque: only pass it to request builders.
/// - Implements `Clone` so it can be reused across multiple dependents.
#[derive(Clone)]
#[cfg(feature = "batch")]
pub struct DeleteMediaResponseReference {
    _priv: (),
}

#[cfg(feature = "batch")]
impl crate::batch::IntoResponseReference for DeleteMedia {
    type ResponseReference = DeleteMediaResponseReference;

    fn into_response_reference(_: Cow<'static, str>) -> Self::ResponseReference {
        Self::ResponseReference { _priv: () }
    }
}

// TODO: This isn't that much to clone around... remove double
// arc-ing
#[derive(Debug)]
pub(crate) struct InnerClient {
    http_client: HttpClient,
    endpoint: Endpoint<'static>,
}

// Endpoint with no version
// If we could parse once
pub(crate) const GRAPH_ENDPOINT: &str = "https://graph.facebook.com";

#[derive(Clone, Copy, Debug)]
pub(crate) struct Endpoint<'a, const N: usize = 0> {
    // TODO: For the data it normally contains, this is too much
    api_version: Option<&'static str>,
    parts: [&'a str; N],
}

impl<'a> Endpoint<'a> {
    #[inline]
    fn new(api_version: &'static str) -> Self {
        // The onus of triming v is on us(hehe)... so we can't
        // have default endpoint as we're not const
        Self {
            api_version: Some(api_version.trim_matches('v')),
            parts: [],
        }
    }

    #[cfg(feature = "batch")]
    pub(crate) const fn without_version() -> Self {
        Self {
            api_version: None,
            parts: [],
        }
    }
}

impl<'a, const N: usize> Endpoint<'a, N> {
    // returning String because if we decide to parse here, we can't
    // construct reqwest error back into the RB.... We'd benefit as
    // as a stream of char to the url parser so we can totally avoid the heap.
    // so sad..
    #[inline]
    pub(crate) fn as_url(&self) -> String /*Cow<'static, str>*/ {
        const GRAPH: usize = GRAPH_ENDPOINT.len();
        const V: usize = "v".len();
        const SLASH: usize = "/".len();

        use std::ops::Add;

        // FIXME: We could return GRAPH_ENDPOINT HERE if no version and part

        /* "https://graph.facebook.com" / v$api_version $(/ $path)+ */
        let size = GRAPH
            + if let Some(version) = self.api_version {
                SLASH
                + (V + version.len())
            } else {
                0
            }
            // We have / before the first also
            + self.parts.iter().map(|part| part.len()).fold(SLASH * N, usize::add);

        let mut url = String::with_capacity(size);

        /* "https://graph.facebook.com" / v$api_version $(/ $path)+ */
        url.push_str(GRAPH_ENDPOINT);
        if let Some(version) = self.api_version {
            url.push_str("/v");
            url.push_str(version);
        }
        self.parts.iter().for_each(|part| {
            url.push('/');
            url.push_str(part)
        });

        url
    }
}

// We can't return {N + 1} on stable
macro_rules! decl_endpoint {
    ($N:tt => $Nplus1:tt) => {
        impl<'a> Endpoint<'a, $N> {
            #[inline]
            pub(crate) const fn join(self, part: &'a str) -> Endpoint<'a, $Nplus1> {
                Endpoint {
                    api_version: self.api_version,
                    parts: unsafe {
                        std::mem::transmute::<([&str; $N], &str), [&str; $N + 1]>((
                            self.parts, part,
                        ))
                    },
                }
            }
        }
    };
}
decl_endpoint! {0 => 1}
decl_endpoint! {1 => 2}
// decl_endpoint! {2 => 3}

impl Client {
    #[inline(always)]
    pub(crate) fn endpoint(&self) -> Endpoint<'static, 0> {
        self.inner.endpoint
    }

    #[inline(always)]
    pub(crate) fn a_node<'a>(&self, node: &'a str) -> Endpoint<'a, 1> {
        self.endpoint().join(node)
    }

    // TODO: Since we don't benefit from going to String, we can just reuse this endpoint
    // in our error message instead of stealing like cavemen
    //
    // TODO: Use raw Endpoint
    #[inline]
    pub(crate) fn post<'a, const N: usize>(&self, url: Endpoint<'a, N>) -> RequestBuilder {
        let url = url.as_url();
        self.inner.http_client.post(url)
    }

    #[inline]
    pub(crate) fn get<'a, const N: usize>(&self, url: Endpoint<'a, N>) -> RequestBuilder {
        let url = url.as_url();
        self.inner.http_client.get(url)
    }

    #[inline]
    pub(crate) fn delete<'a, const N: usize>(&self, url: Endpoint<'a, N>) -> RequestBuilder {
        let url = url.as_url();
        self.inner.http_client.delete(url)
    }

    #[inline]
    pub(crate) fn get_external<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        // We can't expose the auth
        // We've set it as default so we gotta override it...
        self.inner.http_client.get(url).bearer_auth("REDACTED")
    }
}

// because it occured too often
impl Client {
    pub(crate) async fn get_access_token(
        &self,
        request: AccessTokenRequest<'_>,
    ) -> Result<Token, Error> {
        let request = self
            .get(self.endpoint().join("oauth/access_token"))
            .query(&request);

        fut_net_op(request).await
    }
}

impl Display for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            Auth::Token(token) => f.write_str(&token.0),
            Auth::Secret { app_id, app_secret } => write!(f, "{app_id}|{app_secret}"),
            Auth::Parsed(parsed_auth) => parsed_auth
                .header
                .to_str()
                .map_err(|_| std::fmt::Error)
                .and_then(|s| {
                    if let Some(auth) = s.strip_prefix("Bearer ") {
                        f.write_str(auth)
                    } else {
                        Ok(())
                    }
                }),
        }
    }
}

#[derive(thiserror::Error, Debug)]
#[error("{inner}")]
pub struct AuthParsedError {
    inner: reqwest::header::InvalidHeaderValue,
}

impl From<reqwest::header::InvalidHeaderValue> for Error {
    fn from(value: reqwest::header::InvalidHeaderValue) -> Self {
        Error::internal(value.into())
    }
}

impl TryFrom<Auth> for reqwest::header::HeaderValue {
    type Error = reqwest::header::InvalidHeaderValue;

    fn try_from(value: Auth) -> Result<Self, Self::Error> {
        match value {
            Auth::Token { .. } | Auth::Secret { .. } => {
                format!("Bearer {value}")
                    .try_into()
                    .map(|mut h: HeaderValue| {
                        h.set_sensitive(true);
                        h
                    })
            }
            Auth::Parsed(parsed) => Ok(parsed.header),
        }
    }
}

impl TryFrom<&Auth> for reqwest::header::HeaderValue {
    type Error = reqwest::header::InvalidHeaderValue;

    fn try_from(value: &Auth) -> Result<Self, Self::Error> {
        match value {
            Auth::Token { .. } | Auth::Secret { .. } => {
                format!("Bearer {value}")
                    .try_into()
                    .map(|mut h: HeaderValue| {
                        h.set_sensitive(true);
                        h
                    })
            }
            Auth::Parsed(parsed) => Ok(parsed.header.clone()),
        }
    }
}

impl Display for TokenAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Display for AppSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

to_value! {
    Auth TokenAuth AppSecret
}

impl ToValue<'_, TokenAuth> for String {
    #[inline]
    fn to_value(self) -> Cow<'static, TokenAuth> {
        Cow::Owned(TokenAuth(self))
    }
}

// For with_auth: Useless as we can't go Auth
impl<'a> ToValue<'a, TokenAuth> for &'a String {
    #[inline]
    fn to_value(self) -> Cow<'a, TokenAuth> {
        let token_auth: &TokenAuth = unsafe { transmute(self) };
        Cow::Borrowed(token_auth)
    }
}

impl ToValue<'_, TokenAuth> for &str {
    #[inline]
    fn to_value(self) -> Cow<'static, TokenAuth> {
        Cow::Owned(TokenAuth(self.to_owned()))
    }
}

impl<T: Into<String>> From<T> for AppSecret {
    #[inline]
    fn from(value: T) -> Self {
        AppSecret(value.into())
    }
}

impl ToValue<'_, Auth> for String {
    #[inline]
    fn to_value(self) -> Cow<'static, Auth> {
        Cow::Owned(Auth::token(self))
    }
}

impl ToValue<'_, Auth> for &String {
    #[inline]
    fn to_value(self) -> Cow<'static, Auth> {
        Cow::Owned(Auth::token(self))
    }
}

impl ToValue<'_, Auth> for &str {
    #[inline]
    fn to_value(self) -> Cow<'static, Auth> {
        Cow::Owned(Auth::token(self))
    }
}

impl<'s, I, S> ToValue<'_, Auth> for (I, S)
where
    I: Into<String> + Send + Sync,
    S: ToValue<'s, AppSecret>,
{
    #[inline]
    fn to_value(self) -> Cow<'static, Auth> {
        Cow::Owned(Auth::secret(self))
    }
}

impl Deref for TokenAuth {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> ToValue<'a, IdentityRef> for MessageManager<'a> {
    #[inline]
    fn to_value(self) -> Cow<'a, IdentityRef> {
        self.from
    }
}

impl<'a> ToValue<'a, IdentityRef> for &'a MessageManager<'a> {
    #[inline]
    fn to_value(self) -> Cow<'a, IdentityRef> {
        Cow::Borrowed(self.from.as_ref())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    // CD test
    #[test]
    fn ub() {
        let auth = "ExihfkdjojfOEFJJJP".to_owned();

        let token_auth: Cow<'_, TokenAuth> = (&auth).to_value();

        assert_eq!(token_auth.0, "ExihfkdjojfOEFJJJP");

        let token_auth_clone = token_auth.clone();

        assert_eq!(token_auth_clone, token_auth);
    }

    #[test]
    fn endpoint() {
        #[allow(non_local_definitions)]
        impl<'a, const N: usize> Endpoint<'a, N> {
            // This may not be correct... it's fine since it's for test and we know
            // our intention
            fn level(&self) -> usize {
                N
            }
        }

        // param makes them different so can't have in an array together
        macro_rules! test {
            ($endpoint:expr, $want:literal, $level:tt) => {
                let got = $endpoint.as_url();
                assert_eq!(got, $want);
                assert_eq!(got.capacity(), $want.len());
                assert_eq!($endpoint.level(), $level);
            };
        }

        test!(Endpoint::new("20.0"), "https://graph.facebook.com/v20.0", 0);
        test!(
            Endpoint::new("v20.0"),
            "https://graph.facebook.com/v20.0",
            0
        );
        test!(
            Endpoint::new("v20.0").join("path"),
            "https://graph.facebook.com/v20.0/path",
            1
        );
        test!(
            Endpoint::new("20.0").join("path").join("path1"),
            "https://graph.facebook.com/v20.0/path/path1",
            2
        );
        test!(
            Endpoint::new("v20.0").join("oauth/access_token"),
            "https://graph.facebook.com/v20.0/oauth/access_token",
            1
        ); // not 2;
    }
}
