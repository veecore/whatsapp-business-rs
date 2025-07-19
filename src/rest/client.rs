// DOUBLE-CHECK OUR ENDPOINT USE.

use std::{borrow::Cow, fmt::Display, marker::PhantomData, mem::transmute, str::FromStr};

use super::{fut_net_op, FromResponse, IntoMessageRequest, IntoMessageRequestOutput};
use crate::{
    app::WebhookConfig,
    catalog::{MetaProductRef, ProductCreate, ProductRef},
    client::{Auth, Client, Endpoint, MessageManager},
    error::{Error, ServiceError, ServiceErrorKind},
    handle_arg,
    message::{
        CatalogDisplayOptions, Draft, DraftContext, ErrorContent, InteractiveAction,
        InteractiveHeaderMedia, Media, MediaSource, MediaType, MessageCreate, MessageRef,
        MessageStatus, Text,
    },
    waba::AppInfo,
    IdentityRef, MetaError, SimpleOutput,
};
use reqwest::{
    header::CONTENT_TYPE,
    multipart::{Form, Part},
    RequestBuilder, Response, Url,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::io::AsyncWrite;

impl Client {
    /// Get media information. url, etc.
    pub(crate) fn get_media_info(&self, media_id: &str) -> GetMediaInfo {
        GetMediaInfo {
            request: self.get(self.a_node(media_id)),
        }
    }

    pub(crate) fn get_media_info_patch(&self) -> GetMediaInfoPatch {
        GetMediaInfoPatch {
            endpoint: self.endpoint(),
        }
    }

    /// Download media content from WhatsApp servers
    pub(crate) fn download_media<'dst, Dst>(
        &self,
        media_id: &str,
        dst: &'dst mut Dst,
    ) -> DownloadMedia<'dst, Dst> {
        DownloadMedia {
            media_info: self.get_media_info(media_id),
            dst,
        }
    }

    /// Download media from a direct URL
    pub(crate) fn download_media_url<'dst, Dst>(
        &self,
        url: &str,
        dst: &'dst mut Dst,
    ) -> DownloadMediaUrl<'dst, Dst> {
        DownloadMediaUrl {
            request: self.get_external(url),
            dst,
        }
    }

    /// Delete media from WhatsApp servers
    pub(crate) fn delete_media(&self, media_id: &str) -> DeleteMedia {
        DeleteMedia {
            request: self.delete(self.a_node(media_id)),
        }
    }
}

#[derive(Debug)]
pub(crate) struct GetMediaInfoPatch {
    endpoint: Endpoint<'static>,
}

impl GetMediaInfoPatch {
    pub(crate) async fn execute<F>(self, get: F, media_id: &str) -> Result<MediaInfo, Error>
    where
        // endpoint
        F: FnOnce(String) -> RequestBuilder,
    {
        let endpoint = self.endpoint.join(media_id);
        GetMediaInfo {
            request: get(endpoint.as_url()),
        }
        .execute()
        .await
    }
}

SimpleOutput! {
    GetMediaInfo => MediaInfo
}

SimpleOutput! {
    DeleteMedia => ()
}

// Doesn't have with_auth to prevent leaking auth
// TODO: Maybe we should check the url for facebook 'lookaside.fbsbx.com'
#[derive(Debug)]
pub(crate) struct DownloadMediaUrl<'dst, Dst> {
    request: RequestBuilder,
    dst: &'dst mut Dst,
}

impl<'dst, Dst: AsyncWrite + Send + Unpin> DownloadMediaUrl<'dst, Dst> {
    pub(crate) async fn execute(self) -> Result<String, Error> {
        let (response, endpoint);
        handle_arg!(self.request, response, endpoint);

        let content_type = handle_media(response, self.dst, endpoint).await?;
        Ok(content_type.unwrap_or_else(|| "application/octet-stream".to_owned()))
    }
}

