//! WhatsApp Message Types and Builders
//!
//! This module defines the building blocks for composing, sending, and receiving messages
//! via the WhatsApp Business API. It includes representations of various message content types
//! such as text, media, location, reactions, interactive messages, and commerce-related messages like orders.
//!
//! ## Key Types & Features
//!
//! - [`Content`]: A polymorphic enum representing any kind of WhatsApp message payload,
//!   used for both incoming and outgoing messages. It also provides a method to
//!   [`download_media()`](Content::download_media) if the content is media.
//! - [`Draft`]: A fluent, builder-style struct for constructing and preparing outbound messages
//!   before they are sent. Use its associated functions like [`Draft::text()`], [`Draft::media()`],
//!   or [`Draft::interactive()`] to start building.
//! - [`Media`]: Represents image, audio, video, sticker, or document content,
//!   with capabilities to load from file paths or raw bytes.
//! - [`Text`]: A simple string message type, supporting optional link previews.
//! - [`Reaction`]: An emoji reaction to a specific message, identifiable by a message reference.
//! - [`Location`]: Geographical coordinates for sharing places.
//! - [`InteractiveMessage`]: A powerful enum for handling both sending interactive messages (buttons, lists, products)
//!   and parsing incoming user interactions (button clicks, list selections).
//! - [`Order`]: E-commerce-specific message for orders from a catalog.
//! - [`MediaSource`]: An internal abstraction for media content source, supporting raw bytes, public URLs,
//!   or pre-uploaded WhatsApp media IDs.
//!
//! ## Examples
//!
//! ---
//! ### Send a Simple Text Message
//! ```rust,no_run
//! use whatsapp_business_rs::message::Draft;
//! use whatsapp_business_rs::Client;
//!
//! # async fn send_text(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
//! let draft = Draft::text("Hello from Rust! How can I help you today?");
//! client.message("BUSINESS_NO_ID") // Replace with your WhatsApp Number ID
//!      .send("+16012345678", draft) // Replace with recipient's phone number
//!      .await?;
//! println!("Text message sent!");
//! # Ok(())
//! # }
//! ```
//!
//! ---
//! ### Reply to a Message with an Image and Caption
//! ```rust,no_run
//! use whatsapp_business_rs::message::{Draft, Media, Message};
//! use whatsapp_business_rs::Client;
//!
//! # async fn reply_with_image(client: &Client, original_message: &Message) -> Result<(), Box<dyn std::error::Error>> {
//! // Load an image from a local path
//! let media = Media::from_path("path/to/your/image.png").await?;
//!
//! // Create a draft, attach the media, add a caption, and set it as a reply
//! let draft = Draft::media(media)
//!      .with_caption("Hello back at you! Here's something interesting.")
//!      .reply_to(original_message);
//!
//! client.message("BUSINESS_NO_ID") // Replace with your WhatsApp Number ID
//!      .send(&original_message.sender, draft) // Reply to the original sender
//!      .await?;
//! println!("Image message replied!");
//! # Ok(())
//! # }
//! ```
//!
//! ---
//! ### Send an Interactive Button Message
//! ```rust,no_run
//! use whatsapp_business_rs::message::{
//!     Keyboard, Button, Draft, InteractiveAction, InteractiveMessage, Text,
//! };
//! # use whatsapp_business_rs::Client;
//!
//! # async fn send_interactive_buttons(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
//! let buttons = [
//!      Button::reply("yes_clicked", "Yes, I agree"),
//!      Button::reply("no_clicked", "No, I disagree"),
//!      Button::url("https://example.com/more-info", "Learn More"),
//! ];
//!
//! let action = Keyboard::from(buttons);
//! let interactive_msg = InteractiveMessage::new(action, "Please choose your option below:")
//!     .footer("Your feedback is important.");
//!
//! let draft = Draft::interactive(interactive_msg);
//!
//! client.message("BUSINESS_NO_ID") // Replace with your WhatsApp Number ID
//!      .send("+16012345678", draft) // Replace with recipient's phone number
//!      .await?;
//! println!("Interactive button message sent!");
//! # Ok(())
//! # }
//! ```
//!
//! ---

use crate::client::{Auth, SendMessage};
use crate::rest::client::{
    DownloadMedia as InnerDownloadMedia, DownloadMediaFromUrl, deserialize_interactive_action,
    deserialize_str, serialize_interactive_action, serialize_ordinary_text, serialize_str,
    serialize_text_text, serialize_text_text_opt,
};
use crate::rest::option_from_response_default;
use crate::{CatalogRef, MetaError, Timestamp, ToValue};
use crate::{Identity, IdentityRef, catalog::ProductRef, client::Client, error::Error};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::ops::Deref;
#[allow(unused_imports)]
use std::path::Path;
use tokio::io::AsyncWrite;

derive! {
    /// Enum representing all possible WhatsApp message payloads.
    ///
    /// This `Content` enum serves as the central content type for handling
    /// WhatsApp messages. It is used universally for both **incoming** messages
    /// (received via webhooks) and **outgoing** messages (sent through the API).
    ///
    /// Each variant in this enum wraps a specific payload structure,
    /// providing a strongly-typed representation of different message types
    /// supported by the WhatsApp Business API.
    #[derive(#ContentTraits, PartialEq, Clone, Debug)]
    pub enum Content {
        /// Represents various types of **media messages**, including:
        /// - Images (e.g., JPEG, PNG)
        /// - Audio files (e.g., MP3, OGG)
        /// - Video files (e.g., MP4, 3GP)
        /// - Stickers (WebP)
        /// - Documents (e.g., PDF, DOCX)
        ///
        /// This variant includes optional fields for a `caption` (text
        /// accompanying the media) and a `filename` (for documents).
        Media(Media),

        /// Represents a standard **text message**.
        ///
        /// This is the most common message type and contains the textual content
        /// sent or received.
        Text(Text),

        /// Represents an **emoji reaction** to a specific message.
        Reaction(Reaction),

        /// Represents a **geographic location message**.
        ///
        /// This variant contains coordinates (latitude and longitude) and
        /// optionally a name and address for a specific point of interest.
        Location(Location),

        /// Represents an **interactive message** with structured content.
        ///
        /// Interactive messages can include elements like:
        /// - Buttons (e.g., reply buttons, call-to-action buttons)
        /// - Lists (e.g., product lists, service lists)
        /// - Single product messages
        /// - Multi-product messages
        ///
        /// These messages are designed to facilitate more complex user interactions
        /// directly within the chat.
        Interactive(InteractiveContent),


        /// Represents a **product order message**.
        ///
        /// This variant is used for messages related to product orders or shopping
        /// carts, typically containing details about items purchased or being
        /// considered for purchase.
        Order(Order),

        /// Represents an **error message indicating an unsupported or unprocessable
        /// message type** received from WhatsApp.
        ///
        /// This variant is exclusively for **incoming webhook messages**. It signals
        /// that WhatsApp could not process or recognize the original message content
        /// sent by a user (or another system), typically because the message type
        /// is not currently supported by the WhatsApp Business API for webhooks.
        ///
        /// You will usually receive this `Error` content when WhatsApp's API returns
        /// an error like:
        /// ```json
        /// {
        ///   "code": 131051,
        ///   "details": "Message type is not currently supported",
        ///   "title": "Unsupported message type"
        /// }
        /// ```
        ///
        /// It is important to note that this `Error` variant is **not intended
        /// for sending** by your application. It serves as an informative
        /// inbound message indicating a processing failure on WhatsApp's side
        /// for a particular received message.
        #![serde(rename = "errors")]
        Error(ErrorContent),
    }
}

impl Default for Content {
    fn default() -> Self {
        Content::Text(Text::default())
    }
}

impl Content {
    /// Returns the body of a `Text` message, if this is one.
    ///
    /// This only returns text from a `Content::Text` variant.
    /// To get text from captions or order notes, use `text()`.
    pub fn text_body(&self) -> Option<&str> {
        match self {
            Content::Text(text) => Some(&text.body),
            _ => None,
        }
    }

    /// Returns any primary text associated with the message content.
    ///
    /// This checks, in order:
    /// 1. The body of a `Content::Text` message.
    /// 2. The caption of a `Content::Media` message.
    /// 3. The note of a `Content::Order` message.
    pub fn text(&self) -> Option<&str> {
        match self {
            Content::Text(text) => Some(&text.body),
            Content::Media(media) => media.caption.as_ref().map(|t| t.body.as_str()),
            Content::Order(order) => Some(&order.note.body),
            _ => None,
        }
    }

    /// Returns the `Media` payload, if this is a media message.
    pub fn media(&self) -> Option<&Media> {
        match self {
            Content::Media(media) => Some(media),
            _ => None,
        }
    }

    /// Returns the `Button` payload, if this is an interactive button *click*.
    ///
    /// This only returns a value for `Content::Interactive(InteractiveContent::Click)`.
    /// It does *not* return the buttons from an outgoing `InteractiveContent::Message`.
    pub fn button_click(&self) -> Option<&Button> {
        match self {
            Content::Interactive(InteractiveContent::Click(button)) => Some(button),
            _ => None,
        }
    }

    /// Download media content for this message (if it contains media)
    ///
    /// # Arguments
    /// * `client` - WhatsApp API client to use for download
    /// * `dst` - Destination.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::message::Content;
    ///
    /// # async fn example(mut content: Content, client: &whatsapp_business_rs::Client) {
    /// let mut dst = Vec::new();
    /// if let Some(media) = content.download_media(&mut dst, client) {
    ///     media.await.unwrap();
    ///     assert!(!dst.is_empty())
    /// }
    /// # }
    /// ```
    #[must_use]
    pub fn download_media<'dst, Dst>(
        &self,
        dst: &'dst mut Dst,
        client: &Client,
    ) -> Option<DownloadMedia<'dst, Dst>>
    where
        Dst: AsyncWrite + Send + Unpin,
    {
        if let Self::Media(Media { media_source, .. }) = self {
            let resolve = match media_source {
                MediaSource::Id(id) => DownloadMediaResolve::Id(client.download_media(id, dst)),
                MediaSource::Link(url) => {
                    DownloadMediaResolve::Url(client.download_media_from_url(url, dst))
                }
                MediaSource::Bytes(_) => return None,
            };
            Some(DownloadMedia {
                resolve: Box::new(resolve),
            })
        } else {
            None
        }
    }
}

/// A builder for downloading media content.
///
/// This struct is returned by [`Content::download_media()`]. It allows you to
/// specify authentication if needed, and then execute the download operation.
///
/// You typically interact with this builder by `.await`ing it directly, which
/// will trigger the download.
///
/// [`Content::download_media()`]: crate::message::Content::download_media
pub struct DownloadMedia<'dst, Dst> {
    resolve: Box<DownloadMediaResolve<'dst, Dst>>,
}

enum DownloadMediaResolve<'dst, Dst> {
    Id(InnerDownloadMedia<'dst, Dst>),
    Url(DownloadMediaFromUrl<'dst, Dst>),
}

impl<Dst> DownloadMedia<'_, Dst> {
    /// Specifies the authentication token to use for this media download request.
    ///
    /// This is particularly useful for applications managing multiple WhatsApp Business Accounts (WBAs)
    /// where different API tokens might be required for various media operations.
    /// It allows you to reuse a `Client` and apply the appropriate token for each specific download.
    ///
    /// If not called, the request will use the authentication configured with the `Client`
    /// that initiated this download.
    ///
    /// # Parameters
    /// - `auth`: [`Auth`] token to use for this specific request.
    ///
    /// # Returns
    /// The updated `DownloadMedia` builder instance.
    ///
    /// [`Auth`]: crate::client::Auth
    #[inline]
    pub fn with_auth<'a>(mut self, auth: impl ToValue<'a, Auth>) -> Self {
        if let DownloadMediaResolve::Id(download_media) = *self.resolve {
            *self.resolve = DownloadMediaResolve::Id(download_media.with_auth(&auth.to_value()));
            self
        } else {
            self
        }
    }
}

