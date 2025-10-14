// #![deny(missing_docs)]
#![deny(clippy::future_not_send)]
#![deny(clippy::large_enum_variant)]
#![allow(private_bounds)]
#![allow(private_interfaces)]
#![cfg_attr(nightly_rust, feature(impl_trait_in_assoc_type))]

//! # whatsapp_business_rs
//!
//! A comprehensive Rust SDK for interacting with the Meta WhatsApp Business Platform.
//! This crate provides robust and type-safe abstractions for sending and receiving WhatsApp messages,
//! managing WhatsApp Business Accounts (WABAs), configuring webhooks, and handling product catalogs.
//!
//! ## ✨ Features
//!
//! - **Message Management**: Construct, send, and receive various message types, including text, media,
//!   interactive messages (buttons, lists), and reactions.
//! - **Client API**: A fluent builder for authenticating and managing interactions with the WhatsApp Business API.
//! - **Webhook Server**: Easily set up a webhook server to receive and process incoming messages and
//!   message status updates with signature validation.
//! - **App Management**: Configure webhook subscriptions and manage onboarding flows for connecting businesses to your app.
//! - **WABA Management**: Administer your WhatsApp Business Account, including listing catalogs,
//!   managing phone numbers, and running guided phone number registration flows.
//! - **Catalog Management**: Programmatically manage your product catalogs, allowing you to list,
//!   create, and update products.
//!
//! ## 🚀 Examples
//!
//! Here are some quick examples to get you started:
//!
//! ---
//!
//! ### Create a Client
//! ```rust,no_run
//! use std::time::Duration;
//! use whatsapp_business_rs::client::Client;
//!
//! # async fn create_client_example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::builder()
//!     .timeout(Duration::from_secs(15))
//!     .api_version("v19.0")
//!     .connect("YOUR_ACCESS_TOKEN")
//!     .await?;
//! # Ok(()) }
//! ```
//!
//! ---
//!
//! ### Send a Simple Text Message
//! ```rust,no_run
//! use whatsapp_business_rs::{Client, Draft};
//!
//! # async fn send_text_example() -> Result<(), Box<dyn std::error::Error>> {
//! // Initialize your client with your access token
//! let client = Client::new("YOUR_ACCESS_TOKEN").await?;
//!
//! // Replace with your WhatsApp Phone Number ID and recipient's phone number
//! let sender_phone_id = "YOUR_BUSINESS_PHONE_NUMBER_ID";
//! let recipient_phone_number = "+16012345678";
//!
//! // Send a text message
//! client
//!     .message(sender_phone_id)
//!     .send(recipient_phone_number, Draft::text("Hello from Rust! How can I help you today?"))
//!     .await?;
//!
//! println!("Text message sent!");
//! # Ok(())
//! # }
//! ```
//!
//! ---
//!
//! ### Send Bulk Messages with Batch 🚀
//! ```rust,no_run
//! use whatsapp_business_rs::Client;
//!
//! # #[cfg(feature = "batch")]
//! # async fn send_batch_example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::new("YOUR_ACCESS_TOKEN").await?;
//!
//! let sender = client.message("business_phone_id");
//! let batch = client
//!     .batch()
//!         .include(sender.send("+1234567890", "Hi A!"))
//!         .include(sender.send("+1234667809", "Hi B!"))
//!         .include(sender.send("+1224537891", "Hi C!"));
//!
//! batch.execute().await?;
//!
//! # Ok(())
//! # }
//! ```
//!
//! ---
//!
//! ### Send a Media Message (e.g., Video)
//! ```rust,no_run
//! use whatsapp_business_rs::{Client, message::Media};
//!
//! # async fn send_video_example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::new("YOUR_ACCESS_TOKEN").await?;
//!
//! let sender_phone_id = "YOUR_BUSINESS_PHONE_NUMBER_ID";
//! let recipient_phone_number = "+16012345678";
//!
//! // Example: Send a video from a file path
//! // Make sure to replace "path/to/your/video.mp4" with an actual path
//! let video = Media::from_path("path/to/your/video.mp4")
//!     .await?
//!     .caption("Check out this cool video!");
//!
//! client.message(sender_phone_id).send(recipient_phone_number, video).await?;
//! println!("Video message sent!");
//! # Ok(())
//! # }
//! ```
//!
//! ---
//!
//! ### Send a Location Message
//! ```rust,no_run
//! use whatsapp_business_rs::{Client, message::Location};
//!
//! # async fn send_location_example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::new("YOUR_ACCESS_TOKEN").await?;
//!
//! let sender_phone_id = "YOUR_BUSINESS_PHONE_NUMBER_ID";
//! let recipient_phone_number = "+16012345678";
//!
//! // Send a location message with name and address
//! let location = Location::new(37.44216251868683, -122.16153582049394)
//!     .name("Philz Coffee")
//!     .address("101 Forest Ave, Palo Alto, CA 94301");
//!
//! client.message(sender_phone_id).send(recipient_phone_number, location).await?;
//! println!("Location message sent!");
//! # Ok(())
//! # }
//! ```
//!
//! ---
//!
//! ### Send an Interactive Message (Call-to-Action Button)
//! ```rust,no_run
//! use whatsapp_business_rs::message::{InteractiveMessage, UrlButton};
//! use whatsapp_business_rs::Client;
//!
//! # async fn send_cta_example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::new("YOUR_ACCESS_TOKEN").await?;
//!
//! let sender_phone_id = "YOUR_BUSINESS_PHONE_NUMBER_ID";
//! let recipient_phone_number = "+16012345678";
//!
//! // Create a URL button
//! let button = UrlButton::new("https://example.com/purchase", "2 for price of 1");
//!
//! // Create an interactive message with the button and a body text
//! let interactive_message = InteractiveMessage::new(
//!     button,
//!     "One-time offer for this shiny product. Hurry-up!"
//! );
//!
//! client.message(sender_phone_id).send(recipient_phone_number, interactive_message).await?;
//! println!("Call-to-Action message sent!");
//! # Ok(())
//! # }
//! ```
//!
//! ---
//!
//! ### Send an Interactive Message (List of Options)
//! ```rust,no_run
//! use whatsapp_business_rs::message::{InteractiveMessage, OptionList, OptionButton, Section};
//! use whatsapp_business_rs::Client;
//!
//! # async fn send_option_list_example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::new("YOUR_ACCESS_TOKEN").await?;
//!
//! let sender_phone_id = "YOUR_BUSINESS_PHONE_NUMBER_ID";
//! let recipient_phone_number = "+16012345678";
//!
//! // Create sections with options
//! let section = Section::new(
//!     "Choose an option regarding your last order",
//!     [
//!         OptionButton::new("Cancels deliver", "Cancel", "cancel-delivery"),
//!         OptionButton::new("Proceed with order", "Order", "proceed-with-order"),
//!     ],
//! );
//!
//! // Create an option list with the section and a global label
//! let options = OptionList::new_section(section, "Delivery Options");
//!
//! // Create an interactive message with the option list, body, and footer
//! let interactive_message = InteractiveMessage::new(
//!     options,
//!     "Sorry your delivery is delayed... would you like to cancel the order?",
//! ).footer("Only 70% in refund");
//!
//! client.message(sender_phone_id).send(recipient_phone_number, interactive_message).await?;
//! println!("Option list message sent!");
//! # Ok(())
//! # }
//! ```
//!
//! ---
//!
//! ### Configure a Webhook
//! ```rust,no_run
//! use whatsapp_business_rs::{
//!     app::SubscriptionField,
//!     client::Client,
//!     App,
//! };
//!
//! # async fn configure_webhook_example() -> Result<(), Box<dyn std::error::Error>> {
//! let app = App::new("YOUR_APP_ID");
//!
//! let client = Client::new("YOUR_ACCESS_TOKEN").await?;
//! client
//!     .app(app)
//!     .configure_webhook(("https://example.com/webhook", "very_secret"))
//!     .events(
//!         [
//!             SubscriptionField::Messages,
//!             SubscriptionField::MessageTemplateStatusUpdate,
//!         ]
//!     .into(),
//!     )
//!     .await?;
//! # Ok(()) }
//! ```
//!
//! ---
//!
//! ### Start a Webhook Server and Echo Messages
//! ```rust,no_run
//! use whatsapp_business_rs::{
//!     client::Client,
//!     server::{Server, Handler, EventContext, IncomingMessage},
//!     app::SubscriptionField,
//!     Auth,
//! };
//! use std::error::Error;
//!
//! // Define a handler struct for incoming webhook events
//! #[derive(Debug)]
//! struct EchoServerHandler {
//!     client: Client, // Store a client to send replies
//! }
//!
//! impl EchoServerHandler {
//!     fn new(client: Client) -> Self {
//!         Self { client }
//!     }
//! }
//!
//! impl Handler for EchoServerHandler {
//!     // This method handles incoming messages
//!     async fn handle_message(
//!         &self,
//!         _ctx: EventContext,
//!         message: IncomingMessage,
//!     ) {
//!         println!("Received message: {:#?}", message);
//!         let message = message.into_inner();
//!         // Echo the received message content back to the sender
//!         if let Err(e) = self.client
//!             .message(message.recipient) // The recipient for the echo is the original sender's ID
//!             .send(message.sender, message.content) // Send the same content back
//!             .await
//!         {
//!             eprintln!("Failed to send echo message: {}", e);
//!         }
//!     }
//! }
//!
//! # async fn start_webhook_server_example() -> Result<(), Box<dyn Error>> {
//! // Initialize a client with your app token (or system user token)
//! // For app actions, it's recommended to use `Auth::secret`
//! let client = Client::new(Auth::secret(("YOUR_APP_ID", "YOUR_APP_SECRET"))).await?;
//!
//! // Your public webhook URL (e.g., from ngrok) and verify token
//! let webhook_url = "https://your-ngrok-url.ngrok-free.app/webhook"; // Replace with your actual URL
//! let verify_token = "my_super_secret_verify_token";
//!
//! // Configure the webhook subscription with Meta
//! let pending_configure = client
//!     .app("YOUR_APP_ID") // Replace with your app ID
//!     .configure_webhook((webhook_url, verify_token))
//!     .events([SubscriptionField::Messages].into()); // Subscribe to message events
//!
//! // Create an instance of your webhook handler
//! let handler = EchoServerHandler::new(client);
//!
//! // Build and start the webhook server
//! # #[cfg(not(feature = "incoming_message_ext"))]
//! Server::builder()
//!     .endpoint("127.0.0.1:8080".parse().unwrap()) // The local address where your server will listen
//!     .verify_payload("YOUR_APP_SECRET") // Use your app secret for payload verification
//!     .build()
//!     .serve(handler)
//!     .configure_webhook(verify_token, pending_configure) // Register webhook with Meta through the server
//!     .await?;
//!
//! println!("Webhook server started and configured!");
//! # Ok(())
//! # }
//! ```
//!
//! ---
//!
//! ### Listing Catalogs
//! ```rust,no_run
//! use whatsapp_business_rs::{client::Client, Waba, waba::CatalogMetadataField};
//! use futures::TryStreamExt as _;
//!
//! # async fn list_catalogs_example() -> Result<(), Box<dyn std::error::Error>> {
//! let business = Waba::new("YOUR_WABA_ID");
//! let client = Client::new("YOUR_ACCESS_TOKEN").await?;
//!
//! let mut catalogs = client
//!     .waba(business)
//!     .list_catalogs()
//!     .metadata([CatalogMetadataField::Name, CatalogMetadataField::Vertical].into())
//!     .into_stream();
//!
//! while let Some(catalog) = catalogs.try_next().await? {
//!     println!("{:?}", catalog);
//! }
//! # Ok(()) }
//! ```
//!
//! ---
//!
//! ### Creating a Product
//! ```rust,no_run
//! use whatsapp_business_rs::catalog::{ProductData, Price};
//! use whatsapp_business_rs::Client;
//!
//! # async fn create_product_example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::new("YOUR_ACCESS_TOKEN").await?;
//! let catalog_manager = client.catalog("YOUR_CATALOG_ID");
//!
//! let product = ProductData::default()
//!     .name("Rust Programming Book")
//!     .description("Learn Rust with this comprehensive guide")
//!     .price(Price(39.99, "USD".into()))
//!     .currency("USD")
//!     .image_url("https://example.com/book.jpg")
//!     .build("rust-book-001");
//!
//! let created = catalog_manager.create_product(product).await.unwrap();
//! println!("Created product ID: {}", created.product.product_id());
//! # Ok(()) }
//! ```
//!
//! ---