async fn handle_media(
    mut response: Response,
    dst: &mut (impl AsyncWrite + Send + Unpin),
    endpoint: Cow<'_, str>,
) -> Result<Option<String>, Error> {
    if !response.status().is_success() {
        let status = response.status();

        return Err(Client::handle_not_ok(&response.bytes().await?)
            .service(endpoint, status)
            .into());
    }

    // Extract content type from headers
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    use tokio::io::AsyncWriteExt as _;

    // reqwest seems to have its impls on the futures crate
    // we'll just do the simple one...
    while let Some(chunk) = response.chunk().await? {
        dst.write_all(&chunk).await?;
    }
    dst.flush().await?;

    Ok(content_type)
}

// This is a patch for dependent chains of output pending a lasting, efficient
// solution...
// SendMessage is  infact a part of this group but is unable to benefit from
// this because it's more complex. But we're good as it's less dependent than
// these guys... hope we don't encounter an unpatchable case.
/// We take the output we depend on and the method to start us and return
/// an authenticated closure
macro_rules! inherit_auth {
    ($output:ident, $method:ident) => {
        let mut mut_output = $output;
        // for base guys only
        let request = mut_output.request;

        let (client, request) = request.build_split();
        let request = request?;

        // Or get last?
        let mut auth_headers = reqwest::header::HeaderMap::new();
        request
            .headers()
            .get_all(reqwest::header::AUTHORIZATION)
            .iter()
            .for_each(|v| {
                auth_headers.append(reqwest::header::AUTHORIZATION, v.clone());
            });

        let client_clone = client.clone();

        mut_output.request = RequestBuilder::from_parts(client, request);
        let $output = mut_output;

        $method = |url| {
            // FIXME: client is cloned again

            // Since we had no header prior to now, we could just mutably set this
            // on the Request
            client_clone.$method(url).headers(auth_headers)
        };
    };
}

pub(crate) struct DownloadMedia<'dst, Dst> {
    media_info: GetMediaInfo,
    dst: &'dst mut Dst,
}

impl<Dst> DownloadMedia<'_, Dst> {
    #[inline]
    pub(crate) fn with_auth(mut self, auth: &Auth) -> Self {
        self.media_info = self.media_info.with_auth(auth);
        self
    }
}

impl<'dst, Dst: AsyncWrite + Send + Unpin> DownloadMedia<'dst, Dst> {
    pub(crate) async fn execute(self) -> Result<String, Error> {
        let media_info = self.media_info;
        let get;
        inherit_auth!(media_info, get);

        let media_info = media_info.execute().await?;

        // For error
        //
        // consuming here would mean another parsing of the url in
        // reqwest IntoUrl
        let endpoint = media_info.url.as_str().to_owned();

        let response = get(media_info.url).send().await?;
        let content_type = handle_media(response, self.dst, Cow::Owned(endpoint)).await?;
        Ok(content_type.unwrap_or(media_info.mime_type))
    }
}

impl MessageManager<'_> {
    /// Uploads media content.
    pub(crate) fn upload_media_inner(
        &self,
        bytes: Vec<u8>,
        mime_type: &'static str,
        filename: impl Into<Cow<'static, str>>,
    ) -> UploadMedia {
        let url = self.endpoint("media");
        let part = Part::bytes(bytes)
            .file_name(filename)
            .mime_str(mime_type)
            .expect("valid mime_type");

        let form = Form::new()
            .text("messaging_product", "whatsapp")
            .part("file", part);

        UploadMedia {
            request: self.client.post(url).multipart(form),
        }
    }

    pub(crate) fn resolve_media(
        &self,
        media_source: MediaSource,
        media_type: MediaType,
        filename: impl Into<Cow<'static, str>>,
    ) -> ResolveMedia {
        match media_source {
            MediaSource::Bytes(bytes) => ResolveMedia::UploadMedia(Box::new(
                self.upload_media_inner(bytes, media_type.mime_type(), filename),
            )),
            MediaSource::Link(url) => ResolveMedia::MediaRef(MediaRef::Link(url)),
            MediaSource::Id(id) => ResolveMedia::MediaRef(MediaRef::Id(id)),
        }
    }
}

#[derive(Debug)]
pub(crate) struct UploadMedia {
    request: RequestBuilder,
}

impl IntoMessageRequestOutput for UploadMedia {
    type Request = String;

    #[inline]
    fn with_auth(mut self, auth: &Auth) -> Self {
        self.request = self.request.bearer_auth(auth);
        self
    }

