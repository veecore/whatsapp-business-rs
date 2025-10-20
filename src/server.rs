//! WhatsApp webhook server implementation
//!
//! Provides a high-level API for receiving and processing WhatsApp events
//! through webhooks. Handles verification, signature validation, and event routing.
//!
//! This module provides a high-level `Server` that handles all the networking,
//! routing, and lifecycle management for you. It's the easiest way to get started.
//!
//! For a more flexible, low-level integration, see the [`crate::webhook_service`] module.
//!
//! # Key Components
//! - [`ServerBuilder`]: Configure the server's endpoint, route, shutdown signal, and security.
//! - [`Server`]: The configured server, ready to run.
//! - [`Server::serve`]: A method that takes your [`Handler`] and returns a `Future`
//!   which runs the server until it's shut down.
//!
//! # Examples
//!
//! ## Starting a server
//! ```rust,no_run
//! use whatsapp_business_rs::{
//!     client::Client,
//!     server::{Server, Handler, EventContext, IncomingMessage},
//!     Error
//! };
//!
//! struct MyHandler;
//!
//! impl Handler for MyHandler {
//!     async fn handle_message(&self, _ctx: EventContext, msg: IncomingMessage) {
//!         println!("{:#?}", msg);
//!         // If feature incoming_message_ext is enabled
//!         # #[cfg(feature = "incoming_message_ext")]
//!         # {
//!         msg.reply("Thanks for your message!").await.unwrap();
//!         # }
//!     }
//! }
//!
//! # async fn example() {
//! let server = Server::builder()
//!     .endpoint("127.0.0.1:8080".parse().unwrap())
//!     .build();
//!
//! // If feature incoming_message_ext is not enabled
//! # #[cfg(not(feature = "incoming_message_ext"))]
//! # {
//! server.serve(MyHandler).await.unwrap();
//! # }
//!
//! // If feature incoming_message_ext is enabled.
//! // Client is needed for interacting with the API.
//! # #[cfg(feature = "incoming_message_ext")]
//! # {
//! let client = Client::new("ACCESS_TOKEN").await.unwrap();    
//! server.serve(MyHandler, client).await.unwrap();
//! # }
//! # }
//! ```
//!
//! ## Handling different events
//! ```rust,no_run
//! use whatsapp_business_rs::{
//!     server::{Event, EventContext, Handler, IncomingMessage, MessageUpdate},
//!     Error,
//! };
//!
//! struct MyHandler;
//!
//! impl Handler for MyHandler {
//!     async fn handle(&self, ctx: EventContext, event: Event) {
//!         match event {
//!             Event::IncomingMessage(msg) => {
//!                 println!("New message from {}", msg.sender.phone_id);
//!                 self.handle_message(ctx, msg).await
//!             },
//!             Event::MessageUpdate(update) => {
//!                 println!("Message status update: {:?}", update);
//!             },
//!             Event::Waba(waba_event) => {
//!                 println!("WABA event: {:?}", waba_event);
//!             },
//!             _ => ()
//!         }
//!     }
//!
//!     async fn handle_message(&self, _ctx: EventContext, msg: IncomingMessage) {
//!         // If feature incoming_message_ext is enabled
//!         # #[cfg(feature = "incoming_message_ext")]
//!         # {    
//!         msg.reply("Got your message").await.unwrap();
//!         # }
//!     }
//! }
//! ```

use crate::app::ConfigureWebhook;
use crate::client::AppSecret;
use crate::message::IntoDraft;
use crate::rest::server::deserialize_origin;
use crate::{Timestamp, ToValue, Waba};

use std::borrow::Cow;
use std::fmt::{Debug, Display};
use std::mem;
use std::{future::Future, net::SocketAddr, ops::Deref, pin::Pin, sync::Arc};

use futures::pin_mut;
use serde::Deserialize;
use tokio::sync::Notify;

use crate::{
    error::Error,
    message::{Message, MessageRef, MessageStatus},
};

#[cfg(feature = "incoming_message_ext")]
use crate::client::{Client, SendMessage, SetRead, SetReplying};
#[cfg(feature = "incoming_message_ext")]
use crate::message::Reaction;

// Default Server configuration...
const DEFAULT_ENDPOINT: &str = "127.0.0.1:3000";
const DEFAULT_ROUTE_PATH: &str = "/";

/// WhatsApp webhook server
///
/// Listens for incoming events from WhatsApp and routes them to a [`Handler`].
/// Create using [`Server::builder()`] or [`Server::new()`].
#[derive(Default)]
pub struct Server {
    pub(crate) config: ServerBuilder,
}