#[macro_use]
mod rest;
#[cfg(feature = "batch")]
#[macro_use]
pub mod batch;
pub mod app;
pub mod catalog;
pub mod client;
pub mod error;
pub mod message;
#[cfg(feature = "server")]
pub mod server;
pub mod waba;

macro_rules! identity {
    // Generate identity functions with documentation
    ($($variant:ident)*) => {
        paste::paste! {
            impl IdentityRef {
                $(
                    #[doc = "Create a `" $variant "` identity"]
                    pub fn [<$variant:snake>](id: impl Into<String>) -> Self {
                        Self {
                            phone_id: id.into(),
                            identity_type: IdentityType::$variant,
                        }
                    }
                )*
            }

            impl Identity {
                $(
                    #[doc = "Create a `" $variant "` identity"]
                    pub fn [<$variant:snake>](id: impl Into<String>) -> Self {
                        Self {
                            phone_id: id.into(),
                            identity_type: IdentityType::$variant,
                            metadata: IdentityMetadata::default()
                        }
                    }
                )*
            }
        }
    }
}

/// A reference to a WhatsApp identity, used for addressing messages.
///
/// This struct represents either an individual user or a business account within
/// the WhatsApp ecosystem. It's primarily used for specifying senders and recipients
/// in message operations, acting as a lightweight identifier without full metadata.
///
/// You can construct an `IdentityRef` easily using the convenience helpers:
/// - [`IdentityRef::user`] for individual users.
/// - [`IdentityRef::business`] for WhatsApp Business Accounts (WBAs).
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
#[repr(C)]
pub struct IdentityRef {
    /// Identity type (user, business)
    #[serde(rename(serialize = "recipient_type"), skip_deserializing)]
    identity_type: IdentityType,

