use std::{borrow::Cow, fmt::Debug, hash::Hash, marker::PhantomData};

use crate::{
    app::{Token, TokenDebug},
    batch::AResponse,
    catalog::Product,
    client::{Auth, Client, MessageManager},
    derive,
    error::{Error, ServiceErrorKind},
    message::{
        Button, ErrorContent, InteractiveAction, InteractiveHeaderMedia, Location, Media, Order,
        Reaction, Text,
    },
    waba::{Catalog, PhoneNumber, RegisterResponse},
    IdentityRef,
};
use async_stream::try_stream;
use client::{
    MediaInfo, MediaUploadResponse, SendMessageResponse, ShareCreditLineResponse, SuccessStatus,
};
use futures::TryStream;
use reqwest::RequestBuilder;
use serde::Deserialize;

pub(crate) mod client;
pub(crate) mod macros;
pub(crate) mod server;

pub(crate) use macros::{AdjacentHelper, BuilderInto};
/// Trait for types that can be deserialized from API responses
pub(crate) trait FromResponse<'a>: Sized + Send {
    /// The raw response type that will be deserialized
    type Response: Deserialize<'a> + Debug + Send;

    // TODO: Restore when necessary
    /// Deserialize from raw API response
    /* async */
    fn from_response(
        response: Self::Response,
        // client: &Client,
    ) -> Result<Self, ServiceErrorKind>;
}

pub(crate) trait FromResponseOwned: for<'a> FromResponse<'a> {}
impl<T> FromResponseOwned for T where T: for<'de> FromResponse<'de> {}

// FIXME: Make less annoying and make preparation fallible
pub(crate) trait IntoMessageRequest: Sized + Send {
    type Output;

    /// Convert into API request format
    fn into_request<'i, 't>(
        self,
        manager: &MessageManager<'i>,
        to: &'t IdentityRef,
    ) -> Self::Output;
}

pub(crate) trait IntoMessageRequestOutput: /*crate::batch::Requests<ResponseReference = Self::Request> +*/ Sized + Send {
    /// The serializable request type
    type Request;

    fn with_auth(self, auth: Cow<'_, Auth>) -> Self;

    async fn execute(self) -> Result<Self::Request, Error>;
}

// We'd need something like this if we end-up supporting updating of auth in
// client since we wouldn't be able to get the auth immediately (as we'd
// use async Mutex) and since *Output preparations are sync....
// #[derive(Debug)]
// struct AsyncRequestBuilder {}

pub(crate) struct ReadyOutput<T> {
    inner: T,
}

impl<T: Send> IntoMessageRequestOutput for ReadyOutput<T> {
    type Request = T;

    #[inline(always)]
    fn with_auth(self, _: Cow<'_, Auth>) -> Self {
        self
    }

    #[inline(always)]
    async fn execute(self) -> Result<Self::Request, Error> {
        Ok(self.inner)
    }
}

#[cfg(feature = "batch")]
impl<T: Send> crate::batch::Requests for ReadyOutput<T> {
    type BatchHandler = ();

    type ResponseReference = <Self as IntoMessageRequestOutput>::Request;

    fn into_batch_ref(
        self,
        _: &mut crate::batch::Formatter,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), crate::batch::FormatError> {
        Ok(((), self.inner))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(0))
    }

    crate::requests_batch_include! {}
}

// Macro to implement IntoRequest for serializable types
macro_rules! impl_into_request_for_serializable {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IntoMessageRequest for $ty {
                type Output = ReadyOutput<$ty>;

                fn into_request<'i, 't>(
                    self,
                    _manager: &$crate::client::MessageManager<'i>,
                    _to:  &'t IdentityRef,
                ) -> Self::Output {
                    ReadyOutput {inner: self}
                }
            }
        )*
    };
}

// Macro to implement FromResponse for deserializable types
macro_rules! impl_from_response_for_deserializable {
    ($($ty:ty),* $(,)?) => {
        $(
            impl<'a> FromResponse<'a> for $ty {
                type Response = $ty;

                fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind>
                {
                    Ok(response)
                }
            }
        )*
    };
}

impl_into_request_for_serializable!(
    Location,
    Reaction,
    Text,
    InteractiveAction,
    Button,
    Order,
    ErrorContent
);