impl Server {
    /// Create a new server with default settings
    pub fn new() -> Self {
        ServerBuilder::new().build()
    }

    /// Create a server builder for custom configuration
    pub fn builder() -> ServerBuilder {
        ServerBuilder::new()
    }

    /// Prepares the webhook server to start listening for incoming messages.
    ///
    /// This method returns a [`Serve`] struct that acts as a builder for the server's lifecycle.
    /// The server will **not** start listening until the returned `Serve` instance is `.await`ed.
    ///
    /// Use [`Serve::configure_webhook`] to automatically register your webhook with Meta
    /// as part of the server startup process.
    ///
    /// # Arguments
    /// - `handler`: Your custom [`WebhookHandler`] implementation, which defines how your application
    ///   responds to various incoming webhook events.
    ///
    /// # Optional Arguments (`incoming_message_ext` feature)
    /// - `client`: An authenticated WhatsApp [`Client`] instance. This is required if the
    ///   `incoming_message_ext` feature is enabled, as it provides extended functionality
    ///   for processing incoming messages (e.g., replying).
    ///
    /// # Examples
    ///
    /// ## Starting the server without Meta webhook registration:
    /// ```rust,no_run
    /// # use whatsapp_business_rs::{Server, WebhookHandler};
    /// # #[cfg(feature = "incoming_message_ext")] use whatsapp_business_rs::Client;
    /// # async fn example_no_reg(server: Server) {
    /// # struct MyHandler;
    /// # impl WebhookHandler for MyHandler {}
    /// # let handler = MyHandler;
    /// # #[cfg(feature = "incoming_message_ext")]
    /// # {
    /// let client = Client::new("YOUR_TOKEN").await.unwrap();
    /// server.serve(handler, client).await.unwrap();
    /// # }
    /// # #[cfg(not(feature = "incoming_message_ext"))]
    /// # {
    /// server.serve(handler).await.unwrap();
    /// # }
    /// # }
    /// ```
    ///
    /// ## Starting the server and registering the webhook with Meta:
    /// (See [`Serve::configure_webhook`] for a more complete example)
    /// ```rust,no_run
    /// use whatsapp_business_rs::Fields;
    ///
    /// # #[cfg(feature = "incoming_message_ext")] use whatsapp_business_rs::Client;
    /// # async fn example_with_reg(server: whatsapp_business_rs::Server,
    /// # app_manager: whatsapp_business_rs::app::AppManager<'_>,
    /// # webhook_config: whatsapp_business_rs::app::WebhookConfig) {
    /// # struct MyHandler;
    /// # impl whatsapp_business_rs::WebhookHandler for MyHandler {}
    /// # let handler = MyHandler;
    /// let events = Fields::all(); // Subscribe to all events
    ///
    /// let pending_registration = app_manager
    ///     .configure_webhook(webhook_config)
    ///     .events(events);
    ///
    /// // If feature incoming_message_ext is enabled
    /// # #[cfg(feature = "incoming_message_ext")]
    /// # {
    /// let client = Client::new("YOUR_TOKEN").await.unwrap();
    /// server
    ///    .serve(handler, client)
    ///    .configure_webhook(pending_registration)
    ///    .await
    ///    .unwrap();
    /// # }
    ///
    /// // If feature incoming_message_ext is not enabled
    /// # #[cfg(not(feature = "incoming_message_ext"))]
    /// # {
    /// server
    ///    .serve(handler)
    ///    .configure_webhook("very_secret_token", pending_registration)
    ///    .await
    ///    .unwrap();
    /// # }
    /// # }
    /// ```
    ///
    /// [`WebhookHandler`]: crate::WebhookHandler
    /// [`Client`]: crate::Client
    /// [`Serve`]: crate::server::Serve
    /// [`Serve::configure_webhook`]: crate::server::Serve::configure_webhook
    #[cfg(not(feature = "incoming_message_ext"))]
    pub fn serve<H: Handler + 'static>(self, handler: H) -> Serve<H> {
        Serve {
            server: self,
            handler,
            pending_register: None,
        }
    }

    #[cfg(feature = "incoming_message_ext")]
    pub fn serve<H: Handler + 'static>(self, handler: H, client: Client) -> Serve<H> {
        Serve {
            server: self,
            client,
            handler,
            pending_register: None,
        }
    }
}