IntoFuture! {
    impl<'dst, Dst> DownloadMedia<'dst, Dst>
    [where
        Dst: AsyncWrite + Send + Unpin,]
    {
        /// Executes the media download request.
        ///
        /// This method performs the HTTP request to download the media content.
        /// Upon successful download, the `MediaSource` associated with the `Media` message
        /// will be updated to `MediaSource::Bytes`, containing the downloaded content.
        ///
        /// Because `DownloadMedia` implements `IntoFuture`, you can also simply `.await`
        /// the `DownloadMedia` instance directly, which will call this method internally.
        ///
        /// # Returns
        /// A `Result` indicating success or an [`Error`] if the download fails.
        ///
        /// # Example
        /// ```rust,no_run
        /// use whatsapp_business_rs::message::{Content, Media, MediaSource};
        /// use whatsapp_business_rs::Client;
        ///
        /// # async fn example_execute_download(mut content: Content, client: &Client) -> Result<(), Box<dyn std::error::Error>> {
        /// # let mut dst = Vec::new();
        /// if let Some(download_op) = content.download_media(&mut dst, client) {
        ///     download_op.execute().await?; // Explicitly call execute
        ///     println!("Media downloaded via execute()!");
        /// }
        /// # Ok(())
        /// # }
        /// ```
        /// [`Error`]: crate::error::Error
        #[inline]
        pub fn execute(self) -> impl Future<Output = Result<(), Error>> + 'dst {
            async move {
                match *self.resolve {
                    DownloadMediaResolve::Id(download_media) => {
                        download_media.execute().await?;
                    }
                    DownloadMediaResolve::Url(download_media_url) => {
                        download_media_url.execute().await?;
                    }
                };
                Ok(())
            }
        }
    }
}

/// Media message content.
///
/// Used for images, audio, video, documents, and stickers.
/// Attach captions or filenames where applicable.
///
/// The media can come from raw bytes, a public URL, or a WhatsApp media ID.
#[derive(Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct Media {
    /// Media content source (bytes, URL, or WhatsApp ID)
    #[serde(flatten)]
    pub media_source: MediaSource,
    /// Type and format of media
    #[serde(rename = "mime_type")]
    pub media_type: MediaType,
    /// Optional description text
    #[serde(default)]
    pub caption: Option<Text>,
    /// Suggested filename for recipient
    #[serde(default)]
    pub filename: Option<String>,
}

impl Media {
    /// Creates a new `Media` instance.
    ///
    /// This constructor allows you to directly specify the media's source, its MIME type,
    /// and optional caption and filename.
    ///
    /// # Arguments
    ///
    /// * `media_source` - The source of the media content (bytes, URL, or WhatsApp ID).
    ///   Accepts anything convertible into a [`MediaSource`] enum variant.
    /// * `media_type` - The specific MIME type of the media (e.g., `image/jpeg`).
    ///
    /// # Returns
    ///
    /// A new `Media` instance.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::{Media, MediaSource, MediaType, Text};
    ///
    /// let media_from_url = Media::new("https://example.com/my_photo.jpg", "image/jpeg".parse().unwrap())
    ///     .caption("A beautiful sunset.");
    ///
    /// let document_media = Media::new(
    ///     "https://example.com/document.pdf",
    ///     "application/pdf".parse().unwrap(),
    /// ).filename("MyReport.pdf");
    /// ```
    #[inline]
    pub fn new(media_source: impl Into<MediaSource>, media_type: MediaType) -> Self {
        Self {
            media_source: media_source.into(),
            media_type,
            caption: None,
            filename: None,
        }
    }

    /// Creates a new `Media` instance specifically for a PDF document.
    ///
    /// This is a convenience constructor that sets the `media_type` to `application/pdf` automatically.
    ///
    /// # Arguments
    ///
    /// * `media_source` - The source of the PDF document (bytes, URL, or WhatsApp ID).
    ///   Accepts anything convertible into a [`MediaSource`] enum variant.
    /// * `filename` - The desired filename for the document.
    ///
    /// # Returns
    ///
    /// A new `Media` instance configured for a PDF document.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Media;
    ///
    /// let pdf_document = Media::pdf("https://example.com/report.pdf", "MyReport.pdf");
    /// ```
    #[inline]
    pub fn pdf(media_source: impl Into<MediaSource>, filename: impl Into<String>) -> Self {
        Self::new(media_source, MediaType::Document(DocumentExtension::Pdf)).filename(filename)
    }

    /// Creates a new `Media` instance specifically for a JPEG image.
    ///
    /// This is a convenience constructor that sets the `media_type` to `image/jpeg` automatically.
    ///
    /// # Arguments
    ///
    /// * `media_source` - The source of the JPEG image (bytes, URL, or WhatsApp ID).
    ///   Accepts anything convertible into a [`MediaSource`] enum variant.
    ///
    /// # Returns
    ///
    /// A new `Media` instance configured for a JPEG image.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Media;
    ///
    /// let jpeg_image = Media::jpeg("11133444488849");
    /// ```
    #[inline]
    pub fn jpeg(media_source: impl Into<MediaSource>) -> Self {
        Self::new(media_source, MediaType::Image(ImageExtension::Jpeg))
    }

    /// Creates a new `Media` instance specifically for a PNG image.
    ///
    /// This is a convenience constructor that sets the `media_type` to `image/png` automatically.
    ///
    /// # Arguments
    ///
    /// * `media_source` - The source of the JPEG image (bytes, URL, or WhatsApp ID).
    ///   Accepts anything convertible into a [`MediaSource`] enum variant.
    ///
    /// # Returns
    ///
    /// A new `Media` instance configured for a JPEG image.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Media;
    ///
    /// let jpeg_image = Media::png("11133444488849");
    /// ```
    #[inline]
    pub fn png(media_source: impl Into<MediaSource>) -> Self {
        Self::new(media_source, MediaType::Image(ImageExtension::Png))
    }

    /// Creates a new `Media` instance specifically for a WebP sticker.
    ///
    /// This is a convenience constructor that sets the `media_type` to `image/webp` automatically.
    ///
    /// # Arguments
    ///
    /// * `media_source` - The source of the WebP sticker (bytes, URL, or WhatsApp ID).
    ///   Accepts anything convertible into a [`MediaSource`] enum variant.
    ///
    /// # Returns
    ///
    /// A new `Media` instance configured for a WebP sticker.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Media;
    ///
    /// let webp_sticker = Media::sticker("https://example.com/sticker.webp");
    /// ```
    #[inline]
    pub fn sticker(media_source: impl Into<MediaSource>) -> Self {
        Self::new(media_source, MediaType::Sticker(StickerExtension::Webp))
    }

    /// Creates a new `Media` instance specifically for an MP4 video.
    ///
    /// This is a convenience constructor that sets the `media_type` to `video/mp4` automatically.
    ///
    /// # Arguments
    ///
    /// * `media_source` - The source of the MP4 video (bytes, URL, or WhatsApp ID).
    ///   Accepts anything convertible into a [`MediaSource`] enum variant.
    ///
    /// # Returns
    ///
    /// A new `Media` instance configured for an MP4 video.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Media;
    /// # let image_bytes = [0_u8;1024].to_vec();
    ///
    /// let mp4_video = Media::mp4(image_bytes);
    /// ```
    #[inline]
    pub fn mp4(media_source: impl Into<MediaSource>) -> Self {
        Self::new(media_source, MediaType::Video(VideoExtension::Mp4))
    }

    /// Create a `Media` struct by loading content from a file path.
    ///
    /// The file is loaded asynchronously from disk, and the appropriate
    /// [`MediaType`] (MIME type) is inferred from the file's content
    /// using the `infer` crate. The filename is automatically set from the path.
    ///
    /// # Arguments
    /// * `path` - The file path to the media content.
    ///
    /// # Returns
    /// A `Result` containing the `Media` struct if successful, or a [`MediaFromPathError`] if
    /// the file cannot be read or its type inferred.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::message::Media;
    ///
    /// # async fn example_media_from_path() -> Result<(), Box<dyn std::error::Error>> {
    /// let image = Media::from_path("assets/example_image.jpg")
    ///      .await?;
    /// println!("Loaded media with type: {:?}", image.media_type);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "media_ext")]
    #[inline]
    pub async fn from_path(path: impl AsRef<Path> + Send) -> Result<Self, MediaFromPathError> {
        let path_ref = path.as_ref();
        let data = tokio::fs::read(path_ref)
            .await
            .map_err(MediaFromPathError::Io)?;

        let media = Self::from_bytes(data).map_err(MediaFromPathError::Infer)?;

        Ok(if media.is_document() {
            let filename = path_ref.file_name().unwrap_or_default().to_string_lossy();
            media.filename(filename)
        } else {
            media
        })
    }

    /// Create a `Media` struct from in-memory bytes.
    ///
    /// The [`MediaType`] (MIME type) is inferred from the provided bytes using the `infer` crate.
    ///
    /// # Arguments
    /// * `data` - A `Vec<u8>` containing the raw media bytes.
    ///
    /// # Returns
    /// A `Result` containing the `Media` struct if successful, or a [`MediaInferError`] if
    /// the media type cannot be inferred.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::{Media, MediaType, ImageExtension};
    ///
    /// # fn example_media_from_bytes() -> Result<(), Box<dyn std::error::Error>> {
    /// let data = vec![0u8; 1024];
    /// # let media = Media::from_bytes(data)?;
    /// assert_eq!(media.media_type, MediaType::Image(ImageExtension::Png)); // Actual type depends on 'data'
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "media_ext")]
    #[inline]
    pub fn from_bytes(data: Vec<u8>) -> Result<Self, MediaInferError> {
        let mime_type = infer::get(&data)
            .map(|kind| kind.mime_type())
            .unwrap_or_else(|| "application/octet-stream");

        Ok(Self {
            media_source: MediaSource::Bytes(data),
            media_type: mime_type
                .parse()
                .map_err(|err: String| MediaInferError { err })?,
            caption: None,
            filename: None,
        })
    }

    /// Attaches an optional **caption** to the media.
    ///
    /// The caption will be displayed alongside the media in the message.
    /// Note: Captions are not supported for **audio** or **sticker** media types.
    #[inline]
    pub fn caption(mut self, caption: impl Into<Text>) -> Self {
        if !self.is_audio() && !self.is_sticker() {
            self.caption = Some(caption.into());
        }
        self
    }

    /// Suggests a **filename** for the recipient when downloading the media.
    ///
    /// This is only applicable and visible for **document** media types.
    #[inline]
    pub fn filename(mut self, filename: impl Into<String>) -> Self {
        if self.is_document() {
            self.filename = Some(filename.into());
        }
        self
    }

    /// Checks if the media's type is audio.
    ///
    /// # Returns
    ///
    /// `true` if the `media_type` is an audio variant, `false` otherwise.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Media;
    ///
    /// let audio_media = Media::new("some_audio_id", "audio/ogg".parse().unwrap());
    /// assert_eq!(audio_media.is_audio(), true);
    ///
    /// let image_media = Media::jpeg("https://example.com/photo.jpg");
    /// assert_eq!(image_media.is_audio(), false);
    /// ```
    #[inline]
    pub fn is_audio(&self) -> bool {
        matches!(self.media_type, MediaType::Audio(_))
    }

    /// Checks if the media's type is a document.
    ///
    /// # Returns
    ///
    /// `true` if the `media_type` is a document variant, `false` otherwise.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Media;
    ///
    /// let document_media = Media::pdf("some_doc_id", "report.pdf");
    /// assert_eq!(document_media.is_document(), true);
    ///
    /// let video_media = Media::mp4("https://example.com/video.mp4");
    /// assert_eq!(video_media.is_document(), false);
    /// ```
    #[inline]
    pub fn is_document(&self) -> bool {
        matches!(self.media_type, MediaType::Document(_))
    }

    /// Checks if the media's type is an image.
    ///
    /// # Returns
    ///
    /// `true` if the `media_type` is an image variant, `false` otherwise.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Media;
    ///
    /// let image_media = Media::jpeg("https://example.com/photo.jpg");
    /// assert_eq!(image_media.is_image(), true);
    ///
    /// let sticker_media = Media::sticker("some_sticker_id");
    /// assert_eq!(sticker_media.is_image(), false);
    /// ```
    #[inline]
    pub fn is_image(&self) -> bool {
        matches!(self.media_type, MediaType::Image(_))
    }

    /// Checks if the media's type is a sticker.
    ///
    /// # Returns
    ///
    /// `true` if the `media_type` is a sticker variant, `false` otherwise.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Media;
    ///
    /// let sticker_media = Media::sticker("some_sticker_id");
    /// assert_eq!(sticker_media.is_sticker(), true);
    ///
    /// let audio_media = Media::new("some_audio_id", "audio/ogg".parse().unwrap());
    /// assert_eq!(audio_media.is_sticker(), false);
    /// ```
    #[inline]
    pub fn is_sticker(&self) -> bool {
        matches!(self.media_type, MediaType::Sticker(_))
    }

    /// Checks if the media's type is a video.
    ///
    /// # Returns
    ///
    /// `true` if the `media_type` is a video variant, `false` otherwise.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Media;
    ///
    /// let video_media = Media::mp4("https://example.com/my_video.mp4");
    /// assert_eq!(video_media.is_video(), true);
    ///
    /// let document_media = Media::pdf("some_doc_id", "report.pdf");
    /// assert_eq!(document_media.is_video(), false);
    /// ```
    #[inline]
    pub fn is_video(&self) -> bool {
        matches!(self.media_type, MediaType::Video(_))
    }

    #[inline]
    pub fn into_upload_parts(self) -> Option<(Vec<u8>, MediaType, Cow<'static, str>)> {
        match self.media_source {
            MediaSource::Bytes(bytes) => {
                Some((bytes, self.media_type, Self::default_filename().into()))
            }
            _ => None,
        }
    }
}

