//! This module handles all API requests related to media management and the construction
//! of message payloads that include media. It covers uploading, downloading, deleting,
//! and retrieving media information, as well as preparing media content for various
//! message types like images, documents, and interactive message headers.

use std::{borrow::Cow, fmt::Display, marker::PhantomData, mem::transmute, str::FromStr};

use super::{
    FromResponse, FromResponseOwned, IntoMessageRequest, IntoMessageRequestOutput,
    execute_multipart_request,
};
use crate::{
    IdentityRef, MetaError,
    app::WebhookConfig,
    catalog::{MetaProductRef, ProductCreate, ProductRef},
    client::{
        Auth, Client, DeleteMedia, GetMediaInfo, MessageManager, PendingRequest, UploadMedia,
        UploadMediaResponseReference,
    },
    error::{Error, ServiceError, ServiceErrorKind},
    message::{
        CatalogDisplayOptions, Draft, DraftContext, InteractiveAction, InteractiveHeaderMedia,
        Media, MediaSource, MediaType, MessageCreate, MessageRef, MessageStatus, Text,
    },
    waba::AppInfo,
};
use reqwest::{
    Response, StatusCode, Url,
    header::AUTHORIZATION,
    multipart::{Form, Part},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::io::AsyncWrite;

#[cfg(feature = "batch")]
use crate::client::{NullableGetMediaInfoHandler, NullableUploadMediaHandler};

// --- Core Media Operations ---

impl Client {
    /// Builds a request to get media information, such as its public URL.
    pub(crate) fn get_media_info(&self, media_id: &str) -> GetMediaInfo {
        GetMediaInfo {
            request: self.get(self.a_node(media_id)),
        }
    }

    /// Builds a request to download media content from WhatsApp servers.
    /// This first retrieves the media's public URL and then downloads the content.
    pub(crate) fn download_media<'dst, Dst>(
        &self,
        media_id: &str,
        dst: &'dst mut Dst,
    ) -> DownloadMedia<'dst, Dst> {
        DownloadMedia {
            media_info_request: self.get_media_info(media_id),
            destination: dst,
        }
    }

    /// Builds a request to download media directly from a known URL.
    pub(crate) fn download_media_from_url<'dst, Dst>(
        &self,
        url: &str,
        dst: &'dst mut Dst,
    ) -> DownloadMediaFromUrl<'dst, Dst> {
        DownloadMediaFromUrl {
            request: self.get_external(url),
            destination: dst,
        }
    }

    /// Builds a request to delete media from WhatsApp servers.
    pub(crate) fn delete_media(&self, media_id: &str) -> DeleteMedia {
        DeleteMedia {
            request: self.delete(self.a_node(media_id)),
        }
    }
}

/// A pending request to download media directly from a URL.
/// It doesn't expose `with_auth` to prevent leaking the client's auth token
/// to potentially external servers.
#[derive(Debug)]
pub(crate) struct DownloadMediaFromUrl<'dst, Dst> {
    request: PendingRequest<'static>,
    destination: &'dst mut Dst,
}

impl<'dst, Dst: AsyncWrite + Send + Unpin> DownloadMediaFromUrl<'dst, Dst> {
    pub(crate) async fn execute(self) -> Result<(), Error> {
        #[cfg(debug_assertions)]
        let endpoint = self.request.endpoint.clone();
        let response = self.request.send().await?;

        stream_response_body_to_writer(
            response,
            self.destination,
            #[cfg(debug_assertions)]
            endpoint.into(),
        )
        .await
    }
}

/// Helper function to stream an HTTP response body into an async writer.
async fn stream_response_body_to_writer(
    mut response: Response,
    dst: &mut (impl AsyncWrite + Send + Unpin),
    #[cfg(debug_assertions)] endpoint: Cow<'_, str>,
) -> Result<(), Error> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.bytes().await?;
        return Err(Client::handle_error_response(&body)
            .service(
                #[cfg(debug_assertions)]
                endpoint,
                status,
            )
            .into());
    }

    use tokio::io::AsyncWriteExt as _;

    while let Some(chunk) = response.chunk().await? {
        dst.write_all(&chunk).await?;
    }
    dst.flush().await?;

    Ok(())
}

/// A pending request to download media by its ID.
/// This is a two-step operation: get media info (URL), then download from the URL.
pub(crate) struct DownloadMedia<'dst, Dst> {
    media_info_request: GetMediaInfo,
    destination: &'dst mut Dst,
}

impl<Dst> DownloadMedia<'_, Dst> {
    #[inline]
    pub(crate) fn with_auth(mut self, auth: &Auth) -> Self {
        self.media_info_request = self.media_info_request.with_auth(auth);
        self
    }
}

impl<'dst, Dst: AsyncWrite + Send + Unpin> DownloadMedia<'dst, Dst> {
    pub(crate) async fn execute(self) -> Result<(), Error> {
        let auth = self.media_info_request.request.auth.clone();
        let client = self.media_info_request.request.client.clone();

        // Step 1: Get the media info, which contains the download URL.
        let media_info = self.media_info_request.execute().await?;

        #[cfg(debug_assertions)]
        let endpoint = media_info.url.as_str().to_owned();

        // Step 2: Create and send a request to the URL from the media info.
        let mut download_request = client.get(media_info.url);
        if let Some(auth) = auth {
            download_request = match auth {
                Cow::Borrowed(auth) => download_request.header(AUTHORIZATION, auth),
                Cow::Owned(auth) => download_request.header(AUTHORIZATION, auth),
            };
        }

        let response = download_request.send().await?;
        stream_response_body_to_writer(
            response,
            self.destination,
            #[cfg(debug_assertions)]
            Cow::Owned(endpoint),
        )
        .await
    }
}

// --- Media Payload Construction ---