/// A builder for running the webhook server, with optional Meta webhook registration.
///
/// This struct allows you to configure and start the WhatsApp webhook server.
/// It implements `IntoFuture`, meaning you can `.await` an instance of `Serve`
/// directly to start the server.
///
/// Use [`Serve::configure_webhook`] to associate a pending webhook registration
/// with this server, which will then be performed automatically when the server starts.
#[must_use = "Serve does nothing unless you `.await` or `.execute().await` it"]
pub struct Serve<H> {
    server: Server,
    #[cfg(feature = "incoming_message_ext")]
    client: Client,
    handler: H,
    pending_register: Option<ConfigureWebhook<'static>>,
}

impl<H: Handler + 'static> Serve<H> {
    /// See [`Serve::register_webhook`]
    #[deprecated(
        since = "0.3.0",
        note = "This method is error-prone due to `verify_token` repetition.\
            Use `register_webhook` for a safer and simpler API."
    )]
    pub fn configure_webhook(
        mut self,
        verify_token: impl Into<String>,
        pending: ConfigureWebhook<'static>,
    ) -> Self {
        self.pending_register = Some(pending);
        self.server.config.verify_token = Some(verify_token.into());
        self
    }

    /// Configures an associated webhook registration task that will run when the server starts.
    ///
    /// This method takes a `ConfigureWebhook` instance (obtained from [`configure_webhook`])
    /// that has *not* yet been `.await`ed. When this `Serve` instance is `.await`ed,
    /// the `pending` webhook registration will be executed.
    ///
    /// The server will wait for the webhook registration to complete before fully starting
    /// to serve requests. If registration fails, the server will be gracefully shut down.
    ///
    /// # Parameters
    /// - `pending`: A `ConfigureWebhook` instance representing the desired webhook registration.
    ///   **Important**: Do not `.await` this `ConfigureWebhook` instance before passing it here;
    ///   it should be the builder returned by [`configure_webhook`].
    ///
    /// # Returns
    /// The updated `Serve` instance, ready to be `.await`ed to start the server.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::{Fields, app::{SubscriptionField, WebhookConfig}};
    /// # #[cfg(feature = "incoming_message_ext")] use whatsapp_business_rs::Client;
    /// # struct __Handler;
    /// # impl whatsapp_business_rs::WebhookHandler for __Handler {}
    ///
    /// # async fn example_full(serve: whatsapp_business_rs::server::Serve<__Handler>,
    /// # handler: impl whatsapp_business_rs::WebhookHandler + 'static,
    /// # app_manager: whatsapp_business_rs::app::AppManager<'_>)
    /// # -> Result<(), Box<dyn std::error::Error>> {
    /// let events = Fields::new()
    ///      .with(SubscriptionField::Messages)
    ///      .with(SubscriptionField::AccountUpdate);
    ///
    /// // 1. Prepare the webhook registration (DO NOT await it yet)
    /// let pending_registration = app_manager
    ///     .configure_webhook(WebhookConfig {
    ///         webhook_url: "https:///example.com/webhook".to_string(),
    ///         verify_token: "very_secret_token".into(),
    ///     })
    ///     .events(events);
    ///
    /// // 2. Create the server builder and associate the pending registration
    /// serve
    ///    .register_webhook(pending_registration)
    ///    .await?; // Await to start server and trigger webhook registration
    ///
    /// # Ok(()) }
    /// ```
    ///
    /// [`configure_webhook`]: crate::app::AppManager::configure_webhook
    pub fn register_webhook(mut self, pending: ConfigureWebhook<'static>) -> Self {
        // We extract the token from the request definition itself.
        // No more redundant parameters!
        let token = pending.request.query.a.verify_token.clone().into_owned();
        self.server.config.verify_token = Some(token);

        self.pending_register = Some(pending);
        self
    }

    /// Prepares a future that, when awaited, will signal the server to shut down.
    ///
    /// This method modifies the server's internal shutdown configuration to include
    /// a new shutdown signal. Awaiting the returned future will trigger this signal.
    /// If a previous shutdown mechanism was configured, this new signal will be
    /// combined with it, meaning either can trigger the server's shutdown.
    pub fn shutdown_trigger(&mut self) -> impl Future<Output = ()> + Send + 'static {
        let shutdown = Arc::new(Notify::new());
        let shutdown_clone = shutdown.clone();

        let old_shutdown = mem::take(&mut self.server.config.shutdown);

        self.server.config.shutdown = Some(Self::combine_shutdown(old_shutdown, async move {
            shutdown_clone.notified().await;
        }));

        async move {
            shutdown.notify_one();
        }
    }

    /// Combines an optional existing shutdown future with a new one.
    /// The returned future completes when either the old or the new shutdown future completes.
    fn combine_shutdown<S, F>(
        old_shutdown: Option<S>,
        update_shutdown: F,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>
    where
        S: Future<Output = ()> + Send + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        match old_shutdown {
            Some(shutdown) => Box::pin(async {
                tokio::select! {
                    _ = shutdown => (),
                    _ = update_shutdown => (),
                }
            }),
            None => Box::pin(update_shutdown),
        }
    }
}