    /// WhatsApp ID (e.g., phone number ID)
    #[serde(rename(serialize = "to", deserialize = "from"))]
    pub(crate) phone_id: String,
}

impl IdentityRef {
    /// Returns a reference to the WhatsApp ID associated with this identity.
    ///
    /// This ID can be a phone number for a user or a phone number ID for a business.
    ///
    /// # Returns
    /// A `&str` representing the `phone_id`.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::IdentityRef;
    ///
    /// let user_id = IdentityRef::user("+1234567890");
    /// assert_eq!(user_id.phone_id(), "+1234567890");
    /// ```
    pub fn phone_id(&self) -> &str {
        &self.phone_id
    }

    /// Returns the type of this WhatsApp identity.
    ///
    /// This indicates whether the identity represents an individual `User` or a `Business` account.
    ///
    /// # Returns
    /// An `IdentityType` enum value.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::{IdentityRef, IdentityType};
    ///
    /// let business_id = IdentityRef::business("1234567890");
    /// assert_eq!(business_id.identity_type(), IdentityType::Business);
    /// ```
    pub fn identity_type(&self) -> IdentityType {
        self.identity_type
    }
}

/// Defines the type of participant in a WhatsApp conversation.
///
/// This enum is used to distinguish between an individual WhatsApp user
/// and a WhatsApp Business Account (WBA).
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IdentityType {
    /// Represents an **individual WhatsApp user**.
    #[serde(rename(serialize = "individual"))]
    User,

    /// Represents a **WhatsApp Business**.
    #[serde(rename(serialize = "business"))]
    #[default]
    Business,
}