    async fn execute(self) -> Result<Self::Request, Error> {
        let media_id: MediaUploadResponse = fut_net_op(self.request).await?;
        Ok(media_id.id)
    }
}

pub(crate) enum ResolveMedia {
    UploadMedia(Box<UploadMedia>),
    MediaRef(MediaRef),
}

/// `NOTE`: Should be flattened when used
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MediaRef {
    Id(String),
    Link(String),
}

impl IntoMessageRequestOutput for ResolveMedia {
    type Request = MediaRef;

    fn with_auth(mut self, auth: &Auth) -> Self {
        if let Self::UploadMedia(mut upload_media) = self {
            *upload_media = upload_media.with_auth(auth);
            self = Self::UploadMedia(upload_media);
            self
        } else {
            self
        }
    }

    async fn execute(self) -> Result<Self::Request, Error> {
        match self {
            ResolveMedia::UploadMedia(upload_media) => {
                let id = upload_media.execute().await?;
                Ok(MediaRef::Id(id))
            }
            ResolveMedia::MediaRef(media_ref) => Ok(media_ref),
        }
    }
}

/// Media information API response
#[derive(Debug, Deserialize)]
pub struct MediaInfo {
    // messaging_product: String,
    #[serde(deserialize_with = "url_parse")]
    url: Url,
    mime_type: String,
    // sha256: String,
    // file_size: u64,
    // // like I literally used this to get you
    // id: String,
}

fn url_parse<'de, D>(d: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = <Cow<'_, str>>::deserialize(d)?;
    raw.parse().map_err(|err| {
        <D::Error as serde::de::Error>::custom(format!("Invalid MediaInfo.url: {raw}: {err}"))
    })
}

/// Media upload API response
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct MediaUploadResponse {
    id: String,
}

/// Media content request structure.
#[derive(Serialize, Clone, Debug)]
pub(crate) struct MediaContentRequest {
    #[serde(flatten)]
    pub(crate) media: MediaRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) filename: Option<String>,
}

pub(crate) struct MediaOutput {
    pub(crate) media: ResolveMedia,
    pub(crate) caption: Option<String>,
    pub(crate) filename: Option<String>,
}

impl IntoMessageRequestOutput for MediaOutput {
    type Request = MediaContentRequest;

    #[inline]
    fn with_auth(mut self, auth: &Auth) -> Self {
        self.media = self.media.with_auth(auth);
        self
    }

    async fn execute(self) -> Result<Self::Request, Error> {
        Ok(Self::Request {
            media: self.media.execute().await?,
            caption: self.caption,
            filename: self.filename,
        })
    }
}

impl Media {
    // Some("OAuthException"), message: Some("An unknown error has occurred."),
    //
    // got some "unknown error" for not having filename
    // ... we really shouldn't do this though
    fn default_filename(&self) -> &'static str {
        "whatsapp_business_rs_set.ext"
    }
}

impl IntoMessageRequest for Media {
    type Request = MediaContentRequest;

    type Output = MediaOutput;

    fn into_request<'i, 't>(
        self,
        manager: &MessageManager<'i>,
        _to: &'t IdentityRef,
    ) -> Self::Output {
        let filename = self.filename.as_ref().map_or_else(
            || Cow::Borrowed(self.default_filename()),
            |f| Cow::Owned(f.clone()),
        );

        Self::Output {
            media: manager.resolve_media(self.media_source, self.media_type, filename),
            caption: self.caption.map(|c| c.body),
            filename: self.filename,
        }
    }
}

impl InteractiveHeaderMedia {
    fn default_filename(&self) -> &'static str {
        "whatsapp_business_rs_set.ext"
    }
}

impl IntoMessageRequest for InteractiveHeaderMedia {
    type Request = InteractiveMediaContentRequest;

    type Output = InteractiveMediaOutput;