impl MessageManager<'_> {
    /// Prepares a media upload operation.
    pub(crate) fn prepare_media_upload(
        &self,
        bytes: Vec<u8>,
        mime_type: Cow<'static, str>,
        filename: impl Into<Cow<'static, str>>,
    ) -> UploadMedia {
        let url = self.endpoint("media");
        let part = Part::bytes(bytes)
            .file_name(filename)
            .mime_str(&mime_type)
            .expect("BUG: mime_type should be valid");

        UploadMedia {
            request: self.client.post(url),
            media: part,
        }
    }

    /// Creates a `MediaPayloadSource` which is a crucial enum that determines
    /// whether media needs to be uploaded or can be referenced directly by ID or URL.
    #[inline]
    pub(crate) fn create_media_payload_source(
        &self,
        media_source: MediaSource,
        media_type: MediaType,
        filename: impl Into<Cow<'static, str>>,
    ) -> MediaPayloadSource {
        match media_source {
            MediaSource::Bytes(bytes) => MediaPayloadSource::FromBytes(Box::new(
                self.prepare_media_upload(bytes, media_type.mime_type(), filename),
            )),
            MediaSource::Link(url) => MediaPayloadSource::FromRef(MediaRef::Link(url)),
            MediaSource::Id(id) => MediaPayloadSource::FromRef(MediaRef::Id(id)),
        }
    }
}

/// Represents the source of media for an outgoing message.
/// It can be a reference to existing media (by ID or a public link) or
/// new media that must be uploaded first. This enum abstracts that logic.
pub(crate) enum MediaPayloadSource {
    /// Media to be uploaded from raw bytes. The `UploadMedia` request will be
    /// executed first, and its resulting ID will be used in the message.
    FromBytes(Box<UploadMedia>),
    /// A reference to media that already exists, either on WhatsApp's servers (Id)
    /// or at a public URL (Link).
    FromRef(MediaRef),
}

impl IntoMessageRequestOutput for UploadMedia {
    type Request = String; // The request output is the media ID string.

    #[inline]
    fn with_auth(mut self, auth: Cow<'_, Auth>) -> Self {
        self.request.auth = Some(Cow::Owned(auth.into_owned()));
        self
    }

    #[inline]
    async fn execute(self) -> Result<Self::Request, Error> {
        let form = Form::new()
            .text("messaging_product", "whatsapp")
            .part("file", self.media);

        #[cfg(debug_assertions)]
        let handle = async |endpoint: String, response| {
            Client::handle_response(response, endpoint.into()).await
        };

        #[cfg(not(debug_assertions))]
        let handle = async |response| Client::handle_response(response).await;

        let response: MediaUploadResponse =
            execute_multipart_request(self.request, form, handle).await?;

        Ok(response.id)
    }
}

/// Represents a reference to media, used after it has been uploaded or if it's external.
/// This struct is serialized directly into message payloads.
/// `NOTE`: Should be flattened when used
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MediaRef {
    /// An ID of media already uploaded to WhatsApp.
    Id(String),
    /// A public URL pointing to the media.
    Link(String),
}

impl IntoMessageRequestOutput for MediaPayloadSource {
    type Request = MediaRef;

    #[inline]
    fn with_auth(self, auth: Cow<'_, Auth>) -> Self {
        match self {
            Self::FromBytes(mut upload_media) => {
                *upload_media = upload_media.with_auth(auth);
                Self::FromBytes(upload_media)
            }
            Self::FromRef(_) => self, // No auth needed for a simple reference.
        }
    }

    #[inline]
    async fn execute(self) -> Result<Self::Request, Error> {
        match self {
            MediaPayloadSource::FromBytes(upload_media) => {
                let id = upload_media.execute().await?;
                Ok(MediaRef::Id(id))
            }
            MediaPayloadSource::FromRef(media_ref) => Ok(media_ref),
        }
    }
}

/// The final, serializable representation of media content in a message.
#[derive(Serialize, Clone, Debug)]
pub(crate) struct MediaPayload {
    #[serde(flatten)]
    pub(crate) media: MediaRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) filename: Option<String>,
}

/// An intermediate builder for a `MediaPayload`. It holds the `MediaPayloadSource`
/// which will be executed to get the final `MediaRef`.
pub(crate) struct PendingMediaPayload {
    pub(crate) source: MediaPayloadSource,
    pub(crate) caption: Option<String>,
    pub(crate) filename: Option<String>,
}

impl IntoMessageRequestOutput for PendingMediaPayload {
    type Request = MediaPayload;

    #[inline]
    fn with_auth(mut self, auth: Cow<'_, Auth>) -> Self {
        self.source = self.source.with_auth(auth);
        self
    }

    #[inline]
    async fn execute(self) -> Result<Self::Request, Error> {
        Ok(Self::Request {
            media: self.source.execute().await?,
            caption: self.caption,
            filename: self.filename,
        })
    }
}

impl Media {
    /// Provides a default filename. The API sometimes returns an "unknown error"
    /// if a filename is missing for certain media types.
    pub(crate) fn default_filename() -> &'static str {
        // A generic extension is used as the actual file type is determined by mime_type.
        "file.bin"
    }
}

impl IntoMessageRequest for Media {
    type Output = PendingMediaPayload;

    #[inline]
    fn into_request<'i, 't>(
        self,
        manager: &MessageManager<'i>,
        _to: &'t IdentityRef,
    ) -> Self::Output {
        // Ensure a filename is present if not provided by the user.
        let filename = self
            .filename
            .clone()
            .map(Cow::Owned)
            .unwrap_or_else(|| Cow::Borrowed(Self::default_filename()));

        Self::Output {
            source: manager.create_media_payload_source(
                self.media_source,
                self.media_type,
                filename,
            ),
            caption: self.caption.map(|c| c.body),
            filename: self.filename,
        }
    }
}

// --- Interactive Media Header Handling ---

/// A final, serializable payload for an interactive message's media header.
#[derive(Serialize, Clone, Debug)]
pub(crate) struct InteractiveMediaHeaderPayload {
    pub(crate) link: String,
}