// These for from_response from webhook payload
impl_from_response_for_deserializable!(
    Media,
    InteractiveHeaderMedia,
    Location,
    Reaction,
    Text,
    InteractiveAction,
    Button,
    Order,
    ErrorContent,
    MediaInfo,
);

// These for from_response from active client interaction
impl_from_response_for_deserializable!(
    Product,
    MediaUploadResponse,
    SuccessStatus,
    SendMessageResponse,
    Paging,
    PhoneNumber,
    Catalog,
    RegisterResponse,
    Token,
    ShareCreditLineResponse,
    AResponse
);

#[derive(Deserialize, Debug)]
pub(crate) struct UnnecessaryDataWrapper<T> {
    data: T,
}

impl<'a> FromResponse<'a> for TokenDebug {
    type Response = UnnecessaryDataWrapper<TokenDebug>;

    fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind> {
        Ok(response.data)
    }
}
// Implementations for std container types

impl<'a, T: FromResponse<'a>> FromResponse<'a> for Vec<T> {
    type Response = Vec<<T as FromResponse<'a>>::Response>;

    fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind> {
        let mut results = Vec::with_capacity(response.len());
        for item in response {
            results.push(T::from_response(item)?);
        }
        Ok(results)
    }
}

impl<T> IntoMessageRequestOutput for Option<T>
where
    T: IntoMessageRequestOutput,
{
    type Request = Option<T::Request>;

    #[inline]
    fn with_auth(mut self, auth: Cow<'_, Auth>) -> Self {
        self = self.map(|output| output.with_auth(auth));
        self
    }

    #[inline]
    async fn execute(self) -> Result<Self::Request, Error> {
        match self {
            Some(output) => output.execute().await.map(Some),
            None => Ok(None),
        }
    }
}

impl<T: IntoMessageRequest> IntoMessageRequest for Option<T> {
    type Output = Option<T::Output>;

    fn into_request<'i, 't>(
        self,
        manager: &MessageManager<'i>,
        to: &'t IdentityRef,
    ) -> Self::Output {
        self.map(|item| item.into_request(manager, to))
    }
}