identity! {
    User
    Business
}

/// Represents a **full WhatsApp identity** with additional metadata.
///
/// Unlike [`IdentityRef`] which is a lightweight reference for addressing,
/// `Identity` includes richer details about the participant, such as
/// their display name and phone number (as shown in the UI).
///
/// This struct is typically used for **received messages** where WhatsApp
/// provides comprehensive details about the sender and recipient.
///
/// # Fields
/// - `identity_type`: The type of participant (User or Business).
/// - `phone_id`: The WhatsApp ID (phone number or phone number ID) of the participant.
/// - `metadata`: Additional information about the identity, such as name and
///   display phone number.
#[derive(PartialEq, Eq, Clone, Debug)]
#[repr(C)]
pub struct Identity {
    /// The type of this identity, indicating whether it's a `User` or `Business`.
    pub identity_type: IdentityType,

    /// The unique WhatsApp ID associated with this identity.
    /// This is typically the phone number for users or the phone number ID for businesses.
    pub phone_id: String,

    /// Optional metadata associated with this identity, such as display name
    /// or formatted phone number.
    pub metadata: IdentityMetadata,
}

derive! {
    /// Additional information associated with a WhatsApp [`Identity`].
    ///
    /// This struct holds optional, human-readable details about an identity,
    /// supplementing the core `phone_id`.
    #[derive(#Fields, PartialEq, Eq, Clone, Debug, Default)]
    #[repr(C)] // already though
    #[non_exhaustive]
    pub struct IdentityMetadata {
        /// An optional display name for the identity, as configured in WhatsApp.
        /// This might be a contact's name or a business's registered name.
        pub name: Option<String>,

        /// An optional formatted phone number, as it might be displayed in the UI.
        /// This can differ from `phone_id` if `phone_id` is a phone number ID
        /// or if it's a raw E.164 number without formatting.
        pub phone_number: Option<String>,
    }
}

impl Identity {
    /// Converts this full `Identity` into a lightweight `IdentityRef`.
    ///
    /// This method is useful when you have a complete `Identity` object
    /// (e.g., from an incoming message) but only need the essential
    /// addressing information (`identity_type` and `phone_id`) to
    /// construct an outgoing message or interact with other API methods
    /// that accept `IdentityRef`.
    ///
    /// # Returns
    /// An `IdentityRef` containing the `phone_id` and `identity_type`
    /// from this `Identity`.
    pub fn as_ref(&self) -> IdentityRef {
        IdentityRef {
            phone_id: self.phone_id.clone(),
            identity_type: self.identity_type,
        }
    }
}

derive! {
    /// A Whatsapp Business Account (WABA).
    ///
    /// This struct represents the top-level WhatsApp Business Account, identified by its
    /// `account_id`. It's essentially the container for all your WhatsApp Business assets,
    /// including phone numbers, catalogs, and subscribed apps.
    ///
    /// You typically interact with a `Waba` through the `WabaManager` which provides
    /// methods to manage the account's resources.
    #[derive(#NodeImpl, PartialEq, Eq, Clone, Debug)]
    pub struct Waba {
        pub(crate) account_id: String,
    }
}

/// Represents a specific WhatsApp Business profile, associating a WABA with a particular
/// phone number's identity.
///
/// While `Waba` is the overarching business account, `Business` narrows that down to a
/// specific operational identity, tied to a phone number. Think of `Waba` as your
/// entire business entity on WhatsApp, and `Business` as one of its distinct storefronts
/// or communication channels, specifically managed through a registered phone number.
///
/// This struct is useful for operations that need to be scoped to a particular
/// phone number within a WABA.
///
/// # Fields
/// - `waba`: The [`Waba`] (WhatsApp Business Account) this business identity belongs to.
/// - `identity`: The [`IdentityRef`] of this specific business profile, which includes
///   the phone number ID.
///
/// [`Waba`]: crate::Waba
/// [`IdentityRef`]: crate::IdentityRef
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Business {
    pub waba: Waba,
    pub identity: IdentityRef,
}