/// A staged, two-step operation required for interactive media headers.
/// The WhatsApp API requires a public URL for header media, not a media ID.
/// This struct encapsulates the process:
/// 1. Upload the media to get an ID.
/// 2. Use the ID to get media info, which contains the public URL.
struct StagedInteractiveHeader {
    client: Client,
    upload_media: UploadMedia,
}

impl StagedInteractiveHeader {
    fn with_auth(mut self, auth: Cow<'_, Auth>) -> Self {
        self.upload_media.request.auth = Some(Cow::Owned(auth.into_owned()));
        self
    }

    #[inline]
    async fn execute(self) -> Result<String, Error> {
        // Note: We could batch this but it'd be too much work
        // for nothing
        let auth = self.upload_media.request.auth.clone();

        // Step 1: Upload media to get an ID.
        let media_id = self.upload_media.execute().await?;

        // Step 2: Use the ID to get the media info containing the public URL.
        let mut get_info_request = self.client.get_media_info(&media_id);
        if let Some(auth_token) = auth {
            get_info_request = get_info_request.with_auth(auth_token);
        }
        let info = get_info_request.execute().await?;
        Ok(info.url.into())
    }
}

/// An intermediate builder for an interactive media header. It resolves the media source
/// into a public URL, performing an upload and info-fetch if necessary.
#[allow(clippy::enum_variant_names)]
pub(crate) enum PendingInteractiveMediaHeader {
    /// Media that needs to be uploaded, then its info fetched.
    FromUpload(Box<StagedInteractiveHeader>),
    /// Media that already has an ID, so its info just needs to be fetched.
    FromId(Box<GetMediaInfo>),
    /// A direct public link, which can be used as-is.
    FromLink(String),
}

impl IntoMessageRequestOutput for PendingInteractiveMediaHeader {
    type Request = InteractiveMediaHeaderPayload;

    #[inline]
    fn with_auth(self, auth: Cow<'_, Auth>) -> Self {
        match self {
            Self::FromUpload(mut staged_op) => {
                *staged_op = staged_op.with_auth(auth);
                Self::FromUpload(staged_op)
            }
            Self::FromId(mut get_info) => {
                *get_info = get_info.with_auth(auth);
                Self::FromId(get_info)
            }
            Self::FromLink(_) => self,
        }
    }

    #[inline]
    async fn execute(self) -> Result<Self::Request, Error> {
        let link = match self {
            Self::FromUpload(staged_op) => staged_op.execute().await?,
            Self::FromId(get_info) => get_info.execute().await?.url.into(),
            Self::FromLink(link) => link,
        };
        Ok(Self::Request { link })
    }
}

impl IntoMessageRequest for InteractiveHeaderMedia {
    type Output = PendingInteractiveMediaHeader;

    #[inline]
    fn into_request<'i, 't>(
        self,
        manager: &MessageManager<'i>,
        _to: &'t IdentityRef,
    ) -> Self::Output {
        // Interactive headers don't support custom filenames, so we use a default.
        let filename = Cow::Borrowed(Media::default_filename());
        match self.media_source {
            MediaSource::Bytes(bytes) => {
                let upload_media =
                    manager.prepare_media_upload(bytes, self.media_type.mime_type(), filename);

                let staged_op = StagedInteractiveHeader {
                    client: manager.client.clone(),
                    upload_media,
                };
                Self::Output::FromUpload(Box::new(staged_op))
            }
            MediaSource::Id(id) => {
                Self::Output::FromId(Box::new(manager.client.get_media_info(&id)))
            }
            MediaSource::Link(url) => Self::Output::FromLink(url),
        }
    }
}

pub(crate) type ContentPayload = crate::message::ContentRequest;
pub(crate) type PendingContentPayload = crate::message::ContentOutput;
pub(crate) type ContentResponse<'a> = crate::message::ContentResponse<'a>;

impl Draft {
    #[inline]
    pub(crate) fn into_request<'i, 't>(
        self,
        manager: &MessageManager<'i>,
        to: Cow<'t, IdentityRef>,
    ) -> PendingMessagePayload<'t> {
        PendingMessagePayload {
            messaging_product: "whatsapp",
            biz_opaque_callback_data: self.biz_opaque_callback_data,
            content: self.content.into_request(manager, &to),
            context: self.context,
            to,
        }
    }
}

pub(crate) struct PendingMessagePayload<'t> {
    messaging_product: &'static str,
    biz_opaque_callback_data: Option<String>,
    to: Cow<'t, IdentityRef>,
    context: DraftContext,
    content: PendingContentPayload,
}

impl<'t> IntoMessageRequestOutput for PendingMessagePayload<'t> {
    type Request = MessagePayload<'t>;

    #[inline]
    fn with_auth(mut self, auth: Cow<'_, Auth>) -> Self {
        self.content = self.content.with_auth(auth);
        self
    }

    #[inline]
    async fn execute(self) -> Result<Self::Request, Error> {
        Ok(Self::Request {
            messaging_product: self.messaging_product,
            biz_opaque_callback_data: self.biz_opaque_callback_data,
            to: self.to,
            context: self.context,
            content: self.content.execute().await?,
        })
    }
}

/// Top-level message request structure
#[derive(Serialize, Debug)]
pub(crate) struct MessagePayload<'t> {
    messaging_product: &'static str,

    #[serde(skip_serializing_if = "Option::is_none")]
    biz_opaque_callback_data: Option<String>,

    #[serde(flatten)]
    to: Cow<'t, IdentityRef>,

    #[serde(skip_serializing_if = "DraftContext::is_empty")]
    context: DraftContext,

    #[serde(flatten)]
    content: ContentPayload,
}

impl DraftContext {
    /// Check if context is empty (all fields None)
    fn is_empty(&self) -> bool {
        self.replied_to.is_none()
    }
}

impl<'de> Deserialize<'de> for MediaType {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mime_type = <Cow<'_, str> as Deserialize>::deserialize(deserializer)?;
        mime_type.parse().map_err(|err| {
            <D::Error as serde::de::Error>::custom(format!("parsing mime_type: {err}"))
        })
    }
}