IntoFuture! {
    impl<H> Serve<H>
    [where
        H: Handler + 'static,]
    {
        pub fn execute(mut self) -> impl Future<Output = Result<(), Error>> + 'static {
        async {
            if self.pending_register.is_none() {
                self.server
                    .serve_inner(
                        self.handler,
                        #[cfg(feature = "incoming_message_ext")]
                        self.client,
                        None,
                    )
                    .await
            } else {
                // This shutdown trigger will be used if registration fails.
                let registration_failure_shutdown_trigger = self.shutdown_trigger();

                let pending_register = self.pending_register.unwrap();

                // Notifies the registration task when the server is ready to accept connections.
                let server_is_ready_notifier = Arc::new(Notify::new());
                let server_is_ready_notifier_clone = server_is_ready_notifier.clone();

                // Registration task: Awaits server readiness, then executes registration.
                let reg_task = tokio::spawn(async move {
                    server_is_ready_notifier_clone.notified().await;
                    pending_register.execute().await
                });

                // Create server future. It will notify `server_is_ready_notifier` when ready
                let server_fut = self.server.serve_inner(
                    self.handler,
                    #[cfg(feature = "incoming_message_ext")]
                    self.client,
                    // with this guy
                    Some(server_is_ready_notifier),
                );

                pin_mut!(server_fut);

                tokio::select! {
                    reg_result = reg_task => {
                        match reg_result {
                            Ok(Ok(())) => {
                                // Registration succeeded, now await server
                                server_fut.await
                            }
                            Ok(Err(e)) => {
                                // Registration failed - trigger shutdown
                                registration_failure_shutdown_trigger.await;
                                Err(e)
                            }
                            Err(join_err) => panic!("Webhook registration task panicked {join_err}"),
                        }
                    }

                    server_result = &mut server_fut => {
                        // Server finished first (e.g., due to an external shutdown signal
                        // or an internal error).
                        server_result
                    }
                }
            }
        }
        }
    }
}

/// Builder for creating a [`Server`]
///
/// Customize endpoint, route, and shutdown signal.
///
/// # Example
/// ```rust,no_run
/// use whatsapp_business_rs::Server;
///
/// # async fn example() {
/// let server = Server::builder()
///     .endpoint("127.0.0.1:8080".parse().unwrap())
///     .build();
/// # }
/// ```
#[must_use]
pub struct ServerBuilder {
    pub(crate) endpoint: SocketAddr,
    pub(crate) route_path: String,
    pub(crate) shutdown: Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    pub(crate) app_secret: Option<AppSecret>,
    pub(crate) verify_token: Option<String>,
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_ENDPOINT.parse().unwrap(),
            route_path: DEFAULT_ROUTE_PATH.to_owned(),
            shutdown: None,
            app_secret: None,
            verify_token: None,
        }
    }
}

impl ServerBuilder {
    /// Create a new builder with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the network address and port on which the webhook server will listen.
    ///
    /// This specifies the `IP address:port` combination for the server to bind to.
    /// For example, `127.0.0.1:8080` for local access or `0.0.0.0:8080` to listen
    /// on all available network interfaces.
    ///
    /// # Arguments
    /// * `endpoint` - A `SocketAddr` specifying the IP address and port
    ///   (e.g., `SocketAddr::from(([127, 0, 0, 1], 8080))`).
    ///
    /// # Returns
    /// The `ServerBuilder` instance, allowing for method chaining.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::server::ServerBuilder;
    /// use std::net::SocketAddr;
    ///
    /// let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    /// let builder = ServerBuilder::new().endpoint(addr);
    /// ```
    pub fn endpoint(mut self, endpoint: SocketAddr) -> Self {
        self.endpoint = endpoint;
        self
    }