impl Business {
    /// Creates a new `Business` instance.
    ///
    /// This constructor associates a [`Waba`] with a specific phone number identity,
    /// effectively defining a distinct business presence within that WABA.
    ///
    /// # Parameters
    /// - `account`: The [`Waba`] (WhatsApp Business Account) to which this business
    ///   identity belongs. This can be a `Waba` struct or anything that converts into it.
    /// - `phone_id`: The unique identifier for the phone number associated with this
    ///   business identity. This is typically obtained after successfully registering
    ///   and verifying a phone number.
    ///
    /// # Returns
    /// A new `Business` instance.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::{Waba, Business, IdentityRef};
    ///
    /// let my_waba = Waba::new("YOUR_WABA_ID");
    /// let my_phone_id = "YOUR_REGISTERED_PHONE_NUMBER_ID";
    ///
    /// let my_business_profile = Business::new(my_waba, my_phone_id);
    ///
    /// println!(
    ///     "Business profile for WABA: {} and Phone ID: {}",
    ///     my_business_profile.waba.account_id(),
    ///     my_business_profile.identity.phone_id()
    /// );
    /// ```
    ///
    /// [`Waba`]: crate::Waba
    /// [`IdentityRef`]: crate::IdentityRef
    pub fn new(account: impl Into<Waba>, phone_id: impl Into<String>) -> Self {
        Self {
            waba: account.into(),
            identity: IdentityRef::business(phone_id),
        }
    }
}

impl Deref for Business {
    type Target = Waba;

    fn deref(&self) -> &Self::Target {
        &self.waba
    }
}

derive! {
    /// A reference to a catalog
    #[derive(#NodeImpl, PartialEq, Eq, Clone, Debug)]
    #[repr(C)]
    pub struct CatalogRef {
        pub(crate) id: String,
    }
}

derive! {
    /// Represents a Meta App
    #[derive(#NodeImpl, PartialEq, Eq, Clone, Debug)]
    pub struct App {
        pub(crate) id: String,
    }
}

/// A fluent interface for selecting fields for Meta's Graph API.
///
/// This type is used internally by library methods like [`CatalogManager::list_products`] to let
/// you customize the fields included in the response payload.
///
/// `Fields` builds a collection of desired fields.
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::catalog::ProductDataField;
/// use whatsapp_business_rs::Fields;
///
/// # async fn example(catalog: whatsapp_business_rs::catalog::CatalogManager<'_>) {
/// catalog.list_products().metadata(
///     Fields::new()
///         .with(ProductDataField::Price)
///         .with(ProductDataField::Availability),
/// );
/// # }
/// ```
///
/// # Notes
/// - The exact set of available fields depends on the context (e.g. products, subscriptions).
/// - This struct is typically used as an argument to other builder methods (e.g., `.events()` for webhooks).
///
/// [`CatalogManager::list_products`]: crate::catalog::CatalogManager::list_products
#[derive(Clone, Debug)]
pub struct Fields<Field> {
    // still don't belong here
    pub(crate) mandatory: &'static [&'static str],
    // meta should de-dup itself
    pub(crate) fields: Vec<Field>, // pub(crate) fields: HashSet<Field>,
}

impl<F> Default for Fields<F> {
    fn default() -> Self {
        Self {
            fields: Vec::default(),
            mandatory: &[],
        }
    }
}

impl<Field> Fields<Field>
where
    Field: FieldsTrait,
{
    /// Creates a new `Fields` instance
    pub fn new() -> Self {
        Fields::default()
    }

    /// Adds a single field to the request.
    ///
    /// # Parameters
    /// - `field`: The field to include in the request.
    ///
    /// # Returns
    /// The updated `Fields` instance.
    pub fn with(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }

    /// Includes all available fields for this type.
    ///
    /// This is a convenience method for when you want the full response without
    /// specifying each field individually.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::Fields;
    ///
    /// # use whatsapp_business_rs::catalog::CatalogManager;
    /// # async fn example(catalog: CatalogManager<'_>) {
    /// catalog
    ///     .list_products()
    ///     .metadata(Fields::all())
    ///     .into_stream();
    /// # }
    /// ```
    #[inline]
    pub fn all() -> Self {
        let mut s = Self::new();
        s.extend(Field::ALL);
        s
    }

    pub(crate) fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Combine the mandatory fields with the optional fields from the HashSet.
        let fields_query = self
            .mandatory
            .iter()
            .copied() // Convert iterator of &&str to &str
            .chain(self.fields.iter().map(|f| f.as_snake_case()))
            .collect::<Vec<_>>()
            .join(",");

        // Serialize the final string.
        serializer.serialize_str(&fields_query)
    }

    pub(crate) fn mandatory(mut self, mandatory: &'static [&'static str]) -> Self {
        self.mandatory = mandatory;
        self
    }
}