/// Message status update request
#[derive(Serialize, Debug)]
pub(crate) struct MessageStatusRequest<'a> {
    messaging_product: &'static str,
    status: MessageStatus,
    message_id: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    typing_indicator: Option<TypingIndicator>,
}

/// Typing indicator for message status
#[derive(Serialize, Debug)]
pub(crate) struct TypingIndicator {
    r#type: TypingIndicatorType,
}

/// Types of typing indicators
#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TypingIndicatorType {
    Text,
}

impl<'a> MessageStatusRequest<'a> {
    /// Create typing indicator request
    pub(crate) fn set_typing(message: Cow<'a, MessageRef>) -> Self {
        MessageStatusRequest {
            messaging_product: "whatsapp",
            status: MessageStatus::Read,
            message_id: match message {
                Cow::Borrowed(message) => Cow::Borrowed(&message.message_id),
                Cow::Owned(message) => Cow::Owned(message.message_id),
            },
            typing_indicator: Some(TypingIndicator {
                r#type: TypingIndicatorType::Text,
            }),
        }
    }

    /// Create read receipt request
    pub(crate) fn set_read(message: Cow<'a, MessageRef>) -> Self {
        MessageStatusRequest {
            messaging_product: "whatsapp",
            status: MessageStatus::Read,
            message_id: match message {
                Cow::Borrowed(message) => Cow::Borrowed(&message.message_id),
                Cow::Owned(message) => Cow::Owned(message.message_id),
            },
            typing_indicator: None,
        }
    }
}

// ---- Batch impls -----

#[cfg(feature = "batch")]
mod batch {
    use super::*;
    use crate::batch::{
        BatchResponse, BatchSerializer, FormatError, Handler, Nullable, Requests,
        ResponseProcessingError,
    };

    impl Requests for UploadMedia {
        type BatchHandler = UploadMediaHandler;

        type ResponseReference = UploadMediaResponseReference;

        #[inline]
        fn into_batch_ref(
            self,
            batch_serializer: &mut BatchSerializer,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            #[derive(Serialize)]
            struct Payload {
                messaging_product: &'static str,
            }

            #[cfg(debug_assertions)]
            let endpoint = self.request.endpoint.clone();

            let mut request = batch_serializer
                .format_request(self.request)
                .attach_file(None, self.media)
                .json_object_body(Payload {
                    messaging_product: "whatsapp",
                });

            let name = request.get_name();
            let reference = reference!(name => MediaUploadResponse => [id]);
            request.finish()?;

            Ok((
                UploadMediaHandler {
                    #[cfg(debug_assertions)]
                    endpoint: endpoint.into(),
                },
                UploadMediaResponseReference(reference),
            ))
        }

        requests_batch_include! {}
    }

    #[derive(Debug)]
    pub struct UploadMediaHandler {
        #[cfg(debug_assertions)]
        endpoint: Cow<'static, str>,
    }

    impl Handler for UploadMediaHandler {
        type Responses = Result<String, Error>;

        #[inline]
        fn from_batch(
            self,
            response: &mut BatchResponse,
        ) -> Result<Self::Responses, ResponseProcessingError> {
            Ok(response
                .try_next::<MediaUploadResponse>(
                    #[cfg(debug_assertions)]
                    self.endpoint,
                )?
                .map(|r| r.id))
        }
    }

    impl Requests for MediaPayloadSource {
        type BatchHandler = Option<NullableUploadMediaHandler>;

        type ResponseReference = <Self as IntoMessageRequestOutput>::Request;

        #[inline]
        fn into_batch_ref(
            self,
            f: &mut BatchSerializer,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            match self {
                Self::FromBytes(upload_media) => {
                    use Nullable as _;

                    let (handler, id) = upload_media.nullable().into_batch_ref(f)?;
                    Ok((Some(handler), MediaRef::Id(id.0)))
                }
                Self::FromRef(media_ref) => Ok((None, media_ref)),
            }
        }

        #[inline]
        fn size_hint(&self) -> (usize, Option<usize>) {
            if let Self::FromBytes(upload_media) = &self {
                upload_media.size_hint()
            } else {
                (0, Some(0))
            }
        }

        requests_batch_include! {}
    }

    impl Requests for PendingMediaPayload {
        type BatchHandler = <MediaPayloadSource as Requests>::BatchHandler;

        type ResponseReference = <Self as IntoMessageRequestOutput>::Request;

        #[inline]
        fn into_batch_ref(
            self,
            f: &mut BatchSerializer,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            let (h, r) = self.source.into_batch_ref(f)?;

            let request = Self::ResponseReference {
                media: r,
                caption: self.caption,
                filename: self.filename,
            };

            Ok((h, request))
        }

        #[inline]
        fn size_hint(&self) -> (usize, Option<usize>) {
            self.source.size_hint()
        }

        requests_batch_include! {}
    }

    impl StagedInteractiveHeader {
        fn into_batch_ref(
            self,
            batch_serializer: &mut BatchSerializer,
        ) -> Result<
            (
                (NullableUploadMediaHandler, NullableGetMediaInfoHandler),
                String,
            ),
            FormatError,
        > {
            // I'm not sure if depending on a request gives the dependent request
            // the auth. We'd just go from then_nullable into the batch

            use {Nullable as _, Requests as _};
            let auth = self.upload_media.request.auth.clone();

            let (upload_media_handler, upload_media_ref) = self
                .upload_media
                .nullable()
                .into_batch_ref(batch_serializer)?;
            let media_id_ref = upload_media_ref.0;

            let mut get_media_info = self.client.get_media_info(&media_id_ref);
            if let Some(auth) = auth {
                get_media_info = get_media_info.with_auth(auth)
            }

            let (get_media_info_handler, media_info_ref) =
                get_media_info.nullable().into_batch_ref(batch_serializer)?;

            let url_ref = media_info_ref.url();

            Ok(((upload_media_handler, get_media_info_handler), url_ref))
        }
    }