/// Represents an error encountered from inferring the [`MediaType`] (MIME type)
/// of a data.
#[derive(thiserror::Error, Debug)]
#[error("{err}")]
pub struct MediaInferError {
    err: String,
}

/// Represents an error encountered from reading or inferring the [`MediaType`] (MIME type)
/// of a file
#[derive(thiserror::Error, Debug)]
pub enum MediaFromPathError {
    #[error("Error inferring media type: {0}")]
    Infer(MediaInferError),
    #[error("Error reading file: {0}")]
    Io(std::io::Error),
}

/// Text content
///
/// Represents a simple text message. The `preview_url` field controls
/// whether URLs in the message should generate link previews.
#[derive(Serialize, PartialEq, Clone, Debug, Default)]
#[non_exhaustive]
pub struct Text {
    /// Message text content
    pub body: String,
    /// Whether to generate link previews for URLs
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub preview_url: Option<bool>,
}

impl Text {
    /// Creates a new `Text` message instance.
    ///
    /// This constructor allows you to specify both the message's `body` content
    /// and whether `preview_url` functionality should be enabled.
    ///
    /// # Examples
    ///
    /// Create a text message with link preview enabled:
    /// ```rust
    /// use whatsapp_business_rs::message::Text;
    ///
    /// let message = Text::new("Check out our website: [https://example.com](https://example.com)", true);
    /// assert_eq!(message.body, "Check out our website: [https://example.com](https://example.com)");
    /// assert_eq!(message.preview_url, Some(true));
    /// ```
    ///
    /// Create a text message with link preview disabled:
    /// ```rust
    /// use whatsapp_business_rs::message::Text;
    ///
    /// let message = Text::new("Just some plain text.", false);
    /// assert_eq!(message.body, "Just some plain text.");
    /// assert_eq!(message.preview_url, Some(false));
    /// ```
    #[inline]
    pub fn new(body: impl Into<String>, preview_url: bool) -> Self {
        Self {
            body: body.into(),
            preview_url: Some(preview_url),
        }
    }

    #[inline]
    pub fn preview_url(mut self, preview_url: bool) -> Self {
        self.preview_url = Some(preview_url);
        self
    }
}

impl Deref for Text {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.body
    }
}

impl<S: Into<String>> From<S> for Text {
    /// Converts a string-like type into a `Text` message.
    ///
    /// The `preview_url` will be `None` by default.
    #[inline]
    fn from(value: S) -> Self {
        Text {
            body: value.into(),
            preview_url: None,
        }
    }
}

/// Emoji reaction to a previously sent or received message.
///
/// Use an empty string (`""`) for the emoji to remove a previous reaction.
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::message::{Reaction, MessageRef};
///
/// let message_to_react_to = MessageRef::from_message_id("wamid.XXXXX");
/// let reaction = Reaction::new('👍', message_to_react_to);
/// ```
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct Reaction {
    pub emoji: char,
    #[serde(flatten)]
    pub to: MessageRef,
}

impl Reaction {
    #[inline]
    pub fn new<'m, T>(emoji: char, to: T) -> Self
    where
        T: ToValue<'m, MessageRef>,
    {
        Self {
            emoji,
            to: to.to_value().into_owned(),
        }
    }
}

/// A physical location that can be shared with recipients.
///
/// Useful for pointing customers to your physical store or pickup point.
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::message::Location;
///
/// let store_location = Location::new(34.0522, -118.2437)
///     .name("Awesome Store")
///     .address("123 Main St, Anytown, USA");
/// ```
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct Location {
    /// Latitude coordinate
    #[serde(
        serialize_with = "serialize_str",
        deserialize_with = "deserialize_str::<f64, __D>"
    )]
    pub latitude: f64,
    /// Longitude coordinate
    #[serde(
        serialize_with = "serialize_str",
        deserialize_with = "deserialize_str::<f64, __D>"
    )]
    pub longitude: f64,
    /// Optional location name
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    /// Optional street address
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub address: Option<String>,
}

impl Location {
    #[inline]
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
            name: None,
            address: None,
        }
    }

    #[inline]
    pub fn name(mut self, location_name: impl Into<String>) -> Self {
        self.name = Some(location_name.into());
        self
    }

    #[inline]
    pub fn address(mut self, location_address: impl Into<String>) -> Self {
        self.address = Some(location_address.into());
        self
    }
}

/// Represents raw or reference-based media content.
///
/// This is the core abstraction for media transfer. A [`MediaSource`] can:
/// - Contain raw bytes (`Bytes`)
/// - Point to a public URL (`Url`)
/// - Refer to an uploaded WhatsApp media ID (`Id`)
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::message::MediaSource;
///
/// // From bytes
/// let media_source = MediaSource::Bytes(vec![0u8; 1024]);
///
/// // From WhatsApp media ID
/// let media_source_id = MediaSource::Id("MEDIA_ID_123".to_string());
/// ```
#[derive(Deserialize, PartialEq, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MediaSource {
    /// In-memory byte vector
    #[serde(skip)]
    Bytes(Vec<u8>),

    /// URL to publicly accessible content
    Link(String),

    /// WhatsApp media ID (from previously uploaded content)
    Id(String),
}

impl MediaSource {
    #[inline]
    pub fn bytes(bytes: Vec<u8>) -> Self {
        Self::Bytes(bytes)
    }

    #[inline]
    pub fn link(link: impl Into<String>) -> Self {
        Self::Link(link.into())
    }

    #[inline]
    pub fn id(id: impl Into<String>) -> Self {
        Self::Id(id.into())
    }
}

impl From<Vec<u8>> for MediaSource {
    #[inline]
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&str> for MediaSource {
    #[inline]
    fn from(value: &str) -> Self {
        // We used to fully parse as url::Url here but
        // now we make it cheaper since the ids are currently
        // numerical
        if value.contains('.') {
            MediaSource::link(value)
        } else {
            MediaSource::id(value)
        }
    }
}

impl From<String> for MediaSource {
    #[inline]
    fn from(value: String) -> Self {
        // We used to fully parse as url::Url here but
        // now we make it cheaper since the ids are currently
        // numerical
        if value.contains('.') {
            MediaSource::link(value)
        } else {
            MediaSource::id(value)
        }
    }
}

/// Supported media types with specific formats (MIME types).
///
/// Each variant represents a general media category and contains a sub-enum
/// for the specific file format/extension (MIME type) within that category.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MediaType {
    Audio(AudioExtension),
    Document(DocumentExtension),
    Image(ImageExtension),
    Sticker(StickerExtension),
    Video(VideoExtension),
}

impl MediaType {
    /// Returns the standard MIME type string for the given media type and extension.
    #[inline]
    pub fn mime_type(self) -> Cow<'static, str> {
        match self {
            MediaType::Audio(ext) => ext.mime_type().into(),
            MediaType::Document(ext) => ext.mime_type().into(),
            MediaType::Image(ext) => ext.mime_type().into(),
            MediaType::Sticker(ext) => ext.mime_type().into(),
            MediaType::Video(ext) => ext.mime_type().into(),
        }
    }
}

impl FromStr for MediaType {
    type Err = String;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (main, _) = s
            .split_once('/')
            .ok_or_else(|| "Invalid MIME type format".to_string())?;
        match main {
            "audio" => AudioExtension::from_mime_type(s)
                .map(MediaType::Audio)
                .ok_or_else(|| format!("Unsupported audio MIME type: {s}")),
            "application" | "text" => DocumentExtension::from_mime_type(s)
                .map(MediaType::Document)
                .ok_or_else(|| format!("Unsupported document MIME type: {s}")),
            "video" => VideoExtension::from_mime_type(s)
                .map(MediaType::Video)
                .ok_or_else(|| format!("Unsupported video MIME type: {s}")),
            "image" => {
                // We prefer stickers
                if let Some(sticker) = StickerExtension::from_mime_type(s) {
                    Ok(MediaType::Sticker(sticker))
                } else {
                    ImageExtension::from_mime_type(s)
                        .map(MediaType::Image)
                        .ok_or_else(|| format!("Unsupported image MIME type: {s}"))
                }
            }
            _ => Err(format!("Unsupported media type: {s}")),
        }
    }
}

macro_rules! declare_extensions {
    (
        $(
            $(#[$cont_attr:meta])*
            pub enum $name:ident {
                $($variant:ident = $value:literal),+
            }
        )*
    ) => {
        $(
            $(#[$cont_attr])*
            #[derive(Clone, Copy, PartialEq, Debug)]
            pub enum $name {
                $(
                    $variant,
                )+
            }

            impl $name {
                #[inline]
                fn mime_type(self) -> &'static str {
                    match self {
                        $(
                            Self::$variant => $value,
                        )+
                    }
                }

                #[inline]
                fn from_mime_type(mime_type: &str) -> Option<Self> {
                    match mime_type {
                        $(
                            $value => Some(Self::$variant),
                        )+
                        _ => None
                    }
                }
            }
        )*
    };
}

declare_extensions! {
    /// Supported audio formats
    pub enum AudioExtension {
        Aac = "audio/aac",
        Amr = "audio/amr",
        Mpeg = "audio/mpeg",
        Mp4 = "audio/mp4",
        Ogg = "audio/ogg"
    }

    /// Supported document formats
    pub enum DocumentExtension {
        Excel = "application/vnd.ms-excel",
        OpenDoc = "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        OpenPres = "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        OpenSheet = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Pdf = "application/pdf",
        Ppt = "application/vnd.ms-powerpoint",
        Text = "text/plain",
        Word = "application/msword"
    }

    /// Supported image formats
    pub enum ImageExtension {
        Jpeg = "image/jpeg",
        Png = "image/png",
        Webp = "image/webp"
    }

    /// Supported sticker formats
    pub enum StickerExtension {
        Webp = "image/webp"
    }

    /// Supported video formats
    pub enum VideoExtension {
        Mp4 = "video/mp4",
        Threegp = "video/3gp"
    }
}

derive! {
    /// Interactive message type
    ///
    /// Represents any message involving interactivity:
    /// either one sent to the user (e.g. with buttons or lists),
    /// or one received from user interaction (a button click or selection).
    ///
    /// # Variants
    /// - `Message`: An outgoing interactive message, typically built with [`InteractiveMessage`].
    /// - `Click`: An incoming response from user interaction, representing a button click
    ///   or a list item selection (often a [`Button::Reply`] or [`Button::Option`]).
    ///
    /// [`InteractiveMessage`]: crate::message::InteractiveMessage
    /// [`Button::Reply`]: crate::message::Button::Reply
    /// [`Button::Option`]: crate::message::Button::Option
    #[derive(#FromResponse, #IntoRequest, PartialEq, Clone, Debug)]
    #![serde(untagged)]
    #[non_exhaustive]
    pub enum InteractiveContent {
        /// Interactive message sent to user
        Message(InteractiveMessage),
        /// Response from user interaction (button click)
        Click(Button),
    }
}

impl<I: Into<InteractiveMessage>> From<I> for InteractiveContent {
    #[inline]
    fn from(value: I) -> Self {
        InteractiveContent::Message(value.into())
    }
}

derive! {
    /// Optional header content for an interactive message
    ///
    /// WhatsApp interactive messages can include a media or text header.
    /// Useful for providing context above the main message body.
    ///
    /// # Variants
    /// - `Media`: An image, video, or document
    /// - `Text`: A simple text heading
    #[derive(#ContentTraits, PartialEq, Clone, Debug)]
    #[non_exhaustive]
    pub enum InteractiveHeader {
        Media(InteractiveHeaderMedia),
        #![serde(serialize_with = "serialize_ordinary_text")]
        Text(Text),
    }
}

impl From<Media> for InteractiveHeader {
    #[inline]
    fn from(value: Media) -> Self {
        Self::Media(InteractiveHeaderMedia {
            media_source: value.media_source,
            media_type: value.media_type,
        })
    }
}

/// Media content used for interactive message headers (e.g., images, video).
///
/// This struct defines the source and type of media to be displayed in the header of an interactive WhatsApp message.
///
/// **Important Considerations for Media URLs and Authentication:**
///
/// While this structure provides flexibility in specifying media content (raw bytes, public URL, or WhatsApp ID),
/// it's crucial to understand a key limitation imposed by Meta's WhatsApp API regarding URLs:
///
/// * **Authenticated URL Restriction:** Any media URL, including those *allocated by Meta itself* after you upload
///   media (via bytes) or retrieve a URL using a WhatsApp media ID, often requires an `Access Token` for
///   download. When WhatsApp attempts to fetch such an authenticated URL for inclusion in a message, it can fail
///   with an "Authentication Error" (HTTP 401) because its internal system is not implicitly authorized to
///   download content from these authenticated URLs without explicit handling.
///
/// * **Impact on "Our Trick":** Our internal process of converting WhatsApp media IDs or uploaded bytes into
///   a Meta-provided URL is inhibited by this restriction. While we successfully obtain a URL, WhatsApp's
///   messaging system does not "relax" its authentication requirements for these *itself-generated* URLs,
///   leading to download failures when the message is sent.
///
/// * **Current Best Practice (Workaround):** For now, it is strongly recommended that you provide **publicly
///   accessible, unauthenticated URLs** for interactive header media. If your media needs to be uploaded or
///   is only available via a WhatsApp media ID, you may encounter issues until Meta either:
///   1.  Relaxes the authentication requirement for URLs it allocates (making them universally accessible).
///   2.  Allows direct submission of WhatsApp media IDs in interactive message payloads, circumventing the
///       need for a URL conversion step on our end.
///
/// * **Future Compatibility:** This struct's `MediaSource` design is maintained for future compatibility,
///   anticipating that Meta may address these URL authentication challenges, allowing for a more
///   seamless experience when using bytes or WhatsApp IDs as the media source.
#[derive(Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct InteractiveHeaderMedia {
    /// Media content source (bytes, URL, or WhatsApp ID)
    #[serde(flatten)]
    pub media_source: MediaSource,
    /// Type and format of media
    #[serde(rename = "mime_type")]
    pub media_type: MediaType,
}