    #[inline]
    fn into_request<'i, 't>(
        self,
        manager: &MessageManager<'i>,
        _to: &'t IdentityRef,
    ) -> Self::Output {
        let filename = Cow::Borrowed(self.default_filename());
        match self.media_source {
            MediaSource::Bytes(bytes) => Self::Output::UploadMedia((
                Box::new(manager.upload_media_inner(bytes, self.media_type.mime_type(), filename)),
                manager.client.get_media_info_patch(),
            )),
            MediaSource::Id(id) => Self::Output::Id(Box::new(manager.client.get_media_info(&id))),
            MediaSource::Link(url) => Self::Output::Link(url),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub(crate) struct InteractiveMediaContentRequest {
    pub(crate) link: String,
}

pub(crate) enum InteractiveMediaOutput {
    // This may later be solved by some OutputChain
    // so we have Box<OutputChain<UploadMedia, GetMediaInfo>> or similar
    UploadMedia((Box<UploadMedia>, GetMediaInfoPatch)),
    Id(Box<GetMediaInfo>),
    Link(String),
}

impl IntoMessageRequestOutput for InteractiveMediaOutput {
    type Request = InteractiveMediaContentRequest;

    #[inline]
    fn with_auth(mut self, auth: &Auth) -> Self {
        match self {
            Self::UploadMedia(mut upload_media) => {
                *upload_media.0 = upload_media.0.with_auth(auth);
                self = Self::UploadMedia(upload_media);
                self
            }
            Self::Id(mut get_media_info) => {
                *get_media_info = get_media_info.with_auth(auth);
                self = Self::Id(get_media_info);
                self
            }
            _ => self,
        }
    }

    #[inline]
    async fn execute(self) -> Result<Self::Request, Error> {
        let link = match self {
            Self::UploadMedia((upload_media, get_info)) => {
                let get;
                inherit_auth!(upload_media, get);

                let upload_media = upload_media.execute().await?;

                get_info.execute(get, &upload_media).await?.url.into()
            }
            Self::Id(get_media_info) => get_media_info.execute().await?.url.into(),
            Self::Link(link) => link,
        };
        Ok(Self::Request { link })
    }
}

pub(crate) type ContentRequest = crate::message::ContentRequest;
pub(crate) type ContentRequestOutput = crate::message::ContentOutput;
pub(crate) type ContentResponse<'a> = crate::message::ContentResponse<'a>;

impl Draft {
    #[inline]
    pub(crate) fn into_request<'i, 't>(
        self,
        manager: &MessageManager<'i>,
        to: Cow<'t, IdentityRef>,
    ) -> MessageRequestOutput<'t> {
        MessageRequestOutput {
            messaging_product: "whatsapp",
            biz_opaque_callback_data: self.biz_opaque_callback_data,
            content: self.content.into_request(manager, &to),
            context: self.context,
            to,
        }
    }
}

pub(crate) struct MessageRequestOutput<'t> {
    messaging_product: &'static str,
    biz_opaque_callback_data: Option<String>,
    to: Cow<'t, IdentityRef>,
    context: DraftContext,
    content: ContentRequestOutput,
}

impl<'t> IntoMessageRequestOutput for MessageRequestOutput<'t> {
    type Request = MessageRequest<'t>;

    #[inline]
    fn with_auth(mut self, auth: &Auth) -> Self {
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
pub(crate) struct MessageRequest<'t> {
    messaging_product: &'static str,

    #[serde(skip_serializing_if = "Option::is_none")]
    biz_opaque_callback_data: Option<String>,

    #[serde(flatten)]
    to: Cow<'t, IdentityRef>,

    #[serde(skip_serializing_if = "DraftContext::is_empty")]
    context: DraftContext,

    #[serde(flatten)]
    content: ContentRequest,
}

impl DraftContext {
    /// Check if context is empty (all fields None)
    fn is_empty(&self) -> bool {
        self.replied_to.is_none()
    }
}

impl<'de> Deserialize<'de> for MediaType {
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
    message_id: &'a str,
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

impl MessageRef {
    /// Create typing indicator request
    pub(crate) fn set_typing(&self) -> MessageStatusRequest<'_> {
        MessageStatusRequest {
            messaging_product: "whatsapp",
            status: MessageStatus::Read,
            message_id: self.message_id(),
            typing_indicator: Some(TypingIndicator {
                r#type: TypingIndicatorType::Text,
            }),
        }
    }