    impl Requests for PendingInteractiveMediaHeader {
        type BatchHandler = PendingInteractiveMediaHeaderBatchHandler;

        type ResponseReference = <Self as IntoMessageRequestOutput>::Request;

        fn into_batch_ref(
            self,
            batch_serializer: &mut BatchSerializer,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            // Won't work but for completeness
            let (handler, link) = match self {
                Self::FromUpload(upload_media) => {
                    let (handlers, url_ref) = upload_media.into_batch_ref(batch_serializer)?;

                    (
                        PendingInteractiveMediaHeaderBatchHandler::FromUpload(handlers),
                        url_ref,
                    )
                }
                Self::FromId(get_media_info) => {
                    use Nullable as _;

                    let (handler, url_ref) =
                        get_media_info.nullable().into_batch_ref(batch_serializer)?;
                    (
                        PendingInteractiveMediaHeaderBatchHandler::FromId(handler),
                        url_ref.url(),
                    )
                }
                Self::FromLink(link) => (PendingInteractiveMediaHeaderBatchHandler::FromLink, link),
            };

            Ok((handler, Self::ResponseReference { link }))
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            match self {
                Self::FromUpload(_) => (2, None),
                Self::FromId(_) => (1, Some(1)),
                Self::FromLink(_) => (0, Some(0)),
            }
        }

        requests_batch_include! {}
    }

    #[allow(clippy::enum_variant_names)]
    pub(crate) enum PendingInteractiveMediaHeaderBatchHandler {
        FromUpload((NullableUploadMediaHandler, NullableGetMediaInfoHandler)),
        FromId(NullableGetMediaInfoHandler),
        FromLink,
    }

    impl Handler for PendingInteractiveMediaHeaderBatchHandler {
        type Responses = ();

        fn from_batch(
            self,
            response: &mut BatchResponse,
        ) -> Result<Self::Responses, ResponseProcessingError> {
            match self {
                Self::FromUpload((first, second)) => {
                    first.from_batch(response)?;
                    second.from_batch(response)?;
                    Ok(())
                }
                Self::FromId(id) => {
                    id.from_batch(response)?;
                    Ok(())
                }
                Self::FromLink => Ok(()),
            }
        }
    }

    impl<'t> Requests for PendingMessagePayload<'t> {
        type BatchHandler = <PendingContentPayload as Requests>::BatchHandler;

        type ResponseReference = <Self as IntoMessageRequestOutput>::Request;

        #[inline]
        fn into_batch_ref(
            self,
            f: &mut BatchSerializer,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            let (h, r) = self.content.into_batch_ref(f)?;

            let request = Self::ResponseReference {
                messaging_product: self.messaging_product,
                biz_opaque_callback_data: self.biz_opaque_callback_data,
                to: self.to,
                context: self.context,
                content: r,
            };

            Ok((h, request))
        }

        #[inline]
        fn size_hint(&self) -> (usize, Option<usize>) {
            self.content.size_hint()
        }

        requests_batch_include! {}
    }
}

// --- API Response and Helper Structs ---
#[inline]
pub(crate) fn deserialize_url_from_string<'de, D>(d: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = <Cow<'_, str>>::deserialize(d)?;
    raw.parse().map_err(|err| {
        <D::Error as serde::de::Error>::custom(format!("Invalid URL '{raw}': {err}"))
    })
}

/// Media upload API response.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct MediaUploadResponse {
    id: String,
}

// ===== Unified text de/serialization ==== //
// Text request structures
#[derive(Deserialize, Serialize, Debug)]
pub(crate) struct TextRequest {
    pub(crate) body: String,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) preview_url: Option<bool>,
}

#[derive(Deserialize, Serialize, Debug)]
pub(crate) struct TextRequestText {
    pub(crate) text: String,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) preview_url: Option<bool>,
}

type OrdinaryText = String;

#[inline]
pub(crate) fn serialize_text_text<S: Serializer>(v: &Text, ser: S) -> Result<S::Ok, S::Error> {
    let text: &TextRequestText = unsafe { transmute(v) };
    text.serialize(ser)
}

#[inline]
pub(crate) fn serialize_ordinary_text<S: Serializer>(v: &Text, ser: S) -> Result<S::Ok, S::Error> {
    let ordinary_text: &OrdinaryText = unsafe { transmute(&v.body) };
    ordinary_text.serialize(ser)
}

#[inline]
pub(crate) fn serialize_text_text_opt<S: Serializer>(
    v: &Option<Text>,
    ser: S,
) -> Result<S::Ok, S::Error> {
    match v {
        Some(v) => serialize_text_text(v, ser),
        None => ser.serialize_none(), // Should be unreachable
    }
}

// FIXME: Still can't benefit from AnyField
impl<'de> Deserialize<'de> for Text {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        /// Internal enum for deserializing various forms of a Text object.
        #[derive(Deserialize, Serialize, Debug)]
        #[serde(untagged)]
        enum AnyText {
            /// E.g., `{ "body": "Hello", "preview_url": false }`
            TextRequest(TextRequest),
            /// E.g., `{ "text": "Hello", "preview_url": false }`
            TextRequestText(TextRequestText),
            /// E.g., `"Hello"`
            OrdinaryText(OrdinaryText),
        }

        let any_text = AnyText::deserialize(deserializer)?;

        Ok(match any_text {
            AnyText::TextRequest(text_request) => unsafe {
                transmute::<TextRequest, Text>(text_request)
            },
            AnyText::TextRequestText(text_request_text) => unsafe {
                transmute::<TextRequestText, Text>(text_request_text)
            },
            AnyText::OrdinaryText(body) => Text {
                body,
                preview_url: None,
            },
        })
    }
}

/// A helper struct for serializing and deserializing `InteractiveAction`.
/// The API wraps the action object within a JSON object that has a `type` field
/// indicating the kind of action. This struct models that wrapper.
#[derive(Serialize, Deserialize, Debug)]
struct InteractiveActionWrapper<'a, I> {
    /// The type of interactive action, represented as a string.
    r#type: Cow<'a, str>,
    /// The action payload itself.
    action: I,
}