impl From<Media> for InteractiveHeaderMedia {
    #[inline]
    fn from(value: Media) -> Self {
        InteractiveHeaderMedia {
            media_source: value.media_source,
            media_type: value.media_type,
        }
    }
}

impl InteractiveHeaderMedia {
    #[inline]
    pub fn new(media_source: impl Into<MediaSource>, media_type: MediaType) -> Self {
        Self {
            media_source: media_source.into(),
            media_type,
        }
    }
}

derive! {
    /// An outgoing interactive message.
    ///
    /// Use this to send a message with buttons, product lists, or other interactive elements.
    /// You can optionally include a header and footer to customize its appearance.
    ///
    /// ## Easy Creation with Tuples
    ///
    /// For convenience, you can create an interactive message `Draft` directly from tuples.
    /// The tuple elements correspond to the message parts in the order they are generally displayed,
    /// which makes creation intuitive:
    ///
    /// - **`(Header, Body, Action, Footer)`**
    /// - **`(Body, Action, Footer)`**
    /// - **`(Body, Action)`**
    ///
    /// This is the recommended way to create interactive messages.
    ///
    /// # Example (using tuples)
    /// ```rust,no_run
    /// use whatsapp_business_rs::message::{InteractiveMessage, InteractiveAction, Button, Text};
    ///
    /// let buttons = [
    ///     Button::reply("yes_id", "Yes"),
    ///     Button::reply("no_id", "No"),
    /// ];
    ///
    /// // Create a draft from a (Body, Action, Footer) tuple.
    /// let interactive_msg = (
    ///     Text::from("Do you want to proceed?"),
    ///     InteractiveAction::from(buttons),
    ///     Text::from("This is a test message."),
    /// );
    ///
    /// // You can now send this interactive_msg directly as an outgoing message.
    /// ```
    #[derive(#FromResponse, #IntoRequest, PartialEq, Clone, Debug)]
    #[non_exhaustive]
    pub struct InteractiveMessage {
        /// Interactive elements (buttons, etc.)
        #![serde(
            flatten,
            serialize_with = "serialize_interactive_action",
            deserialize_with = "deserialize_interactive_action"
        )]
        pub action: InteractiveAction,

        /// Optional header content
        #![serde(
            skip_serializing_if = "Option::is_none",
            default = "option_from_response_default::<InteractiveHeader>"
        )]
        pub header: Option<InteractiveHeader>,

        /// Main message text
        #![serde(serialize_with = "serialize_text_text")]
        pub body: Text,

        /// Optional footer text
        #![serde(
            skip_serializing_if = "Option::is_none",
            serialize_with = "serialize_text_text_opt",
            default = "option_from_response_default::<Text>"
        )]
        pub footer: Option<Text>,
    }
}

/// Interactive actions supported in WhatsApp messages
///
/// Defines what kind of interactive experience the message contains.
///
/// # Variants
/// - `Cta`: Call-to-action with a URL
/// - `CatalogDisplay`: Show products from a catalog
/// - `LocationRequest`: Ask user to share their location
/// - `OptionList`: Present multiple choice list
/// - `ProductDisplay`: Single product preview
/// - `ProductList`: Multiple product sections
/// - `Keyboard`: Quick-reply buttons
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[serde(tag = "name", content = "parameters")]
#[non_exhaustive]
pub enum InteractiveAction {
    /// A Call-to-Action button that opens a specified URL.
    #[serde(rename = "cta_url")]
    Cta(UrlButton),

    /// Displays a product catalog message, potentially with a specific product as a thumbnail.
    #[serde(rename = "catalog_message")]
    CatalogDisplay(CatalogDisplayOptions),

    /// A request for the user to send their current location.
    #[serde(rename = "send_location")]
    LocationRequest,

    #[doc(alias = "ListMessages")]
    #[serde(untagged)]
    OptionList(OptionList),

    #[doc(alias = "SingleProduct")]
    #[serde(untagged)]
    ProductDisplay(ProductRef),

    #[doc(alias = "MultiProduct")]
    #[serde(untagged)]
    ProductList(ProductList),

    #[doc(alias = "Buttons")]
    #[serde(untagged)]
    Keyboard(Keyboard),
}
// FIXME: CatalogDisplayOptions action['parameters'] cannot be empty.
// 'dk why anyone would throw a fit for including an optional field as empty.

impl InteractiveMessage {
    /// Creates a new interactive message with a main `body` text and an `action`.
    ///
    /// This is the primary constructor for `InteractiveMessage`. Headers and footers
    /// can be added using fluent methods.
    ///
    /// # Parameters
    /// - `action`: The [`InteractiveAction`] defining the interactive elements (e.g., buttons, list).
    /// - `body`: The main text content of the message.
    ///
    /// # Returns
    /// A new `InteractiveMessage` instance.
    ///
    /// [`InteractiveAction`]: crate::message::InteractiveAction
    #[inline]
    pub fn new(action: impl Into<InteractiveAction>, body: impl Into<Text>) -> Self {
        Self {
            action: action.into(),
            header: None,
            body: body.into(),
            footer: None,
        }
    }

    /// Add a footer to the message
    #[inline]
    pub fn footer(mut self, footer: impl Into<Text>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Add a header to the message
    #[inline]
    pub fn header(mut self, header: impl Into<InteractiveHeader>) -> Self {
        self.header = Some(header.into());
        self
    }
}

/// A collection of quick-reply buttons.
///
/// Used in `InteractiveAction::Keyboard` to present a set of clickable buttons
/// below a message.
///
/// # Fields
/// - `buttons`: A vector of [`Button`]s that will appear in the keyboard.
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::message::{Keyboard, Button};
///
/// let buttons = vec![
///     Button::reply("opt_yes", "Yes"),
///     // This is only theoretical... Whatsapp currently only allows reply button in
///     // a keyboard
///     Button::url("https://example.com", "Visit Website"),
/// ];
///
/// let keyboard = Keyboard::from(buttons);
/// // Or more fluently:
/// let keyboard_fluent: Keyboard = [Button::reply("ok", "OK")].into();
/// ```
/// [`Button`]: crate::message::Button
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct Keyboard {
    pub buttons: Vec<Button>,
}

impl Keyboard {
    #[inline]
    pub fn new<I, B>(buttons: I) -> Self
    where
        I: IntoIterator<Item = B>,
        B: Into<Button>,
    {
        Self {
            buttons: buttons.into_iter().map(Into::into).collect(),
        }
    }

    #[inline]
    fn empty() -> Self {
        Self {
            buttons: Vec::new(),
        }
    }
}

impl<I, B> From<I> for Keyboard
where
    I: IntoIterator<Item = B>,
    B: Into<Button>,
{
    #[inline]
    fn from(value: I) -> Self {
        Self::new(value)
    }
}

derive! {
    /// Button types used in interactive messages.
    ///
    /// These are clickable elements users interact with. Use `.reply()`, `.url()`,
    /// or `.call()` constructors to easily build them.
    ///
    /// # Variants
    /// - `Reply`: A button that sends a custom payload back to your webhook when clicked.
    /// - `Url`: A button that opens a web page in the user's browser when clicked.
    /// - `Call`: A button that initiates a phone call to a specified number.
    /// - `Option`: A button that is part of an `OptionList` (list message).
    ///
    /// **NOTE**: Not all button types can be used in all interactive message contexts (e.g.,
    /// `Option` buttons are exclusive to list messages, `Url` and `Call` buttons might
    /// not be supported in all interactive actions).
    #[derive(#DeserializeAdjacent, #SerializeAdjacent, PartialEq, Clone, Debug)]
    pub enum Button {
        /// A button that sends a `ReplyButton` payload back when clicked.
        #![serde(rename(deserialize = "button_reply"))]
        Reply(ReplyButton),
        /// A button that opens a URL when clicked.
        Url(UrlButton),
        /// A button that initiates a phone call when clicked.
        Call(CallButton),
        /// An option button used specifically within an `OptionList` message.
        #![serde(rename(deserialize = "list_reply"))]
        Option(OptionButton),
    }
}

/// A button that sends a callback payload when clicked.
///
/// This is typically used for quick replies, surveys, or simple selections
/// where your application needs to receive an ID upon user interaction.
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct ReplyButton {
    /// Unique ID sent when button is clicked
    #[serde(rename = "id")]
    pub call_back: String,
    /// The visible label text on the button.
    #[serde(rename = "title")]
    pub label: String,
}

impl ReplyButton {
    #[inline]
    pub fn new(call_back: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            call_back: call_back.into(),
            label: label.into(),
        }
    }
}

/// Button that opens a web URL in the browser
///
/// Use for CTAs, product links, or custom dashboards.
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct UrlButton {
    /// Web URL to open
    #[serde(rename = "url")]
    pub url: String,
    /// The visible label text on the button.
    #[serde(rename = "display_text")]
    pub label: String,
}

impl UrlButton {
    #[inline]
    pub fn new(url: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            label: label.into(),
        }
    }
}

/// An option button used within a list-type interactive message (`OptionList`).
///
/// This button type includes a description in addition to the label and callback ID.
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct OptionButton {
    /// A descriptive text for the option, displayed below the label in a list message.
    pub description: String,
    /// The visible label text of the option.
    #[serde(rename = "title")]
    pub label: String,
    /// Unique ID sent when option is clicked
    #[serde(rename = "id")]
    pub call_back: String,
}

impl OptionButton {
    #[inline]
    pub fn new(
        description: impl Into<String>,
        label: impl Into<String>,
        call_back: impl Into<String>,
    ) -> Self {
        Self {
            description: description.into(),
            label: label.into(),
            call_back: call_back.into(),
        }
    }
}

/// Button that initiates a phone call when tapped
///
/// WhatsApp will open the dialer with the specified number.
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct CallButton {
    /// Number to call when clicked
    pub phone_number: String,
}