impl<'a, T: FromResponse<'a>> FromResponse<'a> for Option<T> {
    type Response = Option<<T as FromResponse<'a>>::Response>;

    fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind> {
        match response {
            Some(r) => {
                let value = T::from_response(r)?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }
}

// This is because of the persistent compiler normalization issue
pub(crate) fn option_from_response_default<'a, T>() -> <Option<T> as FromResponse<'a>>::Response
where
    T: FromResponse<'a>,
{
    None
}

derive! {
    /// Paginated API response
    #[derive(#crate::FromResponse, Debug)]
    pub(crate) struct Page<T> {
        data: Vec<T>,
        paging: Option<Paging>,
    }
}

/// Paging information for paginated responses
#[derive(Deserialize, Debug)]
pub(crate) struct Paging {
    cursors: Cursors,
}

/// Cursor information for pagination
#[derive(Deserialize, Debug)]
pub(crate) struct Cursors {
    after: String,
}

impl<T> Page<T> {
    /// Decompose into (data, next_page_token)
    pub(crate) fn into_parts(self) -> (Vec<T>, Option<String>) {
        (self.data, self.paging.map(|p| p.cursors.after))
    }
}

#[derive(Debug)]
pub struct Pager<Item> {
    pub(crate) request: RequestBuilder,
    pub(crate) item: PhantomData<Item>,
}

impl<Item> Pager<Item>
where
    Item: FromResponseOwned,
{
    // pub(crate) async fn next_page(
    //     self,
    // ) -> Result<(impl Iterator<Item = Item>, RequestBuilder), Error> {
    //     let next_request = self.request.try_clone().unwrap();

    //     // There's the page_token on this one
    //     let page: Page<Item> = fut_net_op(self.request).await?;
    //     let (items, token) = page.into_parts();

    //     // FIXME: The page_token param on this keeps building up since no overwrite occurs
    //     let next_request = next_request.query(&[("page_token", token)]);

    //     Ok((items.into_iter(), next_request))
    // }

    pub(crate) fn stream(self) -> impl TryStream<Ok = Item, Error = Error> {
        Box::pin(try_stream! {
            let mut next_page_token = None;

            loop {
                let request = self
                    .request
                    .try_clone()
                    .unwrap()
                    .query(&[("page_token", next_page_token)]);

                let page: Page<Item> = fut_net_op(request).await?;

                let (items, token) = page.into_parts();
                for item in items {
                    yield item;
                }

                next_page_token = token;
                if next_page_token.is_none() {
                    break;
                }
            }
        })
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! handle_arg {
    ($request:expr) => {{
        let (client, request) = $request.build_split();
        // FIXME: endpoint = lost...
        let request = request.map_err(|err| Error::internal(err.into()))?;
        // potential waste
        #[cfg(debug_assertions)]
        let endpoint = Cow::<'_, str>::Owned(request.url().as_str().to_owned());
        let response = client.execute(request).await?;
        #[cfg(debug_assertions)]
        {
            (response, endpoint)
        }

        #[cfg(not(debug_assertions))]
        {
            response
        }
    }};
}

// Make macros?
#[inline(always)]
pub(crate) async fn fut_net_op<T: FromResponseOwned>(request: RequestBuilder) -> Result<T, Error> {
    let r_e = handle_arg!(request);
    #[cfg(debug_assertions)]
    {
        Client::handle_response(r_e.0, r_e.1).await
    }

    #[cfg(not(debug_assertions))]
    {
        Client::handle_response(r_e).await
    }
}

#[inline(always)]
pub(crate) fn stream_net_op<T: FromResponseOwned>(
    request: RequestBuilder,
) -> impl TryStream<Ok = T, Error = Error> {
    let pager = Pager {
        request,
        item: PhantomData,
    };
    pager.stream()
}

pub(crate) trait FieldsTrait: Eq + Hash + Copy + Sized + 'static {
    const ALL: &'static [Self];

    fn as_snake_case(&self) -> &'static str;
}

#[cfg(test)]
mod ser_test {
    use serde_json::{from_str, json, to_value};

    use crate::{
        catalog::ProductRef,
        message::{
            Button, CatalogDisplayOptions, InteractiveAction, Keyboard, Location, MessageRef,
            OptionButton, OptionList, Order, OrderProduct, ProductList, Reaction, ReplyButton,
            Section, Text, UrlButton,
        },
    };

    macro_rules! decl_test {
        (
            |$title:ident|
            $ty:ident {$($body:tt)*} <=> $($jstr:tt)*
        ) => {
            #[test]
            fn $title() {
                let input = $ty {$($body)*};
                let want = json! {$($jstr)*};

                // Value won't borow us str... too many times
                let jstr = stringify!($($jstr)*);

                assert_eq!(to_value(&input).unwrap(), want);
                assert_eq!(from_str::<$ty>(jstr).unwrap(), input)
            }
        };
        (
            |$title:ident|
            $ty:ident {$($body:tt)*} <= $($jstr:tt)*
        ) => {
            #[test]
            fn $title() {
                let jstr = stringify!($($jstr)*);
                let want = $ty {$($body)*};

                assert_eq!(from_str::<$ty>(jstr).unwrap(), want);
            }
        };
        (
            // just success
            |$title:ident|
            $ty:ty = $($jstr:tt)*
        )=> {
            #[test]
            fn $title() {
                let jstr = stringify!($($jstr)*);
                let _: $ty = from_str(jstr).unwrap();

                // Null in Value breaks it
                // assert_eq!(to_value(de).unwrap(), json!{
                //     $($jstr)*
                // });
            }
        }
    }

    macro_rules! decl_test_enum {
        (
            |$title:ident|
            $ty:ident::$v:ident $(($($body:tt)*))? <=> $($jstr:tt)*
        ) => {
            #[test]
            fn $title() {
                let input = $ty::$v $(($($body)*))?;
                let want = json! {$($jstr)*};

                // Value won't borow us str... too many times
                let jstr = stringify!($($jstr)*);

                assert_eq!(to_value(&input).unwrap(), want);
                assert_eq!(from_str::<$ty>(jstr).unwrap(), input)
            }
        };
        {
            |$title:ident|
            $ty:ident::$v:ident $(($($body:tt)*))? => $($jstr:tt)*
        } => {
            #[test]
            fn $title() {
                let input = $ty::$v $(($($body)*))?;
                let want = json! {$($jstr)*};

                assert_eq!(to_value(&input).unwrap(), want);
            }
        };
        (
            |$title:ident|
            $ty:ident::$v:ident $(($($body:tt)*))? <= $($jstr:tt)*
        ) => {
            #[test]
            fn $title() {
                let jstr = stringify!($($jstr)*);
                let want = $ty::$v $(($($body)*))?;

                assert_eq!(from_str::<$ty>(jstr).unwrap(), want);
            }
        }
    }

    decl_test! {
        |location|
        Location {
            latitude: -30.0,
            longitude: 23.6,
            name: Some("Location name".to_owned()),
            address: Some("Location address".to_owned()),
        } <=> {
            "latitude": "-30",
            "longitude": "23.6",
            "name": "Location name",
            "address": "Location address"
        }
    }

    // decl_test! {
    //     |contacts|
    //     Vec<Contact> = [
    //       {
    //         "addresses": [
    //           {
    //             "street": "1 Lucky Shrub Way",
    //             "city": "Menlo Park",
    //             "state": "CA",
    //             "zip": "94025",
    //             "country": "United States",
    //             "country_code": "US",
    //             "type": "Office"
    //           },
    //           {
    //             "street": "1 Hacker Way",
    //             "city": "Menlo Park",
    //             "state": "CA",
    //             "zip": "94025",
    //             "country": "United States",
    //             "country_code": "US",
    //             "type": "Pop-Up"
    //           }
    //         ],
    //         "birthday": "1999-01-23",
    //         "emails": [
    //           {
    //             "email": "bjohnson@luckyshrub.com",
    //             "type": "Work"
    //           },
    //           {
    //             "email": "bjohnson@luckyshrubplants.com",
    //             "type": "Work (old)"
    //           }
    //         ],
    //         "name": {
    //           "formatted_name": "Barbara J. Johnson",
    //           "first_name": "Barbara",
    //           "last_name": "Johnson",
    //           "middle_name": "Joana",
    //           "suffix": "Esq.",
    //           "prefix": "Dr."
    //         },
    //         "org": {
    //           "company": "Lucky Shrub",
    //           "department": "Legal",
    //           "title": "Lead Counsel"
    //         },
    //         "phones": [
    //           {
    //             "phone": "+16505559999",
    //             "type": "Landline"
    //           },
    //           {
    //             "phone": "+19175559999",
    //             "type": "Mobile",
    //             "wa_id": "19175559999"
    //           }
    //         ],
    //         "urls": [
    //           {
    //             "url": "https://www.luckyshrub.com",
    //             "type": "Company"
    //           },
    //           {
    //             "url": "https://www.facebook.com/luckyshrubplants",
    //             "type": "Company (FB)"
    //           }
    //         ]
    //       }
    //     ]
    // }

    decl_test! {
        |reaction|
        Reaction {
            emoji: '😀',
            to: MessageRef::from_message_id("message_id"),
        } <=> {
            "emoji": "😀",
            "message_id": "message_id"
        }
    }

    decl_test! {
        |text|
        Text {
            body: "text message: https://link.msg".to_owned(),
            preview_url: Some(true),
        } <=> {
            "body": "text message: https://link.msg",
            "preview_url": true
        }
    }

    // Plain normal text, plain text, and
    // text with text field for body deserialization
    decl_test! {
        |text_normal_de|
        Text {
            body: "godly text".to_owned(),
            preview_url: None,
        } <= {
            "body": "godly text"
        }
    }

    decl_test! {
        |text_on_plain_text_de|
        Text {
            body: "plain-text".to_owned(),
            preview_url: None,
        } <= "plain-text"
    }

    decl_test! {
        |text_on_steroids_de|
        Text {
            body: "text on steroids".to_owned(),
            preview_url: None,
        } <= {
            "text": "text on steroids"
        }
    }
    //

    decl_test_enum! {
        |interactive_message_location|
        InteractiveAction::LocationRequest <=> {
            "name": "send_location"
        }
    }

    decl_test_enum! {
        |interactive_message_cta|
        InteractiveAction::Cta(UrlButton {
            url: "https://url.btn".to_owned(),
            label: "Fine cover".to_owned(),
        }) <=> {
            "name": "cta_url",
            "parameters": {
                "display_text": "Fine cover",
                "url": "https://url.btn"
            }
        }
    }

    decl_test_enum! {
        |interactive_message_catalog|
        InteractiveAction::CatalogDisplay(CatalogDisplayOptions {
            thumbnail: Some(ProductRef::from_product_retailer_id("product_retailer_id")),
        }) <=> {
            "name": "catalog_message",
            "parameters": {
                "thumbnail_product_retailer_id": "product_retailer_id"
            }
        }
    }

    decl_test_enum! {
        |interactive_message_option_list|
        InteractiveAction::OptionList(OptionList {
            sections: [Section {
                title: "section 1 title".to_owned(),
                items: [OptionButton {
                    description: "description".to_owned(),
                    call_back: "call_back".to_owned(),
                    label: "label".to_owned(),
                }]
                .into(),
            }]
            .into(),
            label: "Option label".to_owned(),
        }) <=> {
            "button": "Option label",
            "sections": [{
                "title": "section 1 title",
                "rows": [{
                    "id": "call_back",
                    "description": "description",
                    "title": "label"
                }]
            }]
        }
    }

    decl_test_enum! {
        |interactive_message_product|
        InteractiveAction::ProductDisplay(
            ProductRef::from_product_retailer_id("product_retailer_id")
                .with_catalog("catalog_id"),
        ) <=> {
            "catalog_id": "catalog_id",
            "product_retailer_id": "product_retailer_id"
        }
    }

    decl_test_enum! {
        |interactive_message_product_list|
        InteractiveAction::ProductList(ProductList {
            sections: [
                Section {
                    title: "section 1 title".to_owned(),
                    items: [ProductRef::from_product_retailer_id("product_retailer_id")]
                        .into(),
                },
                Section {
                    title: "section 2 title".to_owned(),
                    items: [ProductRef::from_product_retailer_id(
                        "product_retailer_id-s2",
                    )]
                    .into(),
                },
            ]
            .into(),
            catalog: "catalog_id".into(),
        }) <=> {
            "catalog_id": "catalog_id",
            "sections": [{
                "title": "section 1 title",
                "product_items": [{
                    "product_retailer_id": "product_retailer_id"
                }]
            }, {
                "title": "section 2 title",
                "product_items": [{
                    "product_retailer_id": "product_retailer_id-s2"
                }]
            }]
        }
    }

    decl_test_enum! {
        |interactive_message_keyboard|
        InteractiveAction::Keyboard(Keyboard {
            buttons: [Button::Reply(ReplyButton {
                call_back: "button_call_back".to_owned(),
                label: "Click me".to_owned(),
            })]
            .into(),
        }) => {
            "buttons": [{
                "type": "reply",
                "reply": {
                    "id": "button_call_back",
                    "title": "Click me"
                }
            }]
        }
    }

    decl_test_enum! {
        |interactive_message_button_reply|
        Button::Reply(ReplyButton {
            call_back: "button_call_back".to_owned(),
            label: "Click me".to_owned(),
        }) <= {
            "type": "button_reply",
            "button_reply": {
                "id": "button_call_back",
                "title": "Click me"
            }
        }
    }

    decl_test_enum! {
        |interactive_message_list_reply|
        Button::Option(OptionButton {
            description: "description".to_owned(),
            call_back: "call_back".to_owned(),
            label: "label".to_owned(),
        }) <= {
            "type": "list_reply",
            "list_reply": {
                "id": "call_back",
                "title": "label",
                "description": "description"
            }
        }
    }

    decl_test! {
        |order|
        Order {
            products: [OrderProduct {
                product: ProductRef::from_product_retailer_id("product_retailer_id"),
                quantity: 3,
                unit_price: 30.0,
                currency: "GBP".to_owned(),
            }]
            .into(),
            note: "Ummmm".into(),
            catalog: "catalog_id".into(),
        } <=> {
            "catalog_id": "catalog_id",
            "text": "Ummmm",
            "product_items": [{
              "product_retailer_id": "product_retailer_id",
              "quantity": "3",
              "item_price": "30",
              "currency": "GBP"
            }]
        }
    }
}