/// Serializes an `InteractiveAction` enum into the format expected by the API.
/// It wraps the action in an `InteractiveActionWrapper` with the correct `type` string.
#[inline]
pub(crate) fn serialize_interactive_action<S: Serializer>(
    action: &<<InteractiveAction as IntoMessageRequest>::Output as IntoMessageRequestOutput>::Request,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let type_str = match action {
        InteractiveAction::Keyboard(_) => "button",
        InteractiveAction::CatalogDisplay(_) => "catalog_message",
        InteractiveAction::LocationRequest => "location_request_message",
        InteractiveAction::Cta(_) => "cta_url",
        InteractiveAction::ProductList(_) => "product_list",
        InteractiveAction::ProductDisplay(_) => "product",
        InteractiveAction::OptionList(_) => "list",
        // InteractiveAction::Flow(_) => "flow",
    };

    InteractiveActionWrapper {
        r#type: Cow::Borrowed(type_str),
        action,
    }
    .serialize(serializer)
}

/// Deserializes an API response into an `InteractiveAction`.
/// This is primarily used for testing and internal validation, as the library
/// typically does not receive interactive actions from the API.
#[inline]
pub(crate) fn deserialize_interactive_action<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<<InteractiveAction as FromResponse<'de>>::Response, D::Error> {
    let request = InteractiveActionWrapper::<InteractiveAction>::deserialize(deserializer)?;
    Ok(request.action)
}

// --- Catalog Display Options Serialization ---

/// A serialization helper for `CatalogDisplayOptions`.
/// It maps the `thumbnail` field to the `thumbnail_product_retailer_id` field in the JSON payload.
#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct CatalogDisplayOptionsHelper<'a> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thumbnail_product_retailer_id: Option<Cow<'a, str>>,
}

impl Serialize for CatalogDisplayOptions {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let helper = CatalogDisplayOptionsHelper {
            thumbnail_product_retailer_id: self
                .thumbnail
                .as_ref()
                .map(|p| p.product_retailer_id().into()),
        };
        helper.serialize(serializer)
    }
}

impl<'a> Deserialize<'a> for CatalogDisplayOptions {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'a>,
    {
        let helper = CatalogDisplayOptionsHelper::deserialize(deserializer)?;
        Ok(Self {
            thumbnail: helper
                .thumbnail_product_retailer_id
                .map(ProductRef::from_product_retailer_id),
        })
    }
}

#[derive(Serialize, Debug)]
#[allow(clippy::owned_cow)] // str might cause reallocation
pub(crate) struct WebhookConfigRequest<'a> {
    /// The object type, always "whatsapp_business_account".
    object: &'static str,
    /// The URL to which webhook notifications will be sent.
    pub callback_url: Cow<'a, String>,
    /// A token used to verify the authenticity of webhook requests.
    pub verify_token: Cow<'a, String>,
}

impl<'a> From<Cow<'a, WebhookConfig>> for WebhookConfigRequest<'a> {
    /// Creates a `WebhookConfigRequest` from a `WebhookConfig`, borrowing data where possible.
    #[inline]
    fn from(config: Cow<'a, WebhookConfig>) -> Self {
        let (callback_url, verify_token) = match config {
            Cow::Borrowed(cfg) => (
                Cow::Borrowed(&cfg.webhook_url),
                Cow::Borrowed(&cfg.verify_token),
            ),
            Cow::Owned(cfg) => (Cow::Owned(cfg.webhook_url), Cow::Owned(cfg.verify_token)),
        };
        WebhookConfigRequest {
            object: "whatsapp_business_account",
            callback_url,
            verify_token,
        }
    }
}

/// Represents the possible grant types for an access token request.
/// Using an enum makes the API safer by preventing invalid string values.
#[derive(Serialize, Debug, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GrantType {
    /// Used for exchanging a short-lived token for a long-lived one.
    FbExchangeToken,
    /// Standard client credentials flow.
    #[default]
    ClientCredentials,
}

/// Represents the request body for obtaining an access token.
#[derive(Serialize, Debug, Default)]
pub(crate) struct AccessTokenRequest<'a> {
    /// The application's unique ID.
    pub(crate) client_id: &'a str,
    /// The application's secret key.
    pub(crate) client_secret: &'a str,
    /// The type of grant being requested.
    pub(crate) grant_type: GrantType,

    /// The authorization code from the OAuth flow.
    /// Required for `client_credentials` grant type.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub(crate) code: &'a str,

    /// The token to be exchanged for a long-lived token.
    /// Required for `fb_exchange_token` grant type.    
    #[serde(skip_serializing_if = "str::is_empty")]
    pub(crate) fb_exchange_token: &'a str,
}

/// Represents the request to register a phone number.
#[derive(Serialize)]
pub(crate) struct RegisterPhoneRequest<'a> {
    messaging_product: &'static str,
    pin: &'a str,
}

impl<'a> RegisterPhoneRequest<'a> {
    /// Creates a new request from a 6-digit PIN.
    #[inline]
    pub(crate) fn from_pin(pin: &'a str) -> Self {
        Self {
            messaging_product: "whatsapp",
            pin,
        }
    }
}

/// Request to share a credit line with another WhatsApp Business Account.
#[derive(Serialize, Debug)]
pub(crate) struct ShareCreditLineRequest<'a> {
    /// The ID of the target WhatsApp Business Account.
    pub(crate) waba_id: &'a str,
    /// The currency of the credit line.
    pub(crate) waba_currency: &'a str,
}

/// Response from a successful credit line sharing request.
#[derive(Deserialize, Debug)]
pub(crate) struct ShareCreditLineResponse {
    /// A unique ID for the allocation configuration. This can be used
    /// to verify that the credit line was shared successfully.
    pub(crate) allocation_config_id: String,
}