impl CallButton {
    pub fn new(phone_number: impl Into<String>) -> Self {
        Self {
            phone_number: phone_number.into(),
        }
    }
}

impl Button {
    /// Create a quick-reply button
    #[inline]
    pub fn reply(call_back: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Reply(ReplyButton {
            call_back: call_back.into(),
            label: text.into(),
        })
    }

    /// Create a URL button
    #[inline]
    pub fn url(url: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Url(UrlButton {
            url: url.into(),
            label: text.into(),
        })
    }

    /// Create a call button
    #[inline]
    pub fn call(phone_number: impl Into<String>) -> Self {
        Self::Call(CallButton {
            phone_number: phone_number.into(),
        })
    }

    /// Returns the callback ID (`id`) of a `Reply` or `Option` button.
    ///
    /// This is the payload sent to your webhook when a user
    /// clicks a reply button or a list option.
    pub fn callback_id(&self) -> Option<&str> {
        match self {
            Button::Reply(rb) => Some(&rb.call_back),
            Button::Option(ob) => Some(&ob.call_back),
            _ => None, // `Url` and `Call` buttons don't send callback IDs
        }
    }

    /// Returns the visible label (display text) of the button.
    ///
    /// Note: This will return `None` for `Button::Call` as its
    /// underlying `CallButton` struct was not provided in the context.
    pub fn label(&self) -> Option<&str> {
        match self {
            Button::Reply(rb) => Some(&rb.label),
            Button::Url(ub) => Some(&ub.label),
            Button::Option(ob) => Some(&ob.label),
            Button::Call(_) => None,
        }
    }
}

/// Options for displaying a product catalog within an interactive message.
///
/// Used with the `InteractiveAction::CatalogDisplay` to show
/// a subset of your catalog or highlight a single item.
///
/// # Fields
/// - `thumbnail`: An optional [`ProductRef`] to use as the thumbnail preview for the catalog message.
///   If `None`, WhatsApp might use a default or the first product in the catalog.
///
/// [`ProductRef`]: crate::catalog::ProductRef
#[derive(PartialEq, Clone, Debug, Default)]
#[non_exhaustive]
pub struct CatalogDisplayOptions {
    /// Product to use as the thumbnail preview (optional)
    pub thumbnail: Option<ProductRef>,
}

impl CatalogDisplayOptions {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn from_thumbnail<'a>(product: impl ToValue<'a, ProductRef>) -> Self {
        Self::default().thumbnail(product)
    }

    #[inline]
    pub fn thumbnail<'a>(mut self, product: impl ToValue<'a, ProductRef>) -> Self {
        self.thumbnail = Some(product.to_value().into_owned());
        self
    }
}

/// A list of products grouped into sections for an interactive message.
///
/// Use `ProductList` to display items from your WhatsApp Business Catalog,
/// allowing users to browse and select products directly within the chat.
/// Products are organized into logical **sections** to improve navigation
/// and user experience (e.g., "Electronics", "Apparel", "Today's Deals").
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::message::{ProductList, Section};
/// use whatsapp_business_rs::catalog::ProductRef;
/// use whatsapp_business_rs::CatalogRef;
///
/// // Create individual product sections
/// let electronics_section = Section::new(
///     "Electronics",
///     vec![
///         ProductRef::from("prod_101"),
///         ProductRef::from("prod_102"),
///     ],
/// );
///
/// let apparel_section = Section::new(
///     "Apparel",
///     vec![
///         ProductRef::from("prod_201"),
///         ProductRef::from("prod_202"),
///     ],
/// );
///
/// // Build the ProductList using the builder pattern
/// let product_list = ProductList::new_section(electronics_section, "your_catalog_id")
///         .add_section(apparel_section);
///
/// // You can now send `product_list` as part of an interactive message
/// ```
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct ProductList {
    /// Sections of products (e.g., categories or bundles).
    /// Grouping products into sections helps users easily navigate large catalogs.
    pub sections: Vec<Section<ProductRef>>,
    /// Catalog to retrieve products from
    #[serde(rename = "catalog_id")]
    pub catalog: CatalogRef,
}

impl ProductList {
    /// Creates a new `ProductList` with a multiple sections.
    #[inline]
    pub fn new_sections<'c, C, I>(sections: I, catalog: C) -> Self
    where
        I: IntoIterator<Item = Section<ProductRef>>,
        C: ToValue<'c, CatalogRef>, // ToValue has more impls than Into
    {
        Self {
            sections: sections.into_iter().collect(),
            catalog: catalog.to_value().into_owned(),
        }
    }

    /// Creates a new `ProductList` with a single initial section.
    #[inline]
    pub fn new_section<'c, C>(section: Section<ProductRef>, catalog: C) -> Self
    where
        C: ToValue<'c, CatalogRef>, // ToValue has more impls than Into
    {
        Self {
            sections: vec![section],
            catalog: catalog.to_value().into_owned(),
        }
    }

    /// Adds a section of products to the `ProductList`.
    ///
    /// Call this method multiple times to add all the desired categories or groupings
    #[inline]
    pub fn add_section(mut self, section: Section<ProductRef>) -> Self {
        self.sections.push(section);
        self
    }
}

/// A list of options presented in an interactive message.
///
/// Use `OptionList` to display a menu of choices to the user, guiding them through
/// predefined conversational paths. Options are organized into **sections** to
/// provide logical groupings (e.g., "Delivery Speed", "Support Options", "Account Settings"),
/// making it easier for users to find and select relevant choices.
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::message::{OptionList, Section, OptionButton};
///
/// // Create individual option sections
/// let shipping_options = Section::new(
///     "Delivery Speed",
///     [
///         OptionButton::new(
///             "Get it in 1-2 business days",
///             "express_ship",
///             "Express",
///         ),
///         OptionButton::new(
///             "Standard delivery in 3-5 business days",
///             "standard_ship",
///             "Standard",
///         ),
///     ]
/// );
///
/// let support_options: Section<OptionButton> = Section::new(
///     "Support",
///     [
///         OptionButton::new(
///             "Get help from a human",
///             "live_chat",
///             "Live Chat",
///         ),
///         OptionButton::new(
///             "Browse FAQs",
///             "faq_page",
///             "FAQs",
///         ),
///     ],
/// );
///
/// // Build the OptionList using the builder pattern
/// let option_list = OptionList::new_section(shipping_options, "Select an Option")
///     .add_section(support_options);
///
/// // You can now send `option_list` as part of an interactive message
/// ```
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
pub struct OptionList {
    /// Sections of options.
    /// Grouping options into sections helps users quickly find the choices they need.
    pub sections: Vec<Section<OptionButton>>,

    /// The label for the button that, when clicked, expands to reveal the list options.
    /// E.g., "Shipping Options" or "Main Menu".
    #[serde(rename = "button")]
    pub label: String,
}

impl OptionList {
    /// Creates a new `OptionList ` with a multiple sections.
    #[inline]
    pub fn new_sections<I>(sections: I, label: impl Into<String>) -> Self
    where
        I: IntoIterator<Item = Section<OptionButton>>,
    {
        Self {
            sections: sections.into_iter().collect(),
            label: label.into(),
        }
    }

    /// Creates a new `OptionList` with a single initial section.
    #[inline]
    pub fn new_section(section: Section<OptionButton>, label: impl Into<String>) -> Self {
        Self {
            sections: vec![section],
            label: label.into(),
        }
    }

    /// Adds a section of options to the `OptionList`.
    ///
    /// Call this method multiple times to add all the desired categories or groupings
    /// of options.
    #[inline]
    pub fn add_section(mut self, section: Section<OptionButton>) -> Self {
        self.sections.push(section);
        self
    }
}

serde_section! {
/// A section within an `OptionList` or `ProductList`.
///
/// This struct allows grouping related items (either `OptionButton`s for lists or
/// `ProductRef`s for product lists) under a common title. Sections are essential
/// for organizing interactive messages, making them easier for users to navigate
/// and understand.
///
/// # Type Parameters
/// - `Item`: The type of items contained within this section (e.g., `OptionButton` or `ProductRef`).
///
/// # Example (for `OptionList`)
/// ```rust
/// use whatsapp_business_rs::message::{Section, OptionButton};
///
/// let shipping_options: Section<OptionButton> = Section::new(
///     "Delivery Speed", // The title displayed above the options
///     [
///         OptionButton::new(
///             "Get it in 1-2 business days",
///             "express_ship",
///             "Express",
///         ),
///         OptionButton::new(
///             "Standard delivery in 3-5 business days",
///             "standard_ship",
///             "Standard",
///         ),
///     ],
/// );
/// ```
/// # Example (for `ProductList`)
/// ```rust
/// use whatsapp_business_rs::message::Section;
/// use whatsapp_business_rs::catalog::ProductRef;
///
/// let today_s_deals: Section<ProductRef> = Section::new(
///     "Today's Deals", // The title for this product category
///     vec![
///         ProductRef::from("deal_item_1"),
///         ProductRef::from("deal_item_2"),
///     ],
/// );
/// ```
#[derive(PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct Section<Item> {
    /// The title of the section, displayed above its items.
    pub title: String,
    /// A vector of the items belonging to this section.
    pub items: Vec<Item>,
}
}

impl<Item> Section<Item> {
    /// Creates a new `Section`.
    ///
    /// This is the primary way to create a section, allowing you to specify its title
    /// and provide an iterable collection of items (e.g., `Vec`, array).
    ///
    /// # Arguments
    /// * `title` - The title of the section, displayed to the user.
    /// * `items` - An iterable collection of items that belong to this section.
    ///
    /// # Examples
    /// Creating a section of `OptionButton`s:
    /// ```rust
    /// use whatsapp_business_rs::message::{Section, OptionButton};
    ///
    /// let my_options: Section<OptionButton> = Section::new(
    ///     "Choose an Action",
    ///     [
    ///         OptionButton::new(
    ///             "Option A",
    ///             "Desc A",
    ///             "A",
    ///         ),
    ///         OptionButton::new(
    ///             "Option B",
    ///             "Desc B",
    ///             "B",
    ///         ),
    ///     ],
    /// );
    /// ```
    ///
    /// Creating a section of `ProductRef`s:
    /// ```rust
    /// use whatsapp_business_rs::message::Section;
    /// use whatsapp_business_rs::catalog::ProductRef;
    ///
    /// let popular_products: Section<ProductRef> = Section::new(
    ///     "Popular Products",
    ///     vec![
    ///         ProductRef::from("prod_xyz"),
    ///         ProductRef::from("prod_abc"),
    ///     ],
    /// );
    /// ```
    #[inline]
    pub fn new<I, T>(title: impl Into<String>, items: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Item>,
    {
        Self {
            title: title.into(),
            items: items.into_iter().map(Into::into).collect(),
        }
    }
}

/// Product order message
///
/// Represents an order placed through WhatsApp
#[doc(alias = "Cart")]
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct Order {
    /// Ordered products
    #[serde(rename = "product_items")]
    pub products: Vec<OrderProduct>,
    /// An optional note or message associated with the order.
    #[serde(rename = "text", serialize_with = "serialize_ordinary_text")]
    pub note: Text,
    /// Catalog from which the products in this order originate.
    #[serde(rename = "catalog_id")]
    pub catalog: CatalogRef,
}

/// A single product item within an [`Order`] message.
///
/// Represents a specific product from a catalog, along with its quantity,
/// unit price, and currency.
///
/// [`Order`]: crate::message::Order
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct OrderProduct {
    /// A reference to the product itself.    
    #[serde(flatten)]
    pub product: ProductRef,

    /// The quantity of this product in the order.
    #[serde(
        serialize_with = "serialize_str",
        deserialize_with = "deserialize_str::<usize, __D>"
    )]
    pub quantity: usize,

    /// The price per unit of the product.
    #[serde(
        rename = "item_price",
        serialize_with = "serialize_str",
        deserialize_with = "deserialize_str::<f64, __D>"
    )]
    pub unit_price: f64,

    /// Currency code (e.g., "USD")
    pub currency: String,
}