    /// Create read receipt request
    pub(crate) fn set_read(&self) -> MessageStatusRequest<'_> {
        MessageStatusRequest {
            messaging_product: "whatsapp",
            status: MessageStatus::Read,
            message_id: self.message_id(),
            typing_indicator: None,
        }
    }
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

pub(crate) fn serialize_text_text<S: Serializer>(v: &Text, ser: S) -> Result<S::Ok, S::Error> {
    let text: &TextRequestText = unsafe { transmute(v) };
    text.serialize(ser)
}

pub(crate) fn serialize_ordinary_text<S: Serializer>(v: &Text, ser: S) -> Result<S::Ok, S::Error> {
    let ordinary_text: &OrdinaryText = unsafe { transmute(&v.body) };
    ordinary_text.serialize(ser)
}

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
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize, Serialize, Debug)]
        #[serde(untagged)]
        enum AnyText {
            TextRequest(TextRequest),
            TextRequestText(TextRequestText),
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

// Interactive action serialization helpers
#[derive(Serialize, Deserialize, Debug)]
struct InteractiveActionRequest<'a, I> {
    r#type: Cow<'a, str>,
    action: I,
}

pub(crate) fn serialize_interactive_action<S: Serializer>(
    action: &<InteractiveAction as IntoMessageRequest>::Request,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let r#type = match action {
        InteractiveAction::Keyboard(_) => "button",
        InteractiveAction::CatalogDisplay(_) => "catalog_message",
        InteractiveAction::LocationRequest => "location_request_message",
        InteractiveAction::Cta(_) => "cta_url",
        InteractiveAction::ProductList(_) => "product_list",
        InteractiveAction::ProductDisplay(_) => "product",
        InteractiveAction::OptionList(_) => "list",
    };

    InteractiveActionRequest {
        r#type: Cow::Borrowed(r#type),
        action,
    }
    .serialize(serializer)
}

pub(crate) fn deserialize_interactive_action<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<<InteractiveAction as FromResponse>::Response<'de>, D::Error> {
    let request = InteractiveActionRequest::<InteractiveAction>::deserialize(deserializer)?;
    Ok(request.action)
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct CatalogDisplayOptionsRequest<'a> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thumbnail_product_retailer_id: Option<Cow<'a, str>>,
}

impl Serialize for CatalogDisplayOptions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let prx = CatalogDisplayOptionsRequest {
            thumbnail_product_retailer_id: self
                .thumbnail
                .as_ref()
                .map(|p| p.product_retailer_id().into()),
        };
        prx.serialize(serializer)
    }
}

impl<'a> Deserialize<'a> for CatalogDisplayOptions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'a>,
    {
        let prx = CatalogDisplayOptionsRequest::deserialize(deserializer)?;
        Ok(Self {
            thumbnail: prx
                .thumbnail_product_retailer_id
                .map(ProductRef::from_product_retailer_id),
        })
    }
}

macro_rules! serde_section {
    ($($item_name:ident => $ty:path),*) => {
    paste::paste!(
        $(
            #[derive(Serialize, Deserialize)]
            struct [<$ty Section>] {
                title: String,
                $item_name: Vec<$ty>
            }

            impl serde::Serialize for $crate::message::Section<$ty> {
                fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                    where
                        S: serde::Serializer {
                    let helper: &[<$ty Section>] = unsafe { std::mem::transmute(self) };
                    helper.serialize(serializer)
                }
            }

            impl<'de> serde::Deserialize<'de> for $crate::message::Section<$ty> {
                fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                    where
                        D: serde::Deserializer<'de> {
                    let helper = [<$ty Section>]::deserialize(deserializer)?;
                    Ok(unsafe { std::mem::transmute::<[<$ty Section>], $crate::message::Section<$ty>>(helper) })
                }
            }
        )*
    );
    }
}
use crate::message::OptionButton;

serde_section! {
    product_items => ProductRef,
    rows => OptionButton
}

#[derive(Serialize, Debug)]
pub(crate) struct WebhookConfigRequest<'a> {
    object: &'static str,
    callback_url: &'a str,
    verify_token: &'a str,
}

impl WebhookConfig {
    pub(crate) fn to_request(&self) -> WebhookConfigRequest<'_> {
        WebhookConfigRequest {
            object: "whatsapp_business_account",
            callback_url: &self.webhook_url,
            verify_token: &self.verify_token,
        }
    }
}

