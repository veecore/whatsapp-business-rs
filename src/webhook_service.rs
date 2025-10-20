//! For "Bring Your Own Server" (BYOS) integrations.
//!
//! This module provides a low-level `WebhookService` that encapsulates the request
//! handling logic for the WhatsApp webhook. It is designed to be integrated into any
//! web server framework that uses standard `http` types, such as `axum`, `hyper`,
//! or `warp`.
//!
//! For a fully managed server, see the [`crate::server`] module.
//!
//! # Key Components
//!
//! - [`WebhookServiceBuilder`]: A builder to configure the service with your
//!   `verify_token` and `app_secret`.
//! - [`WebhookService`]: The handler service. It's `Clone`, `Send`, `Sync`, and `'static'`,
//!   making it suitable for use as shared state in any web framework.
//! - [`WebhookService::handle`]: The single, asynchronous method that processes an
//!   incoming `http::Request` and returns an `http::Response`.
//!
//! # Usage Example (with axum)
//!
//! ```rust,no_run
//! use whatsapp_business_rs::{
//!     webhook_service::WebhookService,
//!     Client,
//!     WebhookHandler,
//!     server::{EventContext, Event},
//! };
//! use axum::{Router, routing::post};
//!
//! #[derive(Clone)]
//! struct MyHandler;
//!
//! impl WebhookHandler for MyHandler {
//!     async fn handle(&self, _ctx: EventContext, event: Event) {
//!         println!("Received event: {:?}", event);
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let handler = MyHandler;
//!     let client = Client::new("YOUR_WHATSAPP_TOKEN").await.unwrap();
//!
//! # #[cfg(feature = "incoming_message_ext")] {
//!     // 1. Build the service
//!     let service = WebhookService::builder()
//!         .verify_token("my_secret_token")
//!         .verify_payload("my_app_secret")
//!         .build(handler, client);
//!
//!     // 2. Integrate into your router
//!     let app = Router::new()
//!         .route("/webhook", post({
//!             // The service can be cloned and used directly as a handler
//!             let service = service.clone();
//!             move |req| service.handle(req)
//!         }));
//!
//!     // 3. Run your server
//!     let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
//!     axum::serve(listener, app).await.unwrap();
//! # }
//! }
//! ```
use crate::{
    ToValue,
    client::AppSecret,
    rest::server::{
        InnerServer, handle_verification, handle_webhook, handle_webhook_verify_payload,
    },
    server::Handler,
};
use axum::response::IntoResponse;
use http::{Request, Response, StatusCode};
use std::sync::Arc;

#[cfg(feature = "incoming_message_ext")]
use crate::client::Client;

/// A builder for creating a [`WebhookService`].
///
/// This builder is for the low-level, "Bring Your Own Server" API.
/// It does **not** configure server details like endpoint or shutdown signals.
#[derive(Debug, Default, Clone)]
#[must_use]
pub struct WebhookServiceBuilder {
    app_secret: Option<AppSecret>,
    verify_token: Option<String>,
}

impl WebhookServiceBuilder {
    /// Creates a new builder with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configures the service to verify incoming payloads using your app secret.
    ///
    /// See [`crate::server::ServerBuilder::verify_payload`] for details.
    pub fn verify_payload<'s>(mut self, app_secret: impl ToValue<'s, AppSecret>) -> Self {
        self.app_secret = Some(app_secret.to_value().into_owned());
        self
    }

    /// Sets the verification token for the webhook challenge-response handshake.
    ///
    /// See [`crate::server::ServerBuilder::verify_token`] for details.
    pub fn verify_token(mut self, verify_token: impl Into<String>) -> Self {
        self.verify_token = Some(verify_token.into());
        self
    }

    /// Builds and returns a [`WebhookService`] from the configured builder.
    ///
    /// This method consumes the builder and creates the final service object,
    /// ready to be integrated into your web server.
    ///
    /// # Arguments
    /// * `handler` - Your custom [`Handler`] implementation.
    #[cfg(not(feature = "incoming_message_ext"))]
    pub fn build<H: Handler + 'static>(self, handler: H) -> WebhookService<H> {
        WebhookService {
            inner: Arc::new(InnerServer {
                handler,
                app_secret: self.app_secret,
                verify_token: self.verify_token,
            }),
        }
    }

    /// Builds and returns a [`WebhookService`] from the configured builder.
    ///
    /// This method consumes the builder and creates the final service object,
    /// ready to be integrated into your web server.
    ///
    /// # Arguments
    /// * `handler` - Your custom [`Handler`] implementation.
    /// * `client` - An authenticated WhatsApp [`Client`] instance.
    #[cfg(feature = "incoming_message_ext")]
    pub fn build<H: Handler + 'static>(self, handler: H, client: Client) -> WebhookService<H> {
        WebhookService {
            inner: Arc::new(InnerServer {
                client,
                handler,
                app_secret: self.app_secret,
                verify_token: self.verify_token,
            }),
        }
    }
}