/// Represents the content of an **error message received from WhatsApp**.
///
/// This struct is used when WhatsApp's API sends a message indicating that it
/// could not process or recognize an incoming message due to an unsupported
/// or malformed content type. It contains a vector of one or more `MetaError`
/// instances that describe the specific issues.
///
/// This `ErrorContent` is typically encountered when parsing webhook payloads
/// for the `Content::Error` variant.
///
/// # Fields
/// - `errors`: A `Vec<MetaError>` containing details about the specific
///   errors encountered by Meta's platform when processing a message.
///
/// # Example (Parsed from a webhook payload)
/// ```json
/// [
///     {
///         "code": 131051,
///         "title": "Unsupported message type",
///         "details": "Message type is not currently supported"
///     }
/// ]
/// ```
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
#[non_exhaustive]
#[serde(transparent)]
pub struct ErrorContent {
    pub errors: Vec<MetaError>,
}

/// Lightweight reference to a [`Message`].
///
/// Used primarily for message context (like replies) without needing to
/// load the full message content. It contains the message ID and
/// optionally the sender and recipient identities.
#[derive(Deserialize, Serialize, Eq, Clone, Debug)]
pub struct MessageRef {
    #[serde(alias = "id")]
    pub(crate) message_id: String,
    #[serde(flatten, skip_serializing)]
    pub(crate) sender: Option<IdentityRef>,
    #[serde(skip)]
    pub(crate) recipient: Option<IdentityRef>,
}

impl MessageRef {
    /// Creates a new `MessageRef` from a message ID.
    ///
    /// # Parameters
    /// - `id`: The unique ID of the message.
    ///
    /// # Returns
    /// A new `MessageRef` instance.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::MessageRef;
    ///
    /// let msg_ref = MessageRef::from_message_id("wamid.ABCDEFG");
    /// assert_eq!(msg_ref.message_id(), "wamid.ABCDEFG");
    /// ```
    #[inline]
    pub fn from_message_id(id: impl Into<String>) -> Self {
        MessageRef {
            message_id: id.into(),
            sender: None,
            recipient: None,
        }
    }

    /// Gets the ID of the referenced message.
    #[inline]
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    /// Gets the optional sender's identity of the referenced message.
    #[inline]
    pub fn sender(&self) -> Option<&IdentityRef> {
        self.sender.as_ref()
    }

    /// Gets the optional recipient's identity of the referenced message.
    #[inline]
    pub fn recipient(&self) -> Option<&IdentityRef> {
        self.recipient.as_ref()
    }
}

impl<T: Into<String>> From<T> for MessageRef {
    #[inline]
    fn from(value: T) -> Self {
        Self::from_message_id(value)
    }
}

/// Just a convenience trait for all MessageRef
pub trait MessageRefExt<'a>: ToValue<'a, MessageRef> + Sized {
    /// Make this outgoing draft a reply to this message
    #[inline]
    fn swipe_reply(self, draft: impl IntoDraft) -> Draft {
        let draft = draft.into_draft();
        draft.reply_to(self)
    }

    /// React to this message
    #[inline]
    fn react_to(self, emoji: char) -> Reaction {
        Reaction::new(emoji, self)
    }
}

impl<'a, T> MessageRefExt<'a> for T where T: ToValue<'a, MessageRef> {}

/// A complete WhatsApp message, representing either a sent or received message.
///
/// This struct encapsulates all metadata associated with a WhatsApp message,
/// including its content, context, sender, recipient, unique ID, timestamp,
/// and current delivery status.
///
/// # Example
/// ```rust,no_run
/// use whatsapp_business_rs::Message;
///
/// fn process_message(msg: &Message) {
///     println!("Message ID: {}", msg.id);
///     println!("From: {}", msg.sender.phone_id);
///     println!("To: {}", msg.recipient.phone_id);
///     println!("Content: {:?}", msg.content);
///     println!("Status: {:?}", msg.message_status);
/// }
/// ```
#[derive(PartialEq, Clone, Debug)]
#[non_exhaustive]
pub struct Message {
    /// The unique message ID assigned by WhatsApp.
    pub id: String,

    /// The identity of the sender of this message.
    pub sender: Identity,

    /// The identity of the recipient of this message.
    pub recipient: Identity,

    /// Message content (text, media, interactive, etc.)
    pub content: Content,

    /// Message context (e.g., if it's a reply to another message)
    pub context: Context,

    /// The timestamp of the message (usually in seconds since Unix epoch).
    pub timestamp: Timestamp,

    /// The current delivery status of the message.
    pub message_status: MessageStatus,
}

impl Deref for Message {
    type Target = Content;

    fn deref(&self) -> &Self::Target {
        &self.content
    }
}

impl Message {
    /// Creates a lightweight reference to this message [`MessageRef`].
    ///
    /// This is useful when you need to refer to a message's ID and sender/recipient
    /// without keeping the full message content in memory (e.g., for replies).
    ///
    /// # Returns
    /// A [`MessageRef`] instance derived from this message.
    ///
    /// [`MessageRef`]: crate::message::MessageRef
    #[inline]
    pub fn as_ref(&self) -> MessageRef {
        MessageRef {
            message_id: self.id.clone(),
            sender: Some(self.sender.as_ref()),
            recipient: Some(self.recipient.as_ref()),
        }
    }

    /// Creates a lightweight reference to this message [`MessageRef`] without metadata.
    #[inline]
    pub fn as_ref_no_metadata(&self) -> MessageRef {
        MessageRef {
            message_id: self.id.clone(),
            sender: None,
            recipient: None,
        }
    }

    /// Begins the process of downloading media content for this message, if it contains media.
    ///
    /// This is a convenience method that delegates to `self.content.download_media()`.
    ///
    /// # Arguments
    /// * `client` - The WhatsApp API [`Client`] to use for the download operation.
    /// * `dst` - Destination.
    ///
    /// # Returns
    /// An `Option<DownloadMedia>`.
    ///
    /// [`Client`]: crate::client::Client
    /// [`DownloadMedia`]: crate::message::DownloadMedia
    #[inline]
    pub fn download_media<'dst, Dst>(
        &self,
        dst: &'dst mut Dst,
        client: &Client,
    ) -> Option<DownloadMedia<'dst, Dst>>
    where
        Dst: AsyncWrite + Send + Unpin,
    {
        self.content.download_media(dst, client)
    }
}

/// Message context information.
///
/// Contains metadata about a message's relationship to other messages,
/// such as whether it's a reply, a forwarded message, or refers to a product.
#[derive(Deserialize, PartialEq, Clone, Debug, Default)]
pub struct Context {
    /// An optional reference to the message this message is replying to.    
    #[serde(flatten, default)]
    pub(crate) replied_to: Option<MessageRef>,

    /// An optional reference to a product if this message is related to a product (e.g., from a catalog).
    #[serde(skip_serializing, default)]
    pub(crate) reffered_product: Option<ProductRef>,

    /// Indicates if the message was forwarded.
    #[serde(skip_serializing, default)]
    pub(crate) forwarded: Option<bool>,

    /// Indicates if the message was frequently forwarded (e.g., more than 5 times).
    #[serde(skip_serializing, default)]
    pub(crate) frequently_forwarded: Option<bool>,
}

impl Context {
    /// Gets an optional reference to the message this message is replying to.
    ///
    /// # Returns
    /// `Some(&MessageRef)` if this message is a reply, otherwise `None`.
    #[inline]
    pub fn replied_to(&self) -> Option<&MessageRef> {
        self.replied_to.as_ref()
    }

    /// Gets an optional reference to a product if this message refers to one.
    ///
    /// # Returns
    /// `Some(&ProductRef)` if a product is referred, otherwise `None`.
    #[inline]
    pub fn reffered_product(&self) -> Option<&ProductRef> {
        self.reffered_product.as_ref()
    }

    /// Checks if the message was forwarded.
    ///
    /// # Returns
    /// `Some(true)` if forwarded, `Some(false)` if not forwarded, `None` if information is unavailable.
    #[inline]
    pub fn is_forwarded(&self) -> Option<bool> {
        self.forwarded
    }

    /// Checks if the message was frequently forwarded.
    ///
    /// # Returns
    /// `Some(true)` if frequently forwarded, `Some(false)` if not, `None` if information is unavailable.
    #[inline]
    pub fn is_frequently_forwarded(&self) -> Option<bool> {
        self.frequently_forwarded
    }
}

/// Message builder for creating messages to send.
///
/// Provides a fluent interface for constructing WhatsApp messages.
/// Once configured, the `Draft` can be sent using [`Client::message().send()`].
///
/// # Example: Sending a text message
/// ```rust,no_run
/// use whatsapp_business_rs::{Draft, IdentityRef};
///
/// # async fn example_draft_text() -> Result<(), Box<dyn std::error::Error>> {
/// let recipient = IdentityRef::user("+16012345678");
/// let draft = Draft::text("Hello from Rust!");
///
/// // To send:
/// // client.message("YOUR_BUSINESS_NO_ID").send(&recipient, draft).await?;
/// # Ok(())
/// # }
/// ```
///
/// # Example: Sending media with a caption and replying to a message
/// ```rust,no_run
/// use whatsapp_business_rs::{
///     message::{Media, Message},
///     Draft,
/// };
///
/// # async fn example_draft_media(original_message: Message)
/// # -> Result<(), Box<dyn std::error::Error>> {
/// let image_bytes = vec![0u8; 1024]; // Replace with actual image data
/// let media = Media::from_bytes(image_bytes)?;
///
/// let media_draft = Draft::media(media)
///     .with_caption("Check out this cool image!")
///     .reply_to(&original_message); // Set as a reply
///
/// // To send:
/// // client.message("YOUR_BUSINESS_NO_ID").send(&original_message.sender, media_draft).await?;
/// # Ok(())
/// # }
/// ```
/// [`Client::message().send()`]: crate::client::Client::message
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
#[must_use = "Draft is an unsent message that might need to be sent."]
pub struct Draft {
    /// The actual content of the message to be sent.
    pub content: Content,
    pub context: DraftContext,
    /// An arbitrary string that will be echoed back in the webhook callback for tracking purposes.
    pub biz_opaque_callback_data: Option<String>,
}

impl Draft {
    /// Creates a new, empty message draft.
    ///
    /// By default, the content is `Text` with an empty body.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a message draft containing only text content.
    ///
    /// # Arguments
    /// * `text` - The text content for the message. Can be a `&str`, `String`, or `Text`.
    ///
    /// # Returns
    /// A `Draft` instance configured with text content.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Draft;
    /// # use whatsapp_business_rs::message::Content;
    ///
    /// let draft = Draft::text("Hello WhatsApp!");
    /// assert!(matches!(draft.content, Content::Text(_)));
    /// ```
    #[inline]
    pub fn text(text: impl Into<Text>) -> Self {
        Self {
            content: Content::Text(text.into()),
            ..Default::default()
        }
    }

    /// Creates a message draft containing media content (image, video, document, etc.).
    ///
    /// # Arguments
    /// * `media` - The [`Media`] content to be sent.
    ///
    /// # Returns
    /// A `Draft` instance configured with media content.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::{Draft, Media};
    /// # async fn example_draft_media_constructor() -> Result<(), Box<dyn std::error::Error>> {
    /// let dummy_image = Media::from_bytes(vec![0u8; 16])?;
    /// let draft = Draft::media(dummy_image);
    /// assert!(matches!(draft.content, whatsapp_business_rs::message::Content::Media(_)));
    /// # Ok(())
    /// # }
    /// ```
    /// [`Media`]: crate::message::Media
    #[inline]
    pub fn media(media: impl Into<Media>) -> Self {
        Self {
            content: Content::Media(media.into()),
            ..Default::default()
        }
    }

    /// Sets the message content to a specific geographic coordinate.
    ///
    /// This initiates a Location message. You can subsequently chain
    /// [`location_name`](Self::location_name) and [`location_address`](Self::location_address)
    /// to add details.
    pub fn location(latitude: f64, longitude: f64) -> Self {
        Self {
            content: Content::Location(Location::new(latitude, longitude)),
            ..Default::default()
        }
    }