    /// Sets the URL path for the WhatsApp webhook endpoint.
    ///
    /// This is the path where the server will expect to receive incoming
    /// webhook events from Meta. The default path is `/whatsapp_webhook`.
    ///
    /// # Arguments
    /// * `path` - A value that can be converted into a `String`, representing
    ///   the desired webhook route (e.g., "/my_webhook_receiver").
    ///
    /// # Returns
    /// The `ServerBuilder` instance, allowing for method chaining.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::server::ServerBuilder;
    ///
    /// let builder = ServerBuilder::new().route("/my_custom_webhook");
    /// ```
    pub fn route<P: Into<String>>(mut self, path: P) -> Self {
        self.route_path = path.into();
        self
    }

    /// Sets a custom `Future` that, when resolved, will trigger the server to shut down.
    ///
    /// This provides a mechanism for graceful server shutdown, allowing you to
    /// integrate with your application's termination logic (e.g., listening for
    /// a signal).
    ///
    /// # Note on Usage
    /// For most common scenarios, consider using the more convenient
    /// [`Serve::shutdown_trigger`] method which provides the future for
    /// signaling shutdown. This `shutdown` method is for more advanced
    /// custom future-based shutdown requirements.
    ///
    /// # Arguments
    /// * `shutdown` - A `Future` that resolves to `()` and implements `Send` and `'static`.
    ///
    /// # Returns
    /// The `ServerBuilder` instance, allowing for method chaining.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::server::ServerBuilder;
    /// use tokio::signal;
    ///
    /// # async fn example() {
    /// let shutdown_signal = async {
    ///     signal::ctrl_c().await.expect("failed to listen for ctrl-c");
    ///     println!("Ctrl-C received, shutting down...");
    /// };
    ///
    /// let builder = ServerBuilder::new().shutdown(shutdown_signal);
    /// # }
    /// ```
    pub fn shutdown<F>(mut self, shutdown: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.shutdown = Some(Box::pin(shutdown));
        self
    }

    /// Configures the server to **verify the authenticity of incoming webhook payloads**
    /// from WhatsApp using the app secret.
    ///
    /// When enabled, the server will compute a signature based on the received payload
    /// and your provided `app_secret`. This computed signature is then compared
    /// against the `X-Hub-Signature-256` header sent by Meta. If they don't match,
    /// the request is considered unauthenticated and will be rejected, preventing
    /// potential spoofing or malicious requests.
    ///
    /// This feature requires your Meta App Secret to perform the verification.
    ///
    /// # Arguments
    /// * `app_secret` - Your Meta App's secret, which can be any type
    ///   convertible to an [`AppSecret`].
    ///
    /// # Returns
    /// The `ServerBuilder` instance, allowing for method chaining.
    ///
    /// # Security Note 🔒
    /// It is **highly recommended** to enable payload verification in production
    /// environments to ensure that all received webhook events genuinely originate
    /// from Meta's servers.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::server::ServerBuilder;
    ///
    /// let builder = ServerBuilder::new()
    ///     .verify_payload("YOUR_META_APP_SECRET");
    /// ```
    pub fn verify_payload<'s>(mut self, app_secret: impl ToValue<'s, AppSecret>) -> Self {
        self.app_secret = Some(app_secret.to_value().into_owned());
        self
    }

    /// Sets a verification token for the webhook endpoint's challenge-response handshake.
    ///
    /// This method is specifically for configuring the server to respond to Meta's
    /// webhook verification challenge, *even if you are not using the convenient
    /// [`Serve::configure_webhook`] method* to register the endpoint with Meta.
    ///
    /// During webhook setup in the Meta Developer Console, you provide a "Verify Token".
    /// When Meta attempts to verify your webhook URL, it sends a GET request
    /// to your endpoint containing a `hub.verify_token` query parameter.
    /// Your server must respond with the exact same token provided in the console.
    ///
    /// By calling this method, you instruct the server to echo back the provided
    /// `verify_token` if it receives a valid verification challenge.
    ///
    /// # Arguments
    /// * `verify_token` - The arbitrary string you configured as the "Verify Token"
    ///   in the Meta Developer Console for your webhook.
    ///
    /// # Returns
    /// The `ServerBuilder` instance, allowing for method chaining.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::server::ServerBuilder;
    ///
    /// let builder = ServerBuilder::new()
    ///     .verify_token("MY_SECURE_VERIFICATION_TOKEN");
    /// ```
    pub fn verify_token(mut self, verify_token: impl Into<String>) -> Self {
        self.verify_token = Some(verify_token.into());
        self
    }

    /// Builds and returns a [`Server`] instance from the configured `ServerBuilder`.
    ///
    /// This method consumes the `ServerBuilder` and creates the final `Server`
    /// object, ready to be started to listen for webhook events.
    ///
    /// # Returns
    /// A `Server` instance configured with the settings from the builder.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::server::ServerBuilder;
    /// use std::net::SocketAddr;
    ///
    /// let addr: SocketAddr = "0.0.0.0:8080".parse().unwrap();
    /// let server = ServerBuilder::new()
    ///     .endpoint(addr)
    ///     .route("/whatsapp")
    ///     .build();
    /// // The 'server' is now ready to be run (server.serve(handler).await)
    /// ```
    pub fn build(self) -> Server {
        Server { config: self }
    }
}