// Not stable
pub type Body = axum::body::Body;

/// A low-level service to handle WhatsApp webhook requests.
///
/// This struct is created using [`WebhookService::builder`] and is designed
/// to be integrated into an existing web server.
#[derive(Clone)]
pub struct WebhookService<H: Handler + 'static> {
    // TODO: Evaluate as fn at once at build to avoid unnecessary runtime
    // checks
    inner: Arc<InnerServer<H, Option<AppSecret>, Option<String>>>,
}

impl<H: Handler + 'static> WebhookService<H> {
    /// Returns a new builder to create a `WebhookService`.
    pub fn builder() -> WebhookServiceBuilder {
        WebhookServiceBuilder::new()
    }

    /// The primary request handler for your BYOS server.
    ///
    /// This single function handles both GET (verification) and POST (payload)
    /// requests by delegating to the appropriate internal logic. It is generic axum::serve(listener, app)
    /// to be used with `axum`, `hyper`, `warp`, and other `http`-compatible frameworks.
    pub async fn handle<B>(&self, req: Request<B>) -> Response<Body>
    where
        B: Into<Body>,
    {
        let req = req.map(Into::into);
        let response = match *req.method() {
            http::Method::GET => {
                // This request is for verification.
                if let Some(verify_token) = &self.inner.verify_token {
                    let state = self.state_with_verify_token(verify_token);
                    <fn(_, _) -> _ as axum::handler::Handler<_, _>>::call(
                        handle_verification,
                        req,
                        state,
                    )
                    .await
                } else {
                    // No verify token configured, so verification is not possible.
                    (
                        StatusCode::METHOD_NOT_ALLOWED,
                        "GET method not supported without a verify_token configured.",
                    )
                        .into_response()
                }
            }
            http::Method::POST => {
                // This request is a webhook payload.
                if let Some(app_secret) = &self.inner.app_secret {
                    // Verify payload
                    let state = self.state_with_app_secret(app_secret);
                    <fn(_, _, _) -> _ as axum::handler::Handler<_, _>>::call(
                        handle_webhook_verify_payload,
                        req,
                        state,
                    )
                    .await
                } else {
                    let state = self.inner.clone();
                    <fn(_, _) -> _ as axum::handler::Handler<_, _>>::call(
                        handle_webhook,
                        req,
                        state,
                    )
                    .await
                }
            }
            _ => (StatusCode::METHOD_NOT_ALLOWED).into_response(),
        };
        response.map(Into::<Body>::into)
    }

    fn state_with_verify_token<'a>(
        &'a self,
        _verify_token: &'a String,
    ) -> Arc<InnerServer<H, Option<AppSecret>, String>> {
        // TODO: Remove... this is because I noticed I was getting too stuck.
        // SAFETY: Option<Cow<'static, String>> has same layout as Cow<'static, String>
        unsafe { std::mem::transmute(self.inner.clone()) }
    }

    fn state_with_app_secret<'a>(
        &'a self,
        _app_secret: &'a AppSecret,
    ) -> Arc<InnerServer<H, AppSecret, String>> {
        // TODO: Remove... this is because I noticed I was getting too stuck.
        // SAFETY: Option<AppSecret> has same layout as AppSecret
        unsafe { std::mem::transmute(self.inner.clone()) }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn ub() {
        struct H;
        impl Handler for H {}
        let s = WebhookServiceBuilder::new()
            .verify_token("verify_token")
            .verify_payload("app_secret")
            .build(H);

        // Prepare request so that a field on state is accessed
        let request = Request::get("http://example.com?hub.challenge=lskjdfriueowdiemoqzpkfr&hub.verify_token=verify_token").body(()).unwrap();
        s.handle(request).await;

        let request = Request::post("http://example.com")
            .header("x-hub-signature-256", "sha256=esmkl2ed")
            .body([0u8; 1024].to_vec())
            .unwrap();
        s.handle(request).await;
    }
}