impl<Field> FromIterator<Field> for Fields<Field>
where
    Field: FieldsTrait,
{
    #[inline]
    fn from_iter<T: IntoIterator<Item = Field>>(iter: T) -> Self {
        let fields = Vec::from_iter(iter);
        Self {
            fields,
            mandatory: &[],
        }
    }
}

impl<Field, const N: usize> From<[Field; N]> for Fields<Field>
where
    Field: FieldsTrait,
{
    fn from(arr: [Field; N]) -> Self {
        Self::from_iter(arr)
    }
}

impl<Field> Extend<Field> for Fields<Field>
where
    Field: FieldsTrait,
{
    #[inline]
    fn extend<T: IntoIterator<Item = Field>>(&mut self, iter: T) {
        self.fields.extend(iter)
    }
}

impl<'a, Field> Extend<&'a Field> for Fields<Field>
where
    Field: FieldsTrait + 'a,
{
    #[inline]
    fn extend<T: IntoIterator<Item = &'a Field>>(&mut self, iter: T) {
        self.fields.extend(iter)
    }
}

/// A composable update request builder for Graph-style resources.
///
/// This struct allows you to fluently configure fields for an update operation
/// (such as updating a product), then execute the request by `.await`ing the
/// builder.
///
/// You typically don’t construct this manually. Instead, it’s returned by
/// helper methods like [`CatalogManager::update_product()`] or
/// [`CatalogManager::update_product_by_id()`].
///
/// # Usage
///
/// ```rust,no_run
/// # use whatsapp_business_rs::catalog::{CatalogManager, ProductRef, Price};
/// # async fn example_update_builder(catalog: CatalogManager<'_>, product_ref: ProductRef)
/// # -> Result<(), Box<dyn std::error::Error>> {
/// let updated_product_info = catalog.update_product(product_ref)
///      .name("Updated Product Name")
///      .price(Price(4999.0, "USD".into()))
///      .currency("USD")
///      .await?; // Await to send the update
/// # Ok(())}
/// ```
///
/// # Type Parameters
/// - `T`: The type holding the fields being updated (e.g., `ProductData`).
/// - `U`: The type returned as a response from the API after a successful update.
///
/// # Notes
/// - Only the fields you explicitly set will be sent in the request body.
/// - The update is not performed until the builder is `.await`ed.
///
/// [`CatalogManager::update_product()`]: crate::catalog::CatalogManager::update_product
/// [`CatalogManager::update_product_by_id()`]: crate::catalog::CatalogManager::update_product_by_id
#[must_use = "Update does nothing unless you `.await` or `.execute().await` it"]
pub struct Update<'a, T, U = ()> {
    // rid this struct
    pub(crate) request: PendingRequest<'static, JsonObjectPayload<T>>,
    response: PhantomData<U>,
    _marker: PhantomData<&'a ()>,
}

impl<T, U> Update<'_, T, U>
where
    T: Default,
{
    /// Creates a new update builder.
    ///
    /// # Parameters
    /// - `request`: The request builder to use.
    ///
    /// # Returns
    /// An `Update<T, U>` instance.
    pub(crate) fn new(request: PendingRequest<'static, JsonObjectPayload<T>>) -> Self {
        Self {
            request,
            response: PhantomData,
            _marker: PhantomData,
        }
    }
}

impl<T, U> Update<'_, T, U> {
    /// Specifies the authentication token to use for this update request.
    ///
    /// This is particularly useful for applications managing multiple entities (e.g., catalogs, products)
    /// where different API tokens might be required for various update operations.
    /// It allows you to reuse a manager instance (e.g., `CatalogManager`) and apply the
    /// appropriate token for each specific update without re-initializing the `Client`.
    ///
    /// If not called, the request will use the authentication configured with the `Client`
    /// that initiated this update.
    ///
    /// # Parameters
    /// - `auth`: [`Auth`] token to use for this specific request.
    ///
    /// [`Auth`]: crate::client::Auth
    pub fn with_auth<'a>(mut self, auth: impl ToValue<'a, Auth>) -> Self {
        self.request = self.request.auth(auth.to_value().into_owned());
        self
    }
}

impl<T, U> Update<'_, T, U>
where
    T: Serialize + Send,
{
    pub(crate) fn request(self) -> PendingRequest<'static, JsonObjectPayload<T>> {
        self.request
    }
}