    /// Sets the message as a **Reaction** to a specific message.
    ///
    /// This is a shortcut for manually constructing a `Reaction` struct.
    ///
    /// # Arguments
    /// * `emoji` - The emoji char (e.g., '👍', '❤️').
    /// * `to` - The message you are reacting to.
    pub fn react<'m>(emoji: char, to: impl ToValue<'m, MessageRef>) -> Self {
        let reaction = Reaction::new(emoji, to);
        Self {
            content: Content::Reaction(reaction),
            ..Default::default()
        }
    }

    /// Creates a message draft containing interactive content (buttons, lists, products).
    ///
    /// # Arguments
    /// * `interactive_message` - The [`InteractiveMessage`] to be sent.
    ///
    /// # Returns
    /// A `Draft` instance configured with interactive content.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::{
    ///     Button, Draft, InteractiveAction, InteractiveMessage, Text,
    /// };
    ///
    /// let buttons = [Button::reply("hello", "Hello")];
    /// let interactive_msg = InteractiveMessage::new(
    ///     InteractiveAction::Keyboard(buttons.into()),
    ///     Text::from("Choose:"),
    /// );
    /// let draft = Draft::interactive(interactive_msg);
    /// assert!(matches!(
    ///     draft.content,
    ///     whatsapp_business_rs::message::Content::Interactive(_)
    /// ));
    /// ```
    /// [`InteractiveMessage`]: crate::message::InteractiveMessage
    #[inline]
    pub fn interactive(interactive_message: InteractiveMessage) -> Self {
        Self {
            content: Content::Interactive(interactive_message.into()),
            ..Default::default()
        }
    }

    /// Set this as a reply to another message
    ///
    /// # Arguments
    /// * `message` - Message to reply to
    #[inline]
    pub fn reply_to<'t, T>(mut self, message: T) -> Self
    where
        T: ToValue<'t, MessageRef>,
    {
        self.context.replied_to = Some(message.to_value().into_owned());
        self
    }

    /// Sets the media caption for the message draft.
    ///
    /// This method is only effective if the `Draft`'s content is `Media`.
    /// If the content is not `Media`, calling this method will have no effect.
    ///
    /// # Arguments
    /// * `caption` - The text content for the media caption. Can be a `&str`, `String`, or `Text`.
    ///
    /// # Returns
    /// The `Draft` instance with the caption set (if applicable), allowing for fluent chaining.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::{Draft, Media, Content};
    /// # async fn example_draft_with_caption() -> Result<(), Box<dyn std::error::Error>> {
    /// let dummy_image = Media::from_bytes(vec![0u8; 16])?;
    /// let draft = Draft::media(dummy_image)
    ///     .with_caption("This is an image caption.");
    ///
    /// if let Content::Media(media) = draft.content {
    ///     assert_eq!(media.caption.unwrap().body, "This is an image caption.");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn with_caption(mut self, caption: impl Into<Text>) -> Self {
        if let Content::Media(mut media) = self.content {
            media = media.caption(caption);
            self.content = Content::Media(media);
        }
        self
    }

    /// Sets arbitrary opaque data that will be returned in the webhook callback.
    ///
    /// This is useful for tracking messages, associating them with internal
    /// system IDs, or passing context for asynchronous webhook processing.
    ///
    /// # Arguments
    /// * `data` - A `String` containing the opaque callback data.
    ///
    /// # Returns
    /// The `Draft` instance with the callback data set, allowing for fluent chaining.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::message::Draft;
    ///
    /// let draft = Draft::text("Your order is ready!")
    ///     .with_biz_opaque_callback_data("order_id_789_customer_123");
    /// assert_eq!(draft.biz_opaque_callback_data.unwrap(), "order_id_789_customer_123");
    /// ```
    #[inline]
    pub fn with_biz_opaque_callback_data(mut self, data: impl Into<String>) -> Self {
        self.biz_opaque_callback_data = Some(data.into());
        self
    }

    /// Sets the body of the message.
    ///
    /// If the current message is not an interactive type, it will be
    /// automatically converted into one, using existing text or media
    /// as the body or header, respectively.
    #[inline]
    pub fn body(mut self, body: impl Into<Text>) -> Self {
        self.promote_to_interactive_mut().body = body.into();
        self
    }

    /// Adds a header to the message.
    ///
    /// If the current message is not an interactive type, it will be
    /// automatically converted.
    #[inline]
    pub fn header(mut self, header: impl Into<InteractiveHeader>) -> Self {
        self.promote_to_interactive_mut().header = Some(header.into());
        self
    }

    /// Adds a footer to the message.
    ///
    /// If the current message is not an interactive type, it will be
    /// automatically converted.
    #[inline]
    pub fn footer(mut self, footer: impl Into<Text>) -> Self {
        self.promote_to_interactive_mut().footer = Some(footer.into());
        self
    }

    /// Enables or disables the web page preview for URLs contained in the message text.
    ///
    /// # Behavior
    /// * This setting only applies if the content is currently a [`Content::Text`] type.
    /// * If set to `true`, WhatsApp will attempt to generate a preview card for the first URL found.
    ///
    /// # Arguments
    /// * `preview_url` - `true` to enable previews, `false` to disable them.
    pub fn preview_url(mut self, preview_url: bool) -> Self {
        if let Content::Text(text) = &mut self.content {
            text.preview_url = Some(preview_url);
        }
        self
    }

    /// Adds a reply button to the message.
    ///
    /// This will convert the message to an interactive message with a `Keyboard`
    /// action if it isn't one already. If the existing action is not a `Keyboard`,
    /// it will be **overwritten**.
    pub fn add_reply_button(
        mut self,
        call_back: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        let interactive = self.promote_to_interactive_mut();
        let button = Button::reply(call_back, label);

        match &mut interactive.action {
            InteractiveAction::Keyboard(keyboard) => {
                keyboard.buttons.push(button);
            }
            _ => {
                // Opinionated: Overwrite a non-keyboard action if a button is added.
                interactive.action = InteractiveAction::Keyboard(Keyboard {
                    buttons: vec![button],
                });
            }
        }
        self
    }

    /// Adds a URL button to the message.
    ///
    /// Note: A message can only contain a maximum of one URL button and two reply buttons.
    /// This method behaves identically to `add_reply_button`.
    pub fn add_url_button(mut self, url: impl Into<String>, label: impl Into<String>) -> Self {
        let interactive = self.promote_to_interactive_mut();
        let button = Button::url(url, label);

        match &mut interactive.action {
            InteractiveAction::Keyboard(keyboard) => {
                keyboard.buttons.push(button);
            }
            _ => {
                interactive.action = InteractiveAction::Keyboard(Keyboard {
                    buttons: vec![button],
                });
            }
        }
        self
    }

    /// Replaces the current action with a standard WhatsApp "Call-to-Action" (CTA) URL button.
    ///
    /// This creates a dedicated button that opens a specific web link when tapped.
    ///
    /// # Note
    /// This is distinct from [`Self::add_url_button`], which adds a button to a Quick Reply keyboard.
    /// This method uses the native `cta_url` interactive action.
    ///
    /// # Arguments
    /// * `url` - The target URL to open (e.g., "https://www.example.com").
    /// * `label` - The text displayed on the button.
    pub fn with_cta_url(mut self, url: impl Into<String>, label: impl Into<String>) -> Self {
        let interactive = self.promote_to_interactive_mut();

        interactive.action = InteractiveAction::Cta(UrlButton::new(url, label));
        self
    }

    /// Sets the message action to a list with the given button label.
    ///
    /// This will overwrite any existing interactive action (like a keyboard).
    /// Chain this with `.add_list_option()` to populate the list.
    pub fn list(mut self, label: impl Into<String>) -> Self {
        let interactive = self.promote_to_interactive_mut();
        interactive.action = InteractiveAction::OptionList(OptionList {
            label: label.into(),
            sections: vec![],
        });
        self
    }

    /// Adds a new section to a list message.
    ///
    /// This allows for creating structured lists with multiple, titled sections.
    /// Subsequent calls to `.add_list_option()` will add items to this new section.
    ///
    /// If the message is not an `OptionList`, this is a no-op.
    pub fn add_list_section(mut self, title: impl Into<String>) -> Self {
        let interactive = self.promote_to_interactive_mut();
        if let InteractiveAction::OptionList(list) = &mut interactive.action {
            list.sections
                .push(Section::new(title, Vec::<OptionButton>::new()));
        }
        self
    }

    /// Adds an option to a list message.
    ///
    /// If the message is not a list, this is a no-op. If the list has no
    /// sections, a default one titled "Options" will be created automatically.
    pub fn add_list_option(
        mut self,
        call_back: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let interactive = self.promote_to_interactive_mut();
        if let InteractiveAction::OptionList(list) = &mut interactive.action {
            if list.sections.is_empty() {
                // Opinionated: create a default section if user adds an option to a new list.
                list.sections
                    .push(Section::new("Options", Vec::<OptionButton>::new()));
            }
            // Add the option to the last section.
            if let Some(last_section) = list.sections.last_mut() {
                let option_button = OptionButton::new(description, label, call_back);
                last_section.items.push(option_button);
            }
        }
        self
    }

    /// Adds a name to the location (e.g., "Headquarters").
    ///
    /// # Behavior
    /// This method is a no-op if the current content is not already a [`Content::Location`].
    /// You must call [`location`](Self::location) first.
    pub fn location_name(mut self, name: impl Into<String>) -> Self {
        if let Content::Location(loc) = &mut self.content {
            loc.name = Some(name.into());
        }
        self
    }

    /// Adds a physical address to the location.
    ///
    /// # Behavior
    /// This method is a no-op if the current content is not already a [`Content::Location`].
    /// You must call [`location`](Self::location) first.
    pub fn location_address(mut self, address: impl Into<String>) -> Self {
        if let Content::Location(loc) = &mut self.content {
            loc.address = Some(address.into());
        }
        self
    }

    /// Returns the number of buttons currently configured in the draft.
    ///
    /// This is useful for validation logic, as WhatsApp enforces limits on button counts
    /// (e.g., maximum of 3 Reply Buttons).
    ///
    /// # Returns
    /// * The count of buttons if the message is interactive and contains a keyboard or CTA.
    /// * `0` if the message is text, media, or a list/catalog.
    pub fn count_buttons(&self) -> usize {
        if let Content::Interactive(InteractiveContent::Message(msg)) = &self.content {
            match &msg.action {
                InteractiveAction::Keyboard(keyboard) => keyboard.buttons.len(),
                // A CTA URL is technically 1 button
                InteractiveAction::Cta(_) => 1,
                // Lists, Catalogs, etc. are not "Buttons" in the Reply Button sense
                _ => 0,
            }
        } else {
            0
        }
    }

    /// Checks if the current draft is an Interactive message.
    pub fn is_interactive(&self) -> bool {
        matches!(self.content, Content::Interactive(_))
    }

    /// Checks if the current draft contains Media (Image, Video, Doc, etc.).
    pub fn has_media(&self) -> bool {
        matches!(self.content, Content::Media(_))
        // Or if it's an interactive message with a media header:
        || if let Content::Interactive(InteractiveContent::Message(msg)) = &self.content {
             matches!(msg.header, Some(InteractiveHeader::Media(_)))
           } else {
             false
           }
    }

    /// Prepares the draft message to be sent to a specific recipient.
    ///
    /// This method initiates the sending process, returning a [`SendMessage`] builder.
    /// The message is actually sent when the `SendMessage` builder is `.await`ed.
    ///
    /// This method is intended to be called on a `Draft` that has been created
    /// via a [`Client::message()`] manager.
    ///
    /// # Arguments
    /// * `recipient` - The [`IdentityRef`] of the recipient (e.g., a user's phone number).
    /// * `sender` - The phone number ID sending the message.
    /// * `client` - The [`Client`] instance to use for sending.
    ///
    /// # Returns
    /// A `SendMessage` builder that can be `.await`ed to send the message.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::{Draft, Client, IdentityRef};
    ///
    /// # async fn example_send_draft(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let recipient = IdentityRef::user("+16012345678");
    /// let business_id = IdentityRef::business("1234567890"); // Your phone number ID sending
    ///
    /// let draft = Draft::text("This message is ready to send!");
    ///
    /// // The `send` method is part of the `SendMessage` builder, which is obtained
    /// // by calling `client.message(sender_business_id)`.
    /// client.message(&business_id)
    ///     .send(&recipient, draft)
    ///     .await?;
    ///
    /// println!("Message sent successfully!");
    /// # Ok(())
    /// # }
    /// ```
    /// [`SendMessage`]: crate::client::SendMessage
    /// [`Client::message()`]: crate::client::Client::message
    /// [`IdentityRef`]: crate::IdentityRef
    #[inline]
    pub fn send<'i, S, R>(self, sender: S, recipient: R, client: &Client) -> SendMessage<'i>
    where
        S: ToValue<'i, IdentityRef>,
        R: ToValue<'i, IdentityRef>,
    {
        client.message(sender).send(recipient, self)
    }

    /// (Internal) Promotes the draft's content to an `Interactive` variant if it isn't already.
    ///
    /// This is the core of the ergonomic builder. It handles the "mental conversion"
    /// by taking existing Text or Media content and placing it appropriately
    /// within a new `InteractiveMessage` structure.
    fn promote_to_interactive_mut(&mut self) -> &mut InteractiveMessage {
        let old_content = std::mem::take(&mut self.content);

        let new_interactive = match old_content {
            Content::Interactive(InteractiveContent::Message(interactive)) => interactive,
            Content::Interactive(InteractiveContent::Click(button)) => {
                InteractiveMessage::new(Keyboard::new([button]), "")
            }
            Content::Text(text) => {
                // Convert a text message into an interactive message's body.
                // Default to a Keyboard action, ready for buttons.
                InteractiveMessage::new(Keyboard::empty(), text)
            }
            Content::Media(media) => {
                // Convert a media message into an interactive message's header.
                // The body will be empty and must be set via the `.body()` method.
                InteractiveMessage::new(Keyboard::empty(), "").header(media)
            }
            // For any other type, start with a fresh interactive message.
            _ => InteractiveMessage::new(Keyboard::empty(), ""),
        };

        self.content = Content::Interactive(new_interactive.into());

        // This unwrap is safe because we just guaranteed the variant is Interactive.
        if let Content::Interactive(InteractiveContent::Message(interactive)) = &mut self.content {
            interactive
        } else {
            unreachable!();
        }
    }
}