/// A generic wrapper for API responses that are nested under a `whatsapp_business_api_data` key.
#[derive(Deserialize, Debug)]
pub(crate) struct WhatsAppBusinessApiData<T> {
    whatsapp_business_api_data: T,
}

impl<'a> FromResponse<'a> for AppInfo {
    type Response = WhatsAppBusinessApiData<AppInfo>;

    fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind> {
        Ok(response.whatsapp_business_api_data)
    }
}

// --- Generic Serde String Parsing Helpers ---

/// An enum to help deserialize fields that can be either a raw string
/// or a fully structured type (like a number or boolean).
#[derive(Deserialize)]
#[serde(untagged)]
enum StringOr<'a, T> {
    String(Cow<'a, str>),
    Parsed(T),
}

/// A deserialization helper that can parse a value from a string or accept it directly.
/// For example, it can deserialize both `"123"` and `123` into an `i32`.
#[inline]
pub(crate) fn deserialize_str<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: FromStr + Deserialize<'de>,
    T::Err: Display,
    D: Deserializer<'de>,
{
    let v = <StringOr<T>>::deserialize(deserializer)?;
    match v {
        StringOr::String(s) => T::from_str(&s)
            .map_err(|err| <D::Error as serde::de::Error>::custom(format!("parsing value: {err}"))),
        StringOr::Parsed(val) => Ok(val),
    }
}

/// A serialization helper that converts a value to its string representation.
#[inline]
pub(crate) fn serialize_str<T, S>(v: &T, ser: S) -> Result<S::Ok, S::Error>
where
    T: ToString,
    S: Serializer,
{
    // FIXME: Inefficient
    ser.serialize_str(&v.to_string())
}

/// An optional version of `deserialize_from_string`.
#[inline]
pub(crate) fn deserialize_str_opt<'de, T, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error>
where
    T: FromStr + Deserialize<'de>,
    T::Err: Display,
{
    match <Option<StringOr<T>>>::deserialize(deserializer)? {
        Some(StringOr::String(s)) => T::from_str(&s)
            .map(Some)
            .map_err(|err| <D::Error as serde::de::Error>::custom(format!("parsing value: {err}"))),
        Some(StringOr::Parsed(val)) => Ok(Some(val)),
        None => Ok(None),
    }
}

impl Client {
    /// Asynchronously handles an API response, parsing success or error cases.
    #[inline]
    pub(crate) async fn handle_response<T: FromResponseOwned>(
        response: Response,
        #[cfg(debug_assertions)] endpoint: Cow<'_, str>,
    ) -> Result<T, Error> {
        let status = response.status();
        let body = response.bytes().await?;

        Self::handle_response_(
            status,
            &body,
            #[cfg(debug_assertions)]
            endpoint,
        )
    }

    /// Synchronously handles a response body, mapping it to a success type `T` or a structured `Error`.
    /// This is the core logic used by `handle_response
    #[inline]
    pub(crate) fn handle_response_<'de, T: FromResponse<'de>>(
        status: StatusCode,
        body: &'de [u8],
        #[cfg(debug_assertions)] endpoint: Cow<'_, str>,
    ) -> Result<T, Error> {
        if status.is_success() {
            match serde_json::from_slice(body) {
                Ok(response) => T::from_response(response).map_err(|err| {
                    err.service(
                        #[cfg(debug_assertions)]
                        endpoint,
                        status,
                    )
                    .into()
                }),
                Err(err) => Err(ServiceError::parse(
                    err.into(),
                    String::from_utf8_lossy(body).to_string(),
                )
                .service(
                    #[cfg(debug_assertions)]
                    endpoint,
                    status,
                )
                .into()),
            }
        } else {
            Err(Self::handle_error_response(body)
                .service(
                    #[cfg(debug_assertions)]
                    endpoint,
                    status,
                )
                .into())
        }
    }

    /// Parses an error response body from the API.
    fn handle_error_response(body: &[u8]) -> ServiceErrorKind {
        /// The API wraps its error object in a top-level `error` key.
        #[derive(Deserialize, Debug)]
        struct ApiErrorWrapper {
            error: MetaError,
        }
        match serde_json::from_slice::<ApiErrorWrapper>(body) {
            Ok(structured_error) => ServiceError::api(structured_error.error),
            Err(parse_error) => ServiceError::parse(
                parse_error.into(),
                String::from_utf8_lossy(body).to_string(),
            ),
        }
    }
}

// --- Specific Response `FromResponse` Implementations ---

/// Represents a simple success response, e.g., `{"success": true}`.
#[derive(Deserialize, Debug)]
pub(crate) struct SuccessStatus {
    pub(crate) success: bool,
}

impl<'a> FromResponse<'a> for () {
    type Response = SuccessStatus;

    #[inline]
    fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind> {
        if response.success {
            Ok(())
        } else {
            Err(ServiceError::payload(
                "Operation failed: API returned success=false".into(),
            ))
        }
    }
}

impl<'a> FromResponse<'a> for PhantomData<()> {
    type Response = <() as FromResponse<'a>>::Response;

    #[inline]
    fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind> {
        <() as FromResponse>::from_response(response).map(|_: ()| PhantomData)
    }
}

#[derive(Deserialize, Debug)]
pub(crate) struct SendMessageResponseContact {
    #[serde(default)]
    wa_id: Option<String>,
    // input: String,
}

#[derive(Deserialize, Debug)]
pub(crate) struct SendMessageResponseMetadata {
    pub(crate) id: String,
    #[serde(default)]
    message_status: Option<MessageStatus>,
}

/// Represents the raw API response after sending a message.
#[derive(Debug, Deserialize)]
pub(crate) struct SendMessageResponse {
    pub(crate) messages: Vec<SendMessageResponseMetadata>,
    #[serde(default)]
    pub(crate) contacts: Vec<SendMessageResponseContact>,
}

impl<'a> FromResponse<'a> for MessageCreate {
    type Response = SendMessageResponse;