#[derive(Debug)]
pub struct ErrorContext {
    pub(crate) _priv: (),
}

/// WhatsApp webhook event
///
/// This enum represents all the supported events
/// that can be received from the WhatsApp webhook.
#[allow(clippy::large_enum_variant)]
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum Event {
    /// A user sent a message to your business
    IncomingMessage(IncomingMessage),

    /// A message's delivery or read status was updated
    MessageUpdate(MessageUpdate),

    /// Partner WABA-related event (e.g. partner added)
    Waba(WabaEvent),
}

/// Context metadata about an incoming event
///
/// Includes the originating business and the time the event was received.
pub struct EventContext {
    /// The business this event belongs to
    pub(crate) waba: Waba,

    /// When the event was received (usually unix timestamp in seconds)
    pub(crate) event_received_at: Timestamp,
}

impl EventContext {
    /// Get the associated business
    pub fn waba(&self) -> &Waba {
        &self.waba
    }

    /// Get the timestamp when the event was received
    pub fn event_received_at(&self) -> Timestamp {
        self.event_received_at
    }
}

/// Event handler trait
///
/// Implement this to process incoming WhatsApp events.
///
/// # Default Implementations
/// The trait provides default implementations for all event types,
/// allowing you to override only the events you care about.
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::{server::{Handler, EventContext, IncomingMessage}, Error};
///
/// struct MyHandler;
///
/// impl Handler for MyHandler {
///     async fn handle_message(&self, _ctx: EventContext, msg: IncomingMessage) -> () {
///         println!("Received message: {:?}", msg.content);
///         // If feature incoming_message_ext is enabled
///         # #[cfg(feature = "incoming_message_ext")]
///         # {    
///         msg.reply("Hello back!").await.unwrap();
///         # }
///     }
/// }
/// ```
pub trait Handler: Send + Sync {
    /// Handle any WhatsApp event
    ///
    /// This is the entry point for all events. The default implementation
    /// routes events to specific handlers based on type.
    #[inline]
    fn handle(&self, ctx: EventContext, event: Event) -> impl Future<Output = ()> + Send {
        async {
            match event {
                Event::IncomingMessage(msg) => self.handle_message(ctx, msg).await,
                Event::MessageUpdate(update) => self.handle_message_update(ctx, update).await,
                Event::Waba(waba_event) => self.handle_waba_event(ctx, waba_event).await,
            }
        }
    }

    /// Handle incoming messages
    fn handle_message(
        &self,
        _ctx: EventContext,
        _msg: IncomingMessage,
    ) -> impl Future<Output = ()> + Send + '_ {
        async {}
    }

    /// Handle message status updates
    fn handle_message_update(
        &self,
        _ctx: EventContext,
        _update: MessageUpdate,
    ) -> impl Future<Output = ()> + Send + '_ {
        async {}
    }

    /// Handle waba event
    fn handle_waba_event(
        &self,
        _ctx: EventContext,
        _waba_event: WabaEvent,
    ) -> impl Future<Output = ()> + Send + '_ {
        async {}
    }

    /// Handle internal-server error
    fn handle_error(
        &self,
        _ctx: ErrorContext,
        error: Box<dyn std::error::Error + Send>,
    ) -> impl Future<Output = ()> + Send + '_ {
        use std::time::{SystemTime, UNIX_EPOCH};

        async move {
            // Get the current time since UNIX_EPOCH
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();

            // Format as seconds + milliseconds
            let secs = now.as_secs();
            let millis = now.subsec_millis();

            eprintln!("[{secs}.{millis:03}] Server error: {error}");
        }
    }
}

impl<F, Fut> Handler for F
where
    Fut: Future<Output = ()> + Send,
    F: FnOnce(EventContext, Event) -> Fut + Send + Sync + Clone,
{
    #[inline]
    fn handle(&self, ctx: EventContext, event: Event) -> impl Future<Output = ()> + Send {
        (self.clone())(ctx, event)
    }
}