/// Contains metadata about a draft's relationship to other messages,
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug, Default)]
#[non_exhaustive]
pub struct DraftContext {
    /// An optional reference to the message this message is replying to.
    ///   
    /// Setting this might make it impossible to send this draft to more
    /// than one recipient.
    #[serde(flatten, skip_serializing_if = "Option::is_none", default)]
    pub replied_to: Option<MessageRef>,
}

/// Metadata from sent messages
///
/// Contains information returned by WhatsApp after sending a message.
#[derive(Deserialize, Clone, Debug)]
#[non_exhaustive]
pub struct MessageCreate {
    pub message: MessageRef,

    /// Initial delivery status
    pub message_status: Option<MessageStatus>,
}

impl MessageCreate {
    /// If message is in transit within WhatsApp systems
    #[inline]
    pub fn is_accepted(&self) -> bool {
        matches!(self.message_status, Some(MessageStatus::Accepted))
    }

    /// If message is delivered to device
    #[inline]
    pub fn is_delivered(&self) -> bool {
        matches!(self.message_status, Some(MessageStatus::Delivered))
    }

    /// If message is read by recipient
    #[inline]
    pub fn is_read(&self) -> bool {
        matches!(self.message_status, Some(MessageStatus::Read))
    }

    /// If message failed to send
    #[inline]
    pub fn failed(&self) -> bool {
        matches!(self.message_status, Some(MessageStatus::Failed))
    }

    /// If message is sent to WhatsApp
    #[inline]
    pub fn is_sent(&self) -> bool {
        matches!(self.message_status, Some(MessageStatus::Sent))
    }

    /// If catalog item in message is unavailable
    #[inline]
    pub fn is_warning(&self) -> bool {
        matches!(self.message_status, Some(MessageStatus::Warning))
    }

    /// If message was deleted by sender
    #[inline]
    pub fn is_deleted(&self) -> bool {
        matches!(self.message_status, Some(MessageStatus::Deleted))
    }
}

impl Deref for MessageCreate {
    type Target = MessageRef;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

/// Message status types
///
/// Represents the delivery state of a message
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    /// Message in transit within WhatsApp systems
    Accepted,
    /// Message delivered to device
    Delivered,
    /// Message read by recipient
    Read,
    /// Message failed to send
    Failed,
    /// Message sent to WhatsApp
    Sent,
    /// Catalog item in message is unavailable
    Warning,
    /// Message was deleted by sender
    Deleted,
}

enum_traits! {
    |InteractiveAction|
    UrlButton => Cta,
    CatalogDisplayOptions => CatalogDisplay,
    OptionList => OptionList,
    ProductRef => ProductDisplay,
    ProductList => ProductList
    // Flow => Flow
    // Keyboard => Keyboard
}

impl<I: Into<Keyboard>> From<I> for InteractiveAction {
    #[inline]
    fn from(value: I) -> Self {
        InteractiveAction::Keyboard(value.into())
    }
}

enum_traits! {
    |Button|
    ReplyButton => Reply,
    UrlButton => Url,
    CallButton => Call,
    OptionButton => Option
}

use std::fmt::{Display, Formatter, Result as FmtResult};
use std::str::FromStr;

impl<T> PartialEq<T> for Text
where
    String: PartialEq<T>,
{
    fn eq(&self, other: &T) -> bool {
        self.body.eq(other)
    }
}

impl PartialEq for MessageRef {
    fn eq(&self, other: &Self) -> bool {
        self.message_id == other.message_id
    }
}

impl PartialEq<MessageCreate> for MessageRef {
    fn eq(&self, other: &MessageCreate) -> bool {
        self.eq(&other.message)
    }
}

impl PartialEq<MessageRef> for MessageCreate {
    fn eq(&self, other: &MessageRef) -> bool {
        self.message.eq(other)
    }
}

impl PartialEq<Message> for MessageRef {
    fn eq(&self, other: &Message) -> bool {
        self.message_id == other.id
    }
}

// For clicks
impl PartialEq<Button> for Content {
    fn eq(&self, other: &Button) -> bool {
        if let Self::Interactive(InteractiveContent::Click(button)) = self {
            button.eq(other)
        } else {
            false
        }
    }
}

impl<C> PartialEq<C> for Draft
where
    Content: PartialEq<C>,
{
    fn eq(&self, other: &C) -> bool {
        self.content.eq(other)
    }
}

impl<C> PartialEq<C> for Message
where
    Content: PartialEq<C>,
{
    fn eq(&self, other: &C) -> bool {
        self.content.eq(other)
    }
}

impl Display for Draft {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        self.content.fmt(f)
    }
}

impl Display for Message {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(
            f,
            "[{} → {}] {}",
            self.sender.phone_id, self.recipient.phone_id, self.content
        )
    }
}

impl Display for Content {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Content::Text(text) => write!(f, "📝 {}", text.body),
            Content::Reaction(reaction) => write!(f, "{}", reaction.emoji),
            Content::Media(media) => write!(
                f,
                "🖼️ {}",
                media.caption.as_ref().map_or("[Media]", |c| &c.body)
            ),
            Content::Location(loc) => {
                if let Some(name) = &loc.name {
                    write!(f, "📍 {} ({:.4}, {:.4})", name, loc.latitude, loc.longitude)
                } else {
                    write!(f, "📍 Location ({:.4}, {:.4})", loc.latitude, loc.longitude)
                }
            }
            Content::Interactive(interactive) => match interactive {
                InteractiveContent::Message(msg) => write!(f, "📱 Interactive: {}", msg.body.body),
                InteractiveContent::Click(button) => write!(f, "🖱️ Clicked: {button:?}"),
            },
            Content::Order(order) => write!(f, "🛒 Order: {}", order.note.body),
            Content::Error(err) => write!(f, "❗ Error: {err:?}"),
        }
    }
}

impl Display for Text {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.body)
    }
}

impl Display for MessageRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(
            f,
            "Ref[{:#?} → {:#?}:{}]",
            self.sender(),
            self.recipient(),
            self.message_id()
        )
    }
}

// Helper function to ensure doc list of
// impl IntoDraft is correct.
#[doc(hidden)]
pub fn assert_into_draft<T: IntoDraft>() {}

/// Conversion to draft messages
///
/// Allows flexible input types for sending messages. Implemented for:
/// - `impl Into<String>`, and [`Text`]: Convert to text draft
/// - [`Media`]
/// - [`Reaction`]
/// - [`Location`]
/// - [`InteractiveContent`]
/// - [`Order`]
/// - [`Content`]
/// - [`Message`]
/// - [`InteractiveMessage`]
/// - [`IncomingMessage`]
/// - [`Draft`][]: Pass-through
/// ```rust
/// # use whatsapp_business_rs::message::{
/// #   Text, Reaction, Location, InteractiveContent, Order,
/// #   Content, Message, Draft, InteractiveMessage, assert_into_draft
/// # };
/// # use whatsapp_business_rs::server::IncomingMessage;
/// # assert_into_draft::<String>();
/// # assert_into_draft::<Text>();
/// # assert_into_draft::<Reaction>();
/// # assert_into_draft::<Location>();
/// # assert_into_draft::<InteractiveContent>();
/// # assert_into_draft::<Order>();
/// # assert_into_draft::<Content>();
/// # assert_into_draft::<Message>();
/// # assert_into_draft::<IncomingMessage>();
/// # assert_into_draft::<Draft>();
/// # assert_into_draft::<InteractiveMessage>();
/// ```
/// [`IncomingMessage`]: crate::server::IncomingMessage
pub trait IntoDraft: Send {
    /// Convert into a message draft
    fn into_draft(self) -> Draft;
}

impl IntoDraft for Content {
    #[inline]
    fn into_draft(self) -> Draft {
        Draft {
            content: self,
            ..Default::default()
        }
    }
}

impl IntoDraft for Draft {
    #[inline]
    fn into_draft(self) -> Draft {
        self
    }
}

impl IntoDraft for Message {
    #[inline]
    fn into_draft(self) -> Draft {
        Draft {
            content: self.content,
            ..Default::default()
        }
    }
}

impl IntoDraft for InteractiveMessage {
    #[inline]
    fn into_draft(self) -> Draft {
        Draft {
            content: Content::Interactive(InteractiveContent::Message(self)),
            ..Default::default()
        }
    }
}

// Let's make it easier to create interactive action using mental model
// And knowing action and body are required
//
// We use full name for diagnostics
// Header => Body => Action => Footer.
impl<IHeader, IBody, IAction, IFooter> IntoDraft for (IHeader, IBody, IAction, IFooter)
where
    IHeader: Into<InteractiveHeader> + Send,
    IBody: Into<Text> + Send,
    IAction: Into<InteractiveAction> + Send,
    IFooter: Into<Text> + Send,
{
    #[inline]
    fn into_draft(self) -> Draft {
        let interactive = InteractiveMessage::new(self.2, self.1)
            .header(self.0)
            .footer(self.3);
        interactive.into_draft()
    }
}

// Body => Action => Footer.
impl<IBody, IAction, IFooter> IntoDraft for (IBody, IAction, IFooter)
where
    IBody: Into<Text> + Send,
    IAction: Into<InteractiveAction> + Send,
    IFooter: Into<Text> + Send,
{
    #[inline]
    fn into_draft(self) -> Draft {
        let interactive = InteractiveMessage::new(self.1, self.0).footer(self.2);
        interactive.into_draft()
    }
}

// Body => Action.. the way it's displayed
impl<IBody, IAction> IntoDraft for (IBody, IAction)
where
    IBody: Into<Text> + Send,
    IAction: Into<InteractiveAction> + Send,
{
    #[inline]
    fn into_draft(self) -> Draft {
        let interactive = InteractiveMessage::new(self.1, self.0);
        interactive.into_draft()
    }
}

impl ToValue<'_, MessageRef> for Message {
    #[inline]
    fn to_value(self) -> Cow<'static, MessageRef> {
        Cow::Owned(MessageRef {
            message_id: self.id,
            // If they're moving, we should too.
            sender: Some(self.sender.to_value().into_owned()),
            recipient: Some(self.recipient.to_value().into_owned()),
        })
    }
}

// TOVIEW
impl ToValue<'_, MessageRef> for &Message {
    #[inline]
    fn to_value(self) -> Cow<'static, MessageRef> {
        Cow::Owned(self.as_ref())
    }
}

impl ToValue<'_, MessageRef> for MessageRef {
    #[inline]
    fn to_value(self) -> Cow<'static, MessageRef> {
        Cow::Owned(self)
    }
}

impl<'m> ToValue<'m, MessageRef> for &'m MessageRef {
    #[inline]
    fn to_value(self) -> Cow<'m, MessageRef> {
        Cow::Borrowed(self)
    }
}

impl<'m> ToValue<'m, MessageRef> for &'m MessageCreate {
    #[inline]
    fn to_value(self) -> Cow<'m, MessageRef> {
        Cow::Borrowed(&self.message)
    }
}

impl ToValue<'_, MessageRef> for MessageCreate {
    #[inline]
    fn to_value(self) -> Cow<'static, MessageRef> {
        Cow::Owned(self.message)
    }
}