// FIXME: Make less loose
#[derive(Serialize, Debug, Default)]
pub(crate) struct AccessTokenRequest<'a> {
    // app id
    pub(crate) client_id: &'a str,

    // app secret
    pub(crate) client_secret: &'a str,

    pub(crate) grant_type: &'static str, // fb_exchange_token, client_credentials

    // for onboarding
    #[serde(skip_serializing_if = "str::is_empty")]
    pub(crate) code: &'a str,

    // for upgrade
    #[serde(skip_serializing_if = "str::is_empty")]
    pub(crate) fb_exchange_token: &'a str,
}

#[derive(Serialize)]
pub(crate) struct RegisterPhoneRequest<'a> {
    messaging_product: &'static str,
    pin: &'a str,
}

impl<'a> RegisterPhoneRequest<'a> {
    pub(crate) fn from_pin(pin: &'a str) -> Self {
        Self {
            messaging_product: "whatsapp",
            pin,
        }
    }
}

#[derive(Serialize, Debug)]
pub(crate) struct ShareCreditLineRequest<'a> {
    pub(crate) waba_id: &'a str,
    pub(crate) waba_currency: &'a str,
}

#[derive(Deserialize, Debug)]
pub(crate) struct ShareCreditLineResponse {
    // Save this ID if you want to verify that your credit
    // line has been shared with the customer.
    // 58501441721238
    pub(crate) allocation_config_id: String,
}

#[derive(Deserialize, Debug)]
pub(crate) struct WhatsAppBusinessApiData<T> {
    whatsapp_business_api_data: T,
}

impl FromResponse for AppInfo {
    type Response<'a> = WhatsAppBusinessApiData<AppInfo>;

    fn from_response<'a>(response: Self::Response<'a>) -> Result<Self, ServiceErrorKind> {
        Ok(response.whatsapp_business_api_data)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawOr<'a, T> {
    Raw(Cow<'a, str>),
    Tee(T),
}

pub(crate) fn deserialize_str<'de, T, D: Deserializer<'de>>(deserializer: D) -> Result<T, D::Error>
where
    T: FromStr + Deserialize<'de>,
    T::Err: Display,
{
    let v = <RawOr<T>>::deserialize(deserializer)?;
    match v {
        RawOr::Raw(s) => T::from_str(&s)
            .map_err(|err| <D::Error as serde::de::Error>::custom(format!("parsing value: {err}"))),
        RawOr::Tee(n) => Ok(n),
    }
}

pub(crate) fn serialize_str<T, S: Serializer>(v: &T, ser: S) -> Result<S::Ok, S::Error>
where
    T: ToString,
{
    ser.serialize_str(&v.to_string())
}

pub(crate) fn deserialize_str_opt<'de, T, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error>
where
    T: FromStr + Deserialize<'de>,
    T::Err: Display,
{
    let v = <Option<RawOr<T>>>::deserialize(deserializer)?;
    if let Some(v) = v {
        Ok(match v {
            RawOr::Raw(s) => Some(T::from_str(&s).map_err(|err| {
                <D::Error as serde::de::Error>::custom(format!("parsing value: {err}"))
            })?),
            RawOr::Tee(n) => Some(n),
        })
    } else {
        Ok(None)
    }
}

impl Serialize for ErrorContent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.errors.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ErrorContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let errors = <Vec<MetaError>>::deserialize(deserializer)?;
        Ok(Self { errors })
    }
}

impl Client {
    /// Handles API responses with consistent error mapping
    pub(crate) async fn handle_response<T: FromResponse>(
        // &self,
        response: Response,
        endpoint: Cow<'_, str>,
    ) -> Result<T, Error> {
        let status = response.status();
        let body = response.bytes().await?;

        if status.is_success() {
            match serde_json::from_slice(&body) {
                Ok(response) => {
                    T::from_response(response).map_err(|err| err.service(endpoint, status).into())
                }
                Err(err) => Err(ServiceError::parse(
                    err.into(),
                    String::from_utf8_lossy(&body).to_string(),
                )
                .service(endpoint, status)
                .into()),
            }
        } else {
            Err(Self::handle_not_ok(&body).service(endpoint, status).into())
        }
    }