/// Status update for a previously sent message
///
/// Indicates delivery status, timestamps, and pricing data.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MessageUpdate {
    /// Involved message
    pub message: MessageRef,

    /// Updated delivery status (e.g. delivered, read)
    pub status: MessageStatus,

    /// When the status update occurred (usually unix timestamp in seconds)
    pub timestamp: Option<Timestamp>,

    /// Additional context (pricing, conversation ID, errors)
    pub context: MessageUpdateContext,
}

impl MessageUpdate {
    /// Data originally attached to the message (e.g., custom tags)
    pub fn callback(&self) -> Option<&str> {
        self.context.biz_opaque_callback_data.as_deref()
    }

    /// If message is in transit within WhatsApp systems
    pub fn is_accepted(&self) -> bool {
        matches!(self.status, MessageStatus::Accepted)
    }

    /// If message is delivered to device
    pub fn is_delivered(&self) -> bool {
        matches!(self.status, MessageStatus::Delivered)
    }

    /// If message is read by recipient
    pub fn is_read(&self) -> bool {
        matches!(self.status, MessageStatus::Read)
    }

    /// If message failed to send
    pub fn failed(&self) -> bool {
        matches!(self.status, MessageStatus::Failed)
    }

    /// If message is sent to WhatsApp
    pub fn is_sent(&self) -> bool {
        matches!(self.status, MessageStatus::Sent)
    }

    /// If catalog item in message is unavailable
    pub fn is_warning(&self) -> bool {
        matches!(self.status, MessageStatus::Warning)
    }

    /// If message was deleted by sender
    pub fn is_deleted(&self) -> bool {
        matches!(self.status, MessageStatus::Deleted)
    }
}

use crate::MetaError as WhatsAppError;

/// Extra metadata attached to a status update
///
/// Includes pricing, conversation IDs, and opaque callback data.
#[derive(Deserialize, Clone, Debug)]
#[non_exhaustive]
pub struct MessageUpdateContext {
    /// Data originally attached to the message (e.g., custom tags)
    #[serde(default)]
    pub biz_opaque_callback_data: Option<String>,

    /// Pricing data for the message
    #[serde(default)]
    pub pricing: Option<MessagePricing>,

    /// Conversation-level metadata
    #[serde(default)]
    pub conversation: Option<ConversationInfo>,

    /// Any WhatsApp platform errors related to this message
    #[serde(default)]
    pub errors: Vec<WhatsAppError>,
}

/// Metadata about the conversation this message is part of
#[derive(Deserialize, Clone, Debug)]
#[non_exhaustive]
pub struct ConversationInfo {
    /// Unique conversation ID
    pub id: String,

    /// Origin type
    #[serde(deserialize_with = "deserialize_origin", default)]
    pub origin: Option<ConversationOrigin>,

    /// When the conversation will expire, if known
    #[serde(default)]
    pub expiration_timestamp: Option<Timestamp>,
}

derive! {
    /// Pricing metadata for a WhatsApp message
    #[derive(#AnyField, Clone, Debug)]
    #[non_exhaustive]
    pub struct MessagePricing {
        /// What type of conversation this falls under
        #![anyfield("category", "conversation_origin")]
        #![serde(default)]
        pub conversation_origin: Option<ConversationOrigin>,

        /// Whether this message is billable
        #![serde(default)]
        pub billable: Option<bool>,

        /// Pricing model name (e.g. "CBP")
        #![serde(default)]
        pub pricing_model: Option<String>,
    }
}

/// The type of conversation being billed
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConversationOrigin {
    /// Any type not enumerated
    #[serde(untagged)]
    Other(String),
}

/// Webhook event related to WABA partnership
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum WabaEvent {
    /// A new WABA was added to your App
    PartnerAdded(PartnerAdded),
}

/// Data for a new partner added to your App
#[derive(Deserialize, Clone, Debug)]
#[non_exhaustive]
pub struct PartnerAdded {
    /// WhatsApp Business Account ID
    pub waba_id: String,

    /// Meta Business Manager ID that owns this WABA
    pub owner_business_id: String,
}

/// Incoming message with reply capabilities
///
/// Represents a message received through webhooks. Provides methods
/// for replying and accessing message.
#[derive(Clone)]
pub struct IncomingMessage {
    /// The received message
    #[cfg(not(feature = "incoming_message_ext"))]
    pub message: Message,

    #[cfg(feature = "incoming_message_ext")]
    pub(crate) message: Message,