IntoFuture! {
    impl<T, U> Update<'_, T, U>
    [
    where
        T: Serialize + Send,
        U: FromResponseOwned,
    ]
    {
        /// Sends the update request with the configured fields.
        ///
        /// This method serializes the inner update object and performs the actual
        /// HTTP request. It returns the deserialized response of type `U`.
        /// Because `Update` implements `IntoFuture`, you can also simply `.await`
        /// the `Update` instance directly, which will call this method internally.
        ///
        /// # Returns
        /// A `Result` containing either the deserialized response of type `U`, or an [`Error`].
        ///
        /// # Example
        /// ```rust,no_run
        /// # use whatsapp_business_rs::catalog::{CatalogManager, ProductRef};
        /// # use whatsapp_business_rs::client::Auth;
        /// # async fn example_execute_update(catalog: CatalogManager<'_>,
        /// # product_ref: ProductRef) -> Result<(), Box<dyn std::error::Error>> {
        /// // Preferred: Await directly
        /// catalog.update_product(&product_ref)
        ///      .name("New Name")
        ///      .availability("in stock")
        ///      .with_auth("auth_token") // Use with_auth if a specific token is needed
        ///      .await?;
        ///
        /// // Alternative: Call .execute().await
        /// catalog.update_product(&product_ref)
        ///      .name("Another Name")
        ///      .execute()
        ///      .await?;
        /// # Ok(())}
        /// ```
        /// [`Error`]: crate::error::Error
        pub fn execute(self) ->  impl Future<Output = Result<U, Error>> + 'static {
            execute_request(self.request())
        }
    }
}

#[derive(Clone)]
#[cfg(feature = "batch")]
pub struct UpdateResponseReference<U> {
    _priv: (),
    _marker: PhantomData<U>,
}

#[cfg(feature = "batch")]
impl<T, U> crate::batch::IntoResponseReference for Update<'_, T, U> {
    type ResponseReference = UpdateResponseReference<U>;

    fn into_response_reference(_: Cow<'static, str>) -> Self::ResponseReference {
        Self::ResponseReference {
            _priv: (),
            _marker: PhantomData,
        }
    }
}

/// A type indicating that an optional flow's step had been skipped.
///
/// See [`OnboardingFlow::share_credit_line`].
///
/// [`OnboardingFlow::share_credit_line`]: crate::app::OnboardingFlow::share_credit_line
pub struct FlowStepSkipped;

/// Represents an **error object returned directly by Meta's Graph API** in its responses
/// or webhook payload.
///
/// This struct captures detailed information about an error encountered during an API call
/// to Meta's platform or webhook analysis. It is distinct from the crate's own `Error` enum,
/// as `MetaError` specifically describes issues reported by the Meta API itself.
///
/// # Fields
/// - `code`: A numerical error code provided by Meta, indicating the type of error.
/// - `title`: An optional, concise title or summary of the error.
/// - `message`: An optional, more descriptive message explaining the error.
/// - `support`: An optional URL pointing to Meta's documentation or support
///   pages related to the error.
/// - `error_metadata`: Additional metadata about the error, often used for specific
///   error categories or context.
///
/// # Example (from Meta API response)
/// ```json
/// {
///   "error": {
///     "message": "(#100) Parameter missing",
///     "type": "OAuthException",
///     "code": 100,
///     "fbtrace_id": "A4K...",
///     "error_data": {
///       "messaging_product": "whatsapp",
///       "details": "The recipient phone number is not valid."
///     }
///   }
/// }
/// ```
#[derive(thiserror::Error, Serialize, Deserialize, PartialEq, Clone, Debug, Default)]
#[non_exhaustive]
pub struct MetaError {
    pub code: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fbtrace_id: Option<String>,
    #[serde(rename = "href", default, skip_serializing_if = "Option::is_none")]
    pub support: Option<String>,
    #[serde(
        rename = "error_data",
        default,
        skip_serializing_if = "MetaErrorMetadata::is_none"
    )]
    pub error_metadata: MetaErrorMetadata,
}

impl fmt::Display for MetaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(code: {})", self.code)?;

        if let Some(title) = &self.title {
            write!(f, " - {}", title)?;
        }

        if let Some(r#type) = &self.r#type {
            write!(f, " (type: {})", r#type)?;
        }

        if let Some(message) = &self.message {
            write!(f, ": {}", message)?;
        }

        if let Some(details) = &self.error_metadata.details {
            writeln!(f, "\n  Details: {}", details)?;
        }

        if let Some(support) = &self.support {
            writeln!(f, "\n  More info: {}", support)?;
        }

        if let Some(id) = &self.fbtrace_id {
            writeln!(f, "\n  Trace ID: {}", id)?;
        }

        Ok(())
    }
}

#[derive(Serialize, Deserialize, PartialEq, Clone, Debug, Default)]
#[non_exhaustive]
pub struct MetaErrorMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl MetaErrorMetadata {
    fn is_none(&self) -> bool {
        self.details.is_none()
    }
}

/// Represents a timestamp returned by the WhatsApp Business API.
///
/// Meta typically uses UNIX timestamps (`seconds since epoch`) across most
/// of the Graph API. However, this is not guaranteed to remain consistent in the future.
///
/// # Note
/// - Currently, all observed timestamps are UNIX-based.
/// - This may change without warning.
/// - Always assume the value is a raw `i64` unless explicitly documented otherwise.
#[derive(PartialEq, Clone, Copy, Debug)]
pub struct Timestamp {
    pub(crate) inner: i64,
}