    #[inline(always)]
    fn handle_not_ok(body: &[u8]) -> ServiceErrorKind {
        // double error
        #[derive(Deserialize, Debug)]
        struct Error {
            error: MetaError,
        }
        match serde_json::from_slice::<Error>(body) {
            Ok(structured_error) => ServiceError::api(structured_error.error),
            Err(structured_parse) => ServiceError::parse(
                structured_parse.into(),
                String::from_utf8_lossy(body).to_string(),
            ),
        }
    }
}

#[derive(Deserialize, Debug)]
pub(crate) struct SuccessStatus {
    pub(crate) success: bool,
}

impl FromResponse for () {
    type Response<'a> = SuccessStatus;

    #[inline]
    fn from_response<'a>(response: Self::Response<'a>) -> Result<Self, ServiceErrorKind> {
        if !response.success {
            Err(ServiceError::payload("Operation failed".into()))
        } else {
            Ok(())
        }
    }
}

impl FromResponse for PhantomData<()> {
    type Response<'a> = <() as FromResponse>::Response<'a>;

    fn from_response<'a>(response: Self::Response<'a>) -> Result<Self, ServiceErrorKind> {
        <() as FromResponse>::from_response(response).map(|_: ()| PhantomData)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct SendMessageResponseContact {
    wa_id: String,
    input: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct SendMessageResponseMetadata {
    id: String,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    message_status: Option<MessageStatus>,
}

/// Send message API response
#[derive(Debug, Deserialize)]
pub(crate) struct SendMessageResponse {
    messages: Vec<SendMessageResponseMetadata>,
    contacts: Vec<SendMessageResponseContact>,
}

impl FromResponse for MessageCreate {
    type Response<'a> = SendMessageResponse;

    fn from_response<'a>(mut response: Self::Response<'a>) -> Result<Self, ServiceErrorKind> {
        let metadata = response
            .messages
            .pop()
            .ok_or_else(|| ServiceError::payload("Missing message metadata in response".into()))?;

        let (message_id, status) = (metadata.id, metadata.message_status);

        let recipient_id = response
            .contacts
            .pop()
            .ok_or_else(|| ServiceError::payload("Missing message metadata in response".into()))?
            .wa_id;

        Ok(MessageCreate {
            message: MessageRef {
                message_id,
                // FIXME?: I mean it's us.... c'mon
                sender: None,
                recipient: Some(IdentityRef::business(recipient_id)),
            },
            message_status: status,
        })
    }
}

// Not so sure about this... their doc is too obscure
#[derive(Debug, Deserialize)]
pub(crate) struct CreateProductResponse {
    product: CreatedProduct,
}

#[derive(Debug, Deserialize)]
struct CreatedProduct {
    #[serde(rename = "product_id")]
    product_id: String,

    #[serde(rename = "retailer_id", default)]
    _retailer_id: Option<String>,
}

impl FromResponse for ProductCreate {
    type Response<'a> = CreateProductResponse;

    fn from_response<'a>(response: Self::Response<'a>) -> Result<Self, ServiceErrorKind> {
        // FIXME
        Ok(Self {
            product: MetaProductRef::from_product_id(response.product.product_id),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use reqwest::{header::AUTHORIZATION, RequestBuilder};

    use super::deserialize_str;
    use crate::message::{DocumentExtension, ImageExtension, MediaType};

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

    #[test]
    fn inherit_auth_test() -> Result<(), Box<dyn Error>> {
        struct Output {
            request: RequestBuilder,
        }

        let client = reqwest::Client::new();

        let output = Output {
            request: client
                .get("https://example.com")
                .bearer_auth("A")
                .bearer_auth("B"),
        };

        let get;
        inherit_auth!(output, get);

        let binding = get("https://example.com").build()?;
        let dep = binding.headers();
        let binding = output.request.build()?;
        let main = binding.headers();

        assert_eq!(dep, main);
        assert_eq!(
            dep.get_all(AUTHORIZATION)
                .into_iter()
                .collect::<Vec<_>>()
                .len(),
            2
        );
        Ok(())
    }
}