    /// When the message was received (usually unix timestamp in seconds)
    pub timestamp: Option<Timestamp>,

    #[cfg(feature = "incoming_message_ext")]
    /// Internal client for sending replies
    pub(crate) client: Client,
}

impl IncomingMessage {
    /// When the message was received (usually unix timestamp in seconds)
    #[inline]
    pub fn timestamp(&self) -> Option<&Timestamp> {
        self.timestamp.as_ref()
    }
}

impl IncomingMessage {
    /// Consume the wrapper and get the raw [`Message`].
    #[inline]
    pub fn into_inner(self) -> Message {
        self.message
    }

    /// Get a reference to the inner [`Message`].
    #[inline]
    pub fn message(&self) -> &Message {
        &self.message
    }
}

#[cfg(feature = "incoming_message_ext")]
impl IncomingMessage {
    /// Reply to this message
    ///
    /// # Example
    /// ```rust,no_run
    /// # use whatsapp_business_rs::{message::{Draft, Media}, server::IncomingMessage};
    /// # async fn example(msg: IncomingMessage, media: Media) {
    /// // Simple text reply
    /// msg.reply("Thanks for your message!").await.unwrap();
    ///
    /// // Custom reply with media
    /// let draft = Draft::media(media)
    ///     .with_caption("Here's what you asked for");
    /// msg.reply(draft).await.unwrap();
    /// # }
    /// ```
    #[inline]
    pub fn reply<'i, D>(&'i self, draft: D) -> SendMessage<'i>
    where
        D: IntoDraft,
    {
        self.client
            .message(&self.message.recipient)
            .send(&self.message.sender, draft)
    }

    /// *Swipe Reply* to this message
    ///
    /// # Example
    /// ```rust,no_run
    /// # use whatsapp_business_rs::{message::{Draft, Media}, server::IncomingMessage};
    /// # async fn example(msg: IncomingMessage, media: Media) {
    /// msg.swipe_reply("Thanks for this message!").await.unwrap();
    /// # }
    /// ```
    #[inline]
    pub fn swipe_reply<'i, D>(&'i self, draft: D) -> SendMessage<'i>
    where
        D: IntoDraft,
    {
        let draft = draft.into_draft();
        let draft = draft.reply_to(self.as_ref_no_metadata());
        self.reply(draft)
    }

    /// Set "replying" indicator for this message
    ///
    /// This causes the reply bubble in WhatsApp UI. Only lasts ~25s.
    #[inline]
    pub fn set_replying<'m>(&'m self) -> SetReplying<'m> {
        self.client
            .message(&self.recipient)
            .set_replying(self.as_ref_no_metadata())
    }

    /// Mark the message as read
    #[inline]
    pub fn set_read<'m>(&'m self) -> SetRead<'m> {
        self.client
            .message(&self.recipient)
            .set_read(self.as_ref_no_metadata())
    }

    /// React to this message
    #[inline]
    pub fn react<'m>(&'m self, emoji: char) -> SendMessage<'m> {
        self.reply(Reaction::new(emoji, self.as_ref_no_metadata()))
    }
}

impl Deref for IncomingMessage {
    type Target = Message;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl ToValue<'_, MessageRef> for IncomingMessage {
    #[inline]
    fn to_value(self) -> Cow<'static, MessageRef> {
        self.message.to_value()
    }
}

impl ToValue<'_, MessageRef> for &IncomingMessage {
    #[inline]
    fn to_value(self) -> Cow<'static, MessageRef> {
        (&self.message).to_value()
    }
}

#[cfg(feature = "incoming_message_ext")]
impl IncomingMessage {
    #[inline]
    fn as_ref_no_metadata(&self) -> MessageRef {
        self.message.id.clone().into()
    }
}

impl Display for IncomingMessage {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Message as Display>::fmt(&self.message, f)
    }
}

impl Debug for IncomingMessage {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Message as Debug>::fmt(&self.message, f)
    }
}

impl IntoDraft for IncomingMessage {
    #[inline]
    fn into_draft(self) -> crate::Draft {
        self.message.into_draft()
    }
}

impl<M> PartialEq<M> for IncomingMessage
where
    Message: PartialEq<M>,
{
    fn eq(&self, other: &M) -> bool {
        self.message.eq(other)
    }
}

impl<M> PartialEq<M> for MessageUpdate
where
    MessageRef: PartialEq<M>,
{
    fn eq(&self, other: &M) -> bool {
        self.message.eq(other)
    }
}