impl Timestamp {
    /// Returns the raw timestamp in seconds.
    ///
    /// This is usually a UNIX timestamp (seconds since epoch),
    /// but could be relative in rare cases (e.g., token TTL).
    pub fn seconds(&self) -> i64 {
        self.inner
    }
}

/// Trait used for identity/value conversion with Cow optimization
pub(crate) trait ToValue<'a, Value>: Send + Sync
where
    Value: Clone + Send + Sync,
{
    fn to_value(self) -> Cow<'a, Value>;
}

impl<'a, Value> ToValue<'a, Value> for Cow<'a, Value>
where
    Value: Clone + Send + Sync,
{
    #[inline]
    fn to_value(self) -> Cow<'a, Value> {
        self
    }
}

macro_rules! impl_to_value_strings {
    ($ty:ty) => {
        impl<'_life, 'a, Value> ToValue<'a, Value> for $ty
        where
            Value: From<$ty> + Clone + Send + Sync + 'a,
        {
            #[inline]
            fn to_value(self) -> Cow<'a, Value> {
                Cow::Owned(Value::from(self))
            }
        }
    };
}

impl_common_strings! {
    impl_to_value_strings
}

impl<T: Into<String>> From<T> for IdentityRef {
    /// Implements conversion from a `String` into an `IdentityRef`.
    ///
    /// This allows for convenient creation of `IdentityRef` instances directly
    /// from string representations of WhatsApp IDs. The conversion logic attempts
    /// to infer the `IdentityType` based on whether the string starts with a "+":
    ///
    /// - If the `value` starts with `+` (indicating an E.164 formatted phone number),
    ///   it's treated as an [`IdentityType::User`].
    /// - Otherwise (e.g., a numerical ID), it's treated as an [`IdentityType::Business`].
    ///
    /// # Arguments
    /// - `value`: The `String` representing the WhatsApp ID (phone number or phone number ID).
    ///
    /// # Returns
    /// An `IdentityRef` with the inferred `identity_type` and the provided `phone_id`.
    ///
    /// # Examples
    /// ```rust
    /// use whatsapp_business_rs::{IdentityRef, IdentityType};
    ///
    /// let user_ref: IdentityRef = "+1234567890".to_string().into();
    /// assert_eq!(user_ref.phone_id(), "+1234567890");
    /// assert_eq!(user_ref.identity_type(), IdentityType::User);
    ///
    /// let business_ref: IdentityRef = "123456789012345".to_string().into();
    /// assert_eq!(business_ref.phone_id(), "123456789012345");
    /// assert_eq!(business_ref.identity_type(), IdentityType::Business);
    /// ```
    #[inline]
    fn from(value: T) -> Self {
        let value = value.into();
        if value.starts_with("+") || value.len() < 15 {
            IdentityRef::user(value)
        } else {
            IdentityRef::business(value)
        }
    }
}

to_value! {
    IdentityRef Business
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.identity_type {
            IdentityType::User => write!(f, "👤 {}", self.phone_id),
            IdentityType::Business => write!(f, "🏢 {}", self.phone_id),
        }
    }
}

impl ToValue<'_, IdentityRef> for Identity {
    #[inline]
    fn to_value(self) -> Cow<'static, IdentityRef> {
        Cow::Owned(IdentityRef {
            phone_id: self.phone_id,
            identity_type: self.identity_type,
        })
    }
}

impl<'a> ToValue<'a, IdentityRef> for &'a Identity {
    #[inline]
    fn to_value(self) -> Cow<'a, IdentityRef> {
        // SAFETY:
        // - `Identity` is #[repr(C)] so fields are laid out in declared order
        // - `IdentityRef` is #[repr(C)] and only includes a prefix of `Identity` fields.
        // - So it's safe to transmute a `&Identity` into a `&IdentityRef`.
        let view = unsafe { view_ref(self) };
        Cow::Borrowed(view)
    }
}

impl ToValue<'_, Waba> for Business {
    #[inline]
    fn to_value(self) -> Cow<'static, Waba> {
        Cow::Owned(self.waba)
    }
}

impl<'a> ToValue<'a, Waba> for &'a Business {
    #[inline]
    fn to_value(self) -> Cow<'a, Waba> {
        Cow::Borrowed(&self.waba)
    }
}

// TODO: Reduce
pub use client::{Auth, Client};
use client::{JsonObjectPayload, PendingRequest};
pub use error::Error;
pub use message::{Draft, Message};
pub use server::{Handler as WebhookHandler, Server};

use rest::{FieldsTrait, FromResponseOwned, execute_request, macros::view_ref};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, fmt, marker::PhantomData, ops::Deref};

#[cfg(test)]
mod test {
    use super::*;
    // CD test
    #[test]
    fn ub() {
        let identity = Identity::user("+1399444994");
        let view: &IdentityRef = unsafe { view_ref(&identity) };

        assert_eq!(view.identity_type, IdentityType::User);
        assert_eq!(view.phone_id, "+1399444994");

        let view_clone = view.clone();

        assert_eq!(view_clone, *view);
    }
}