    #[inline]
    fn from_response(mut response: Self::Response) -> Result<Self, ServiceErrorKind> {
        let metadata = response.messages.pop().ok_or_else(|| {
            ServiceError::payload(
                format!(
                    "Expected 1 message in response, but received {}",
                    response.messages.len()
                )
                .into(),
            )
        })?;

        let (message_id, message_status) = (metadata.id, metadata.message_status);

        let recipient = response
            .contacts
            .pop()
            .and_then(|contact| contact.wa_id.map(IdentityRef::user));

        Ok(MessageCreate {
            message: MessageRef {
                message_id,
                sender: None, // Sender info is not in the response
                recipient,
            },
            message_status,
        })
    }
}

// Not so sure about this... their doc is too obscure
// TODO: Confirm
// {
//   "id": "9876543210",
//   "retailer_id": "SKU-12345"
// }
#[derive(Debug, Deserialize)]
pub(crate) struct CreateProductResponse {
    pub id: String,
}

impl<'a> FromResponse<'a> for ProductCreate {
    type Response = CreateProductResponse;

    fn from_response(create_product_response: Self::Response) -> Result<Self, ServiceErrorKind> {
        Ok(Self {
            product: MetaProductRef::from_product_id(create_product_response.id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ProductRef;
    use crate::message::{DocumentExtension, ImageExtension, MediaType, OptionButton, Section};

    use serde_json::json;

    #[test]
    fn test_deserialize_from_string_helper() {
        #[derive(Deserialize, PartialEq, Debug)]
        struct TestStruct {
            #[serde(deserialize_with = "deserialize_str")]
            num: i32,
        }

        let json_from_string = r#"{"num": "123"}"#;
        let json_from_number = r#"{"num": 123}"#;

        let from_string: TestStruct = serde_json::from_str(json_from_string).unwrap();
        let from_number: TestStruct = serde_json::from_str(json_from_number).unwrap();

        assert_eq!(from_string.num, 123);
        assert_eq!(from_number.num, 123);
        assert_eq!(from_string, from_number);
    }

    #[test]
    fn test_serde_section_product_ref() {
        let section = Section {
            title: "My Products".to_string(),
            items: vec![ProductRef::from_product_retailer_id("prod1")],
        };

        let serialized = serde_json::to_string(&section).unwrap();
        let expected_json =
            r#"{"title":"My Products","product_items":[{"product_retailer_id":"prod1"}]}"#;
        assert_eq!(serialized, expected_json);

        let deserialized: Section<ProductRef> = serde_json::from_str(expected_json).unwrap();
        assert_eq!(deserialized.title, section.title);
        assert_eq!(deserialized.items.len(), 1);
    }

    #[test]
    fn test_serde_section_option_button() {
        let section = Section {
            title: "My Options".to_string(),
            items: vec![OptionButton::new("description", "Option 1", "opt1")],
        };

        let serialized = serde_json::to_string(&section).unwrap();
        let expected_json = r#"{"title":"My Options","rows":[{"description":"description","title":"Option 1","id":"opt1"}]}"#;
        assert_eq!(serialized, expected_json);

        let deserialized: Section<OptionButton> = serde_json::from_str(expected_json).unwrap();
        assert_eq!(deserialized.title, section.title);
        assert_eq!(deserialized.items.len(), 1);
    }

    #[test]
    fn test_handle_response_sync_success() {
        let body = br#"{"success": true}"#;
        let result: Result<(), Error> = Client::handle_response_(
            StatusCode::OK,
            body,
            #[cfg(debug_assertions)]
            "test".into(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_message_create_response() {
        let body = json!({
            "messages": [
                {
                    "id": "wamid.12345",
                    "message_status": "accepted"
                }
            ],
            "contacts": [
                {
                    "wa_id": "15551234567",
                    "input": "15551234567"
                }
            ]
        });

        let response: SendMessageResponse = serde_json::from_value(body).unwrap();
        let message_create = MessageCreate::from_response(response).unwrap();

        assert_eq!(message_create.message.message_id, "wamid.12345");
        assert_eq!(message_create.message_status, Some(MessageStatus::Accepted));
        assert_eq!(
            message_create.message.recipient.unwrap().phone_id(),
            "15551234567"
        );
    }

    #[test]
    fn test_handle_response_sync_api_error() {
        let body = br#"{
            "error": {
                "message": "Invalid parameter",
                "type": "OAuthException",
                "code": 100,
                "fbtrace_id": "Abc123Def456"
            }
        }"#;
        let result: Result<(), Error> = Client::handle_response_(
            StatusCode::BAD_REQUEST,
            body,
            #[cfg(debug_assertions)]
            "test".into(),
        );

        assert!(result.is_err());
        if let Error::Service(err) = result.unwrap_err() {
            assert_eq!(err.status, StatusCode::BAD_REQUEST);
            if let ServiceErrorKind::Api(api_err) = err.kind {
                assert_eq!(api_err.error.code, 100);
                assert_eq!(api_err.error.message.unwrap(), "Invalid parameter");
            } else {
                panic!("Expected API error variant");
            }
        } else {
            panic!("Expected Service error");
        }
    }

    #[test]
    fn media_type() {
        assert_eq!(
            serde_json::from_str::<MediaType>("\"application/pdf\"").unwrap(),
            MediaType::Document(DocumentExtension::Pdf)
        );

        assert_eq!(
            serde_json::from_str::<MediaType>("\"image/jpeg\"").unwrap(),
            MediaType::Image(ImageExtension::Jpeg)
        )
    }

    #[test]
    fn deserialize_str_test() {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        pub struct OrderProduct {
            #[serde(
                serialize_with = "serialize_str",
                deserialize_with = "deserialize_str::<usize, __D>"
            )]
            pub quantity: usize,
            #[serde(
                rename = "item_price",
                serialize_with = "serialize_str",
                deserialize_with = "deserialize_str::<f64, __D>"
            )]
            pub unit_price: f64,
        }

        serde_json::from_str::<OrderProduct>(
            r#"{
                "quantity": "3",
                "item_price": "20"
            }"#,
        )
        .unwrap();
    }
}
