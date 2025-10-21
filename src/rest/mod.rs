//! This module provides the core traits and helper functions for interacting with the API.
//! It defines a generic framework for creating, sending, and handling API requests and responses,
//! including support for paginated endpoints and multipart form uploads.

use std::{borrow::Cow, fmt::Debug, future::Future, hash::Hash, marker::PhantomData};

#[cfg(feature = "batch")]
use crate::batch::BatchSerializer;
use crate::{
    IdentityRef,
    app::{Token, TokenDebug},
    batch::BatchSubResponse,
    catalog::Product,
    client::{Auth, Client, MediaInfo, MessageManager, PendingRequest},
    error::{Error, ServiceErrorKind},
    message::{
        Button, ErrorContent, InteractiveAction, InteractiveHeaderMedia, Location, Media, Order,
        Reaction, Text,
    },
    waba::{Catalog, FlowInfo, PhoneNumber, RegisterResponse},
};
use async_stream::try_stream;
use client::{MediaUploadResponse, SendMessageResponse, ShareCreditLineResponse, SuccessStatus};
use futures::TryStream;
use reqwest::{RequestBuilder, multipart::Form};
use serde::{Deserialize, Serialize};

#[macro_use]
pub(crate) mod macros;
pub(crate) mod client;
#[cfg(any(feature = "server", feature = "byos"))]
pub(crate) mod server;

pub(crate) use macros::{AdjacentHelper, BuilderInto};

// --- Core API Traits ---

/// A trait for types that can be created from a raw, deserialized API response.
///
/// This trait is central to mapping the JSON responses from the API into the
/// strongly-typed structures used throughout this library.
pub(crate) trait FromResponse<'a>: Sized + Send {
    /// The raw response type that will be deserialized
    type Response: Deserialize<'a> + Debug + Send;

    /// Converts the raw, deserialized response into the final, high-level type.
    /// This can fail if the response data is invalid or incomplete.
    fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind>;
}

/// A helper trait for types that implement `FromResponse` for *any* lifetime `'a`.
///
/// This uses a Higher-Ranked Trait Bound (`for<'a>`) to indicate lifetime independence,
/// which is often required for types used in async functions and streams.
pub(crate) trait FromResponseOwned: for<'a> FromResponse<'a> {}
impl<T> FromResponseOwned for T where T: for<'de> FromResponse<'de> {}

// FIXME: Make less annoying

/// A trait for preparing a message to be sent via the API.
///
/// It handles the conversion of a high-level message type into a lower-level,
/// ready-to-send request object.
pub(crate) trait IntoMessageRequest: Sized + Send {
    /// The output type after preparation, which can then be executed.
    type Output;

    /// Converts the object into an object that gives the API request format.
    fn into_request<'i, 't>(
        self,
        manager: &MessageManager<'i>,
        to: &'t IdentityRef,
    ) -> Self::Output;
}

/// A trait for a prepared request that is ready to be executed.
pub(crate) trait IntoMessageRequestOutput: Sized + Send {
    /// The final, serializable request type that will be sent to the API.
    type Request;

    /// Attaches authentication credentials to the request.
    fn with_auth(self, auth: Cow<'_, Auth>) -> Self;

    /// Executes the request asynchronously, returning the final request object or an error.
    async fn execute(self) -> Result<Self::Request, Error>;
}

// --- Default Trait Implementations ---

/// A simple wrapper for requests that are already prepared and don't require async processing.
pub(crate) struct ReadyOutput<T> {
    inner: T,
}

impl<T: Send> IntoMessageRequestOutput for ReadyOutput<T> {
    type Request = T;

    #[inline(always)]
    fn with_auth(self, _auth: Cow<'_, Auth>) -> Self {
        self // No auth needed for an already-prepared object.
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

    #[inline]
    fn into_batch_ref(
        self,
        _: &mut BatchSerializer,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), crate::batch::FormatError> {
        Ok(((), self.inner))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(0))
    }

    requests_batch_include! {}
}

/// Implements `FromResponse` for types that can be directly deserialized without transformation
macro_rules! impl_from_response_for_deserializable {
    ($($ty:ty),* $(,)?) => {
        $(
            impl<'a> FromResponse<'a> for $ty {
                type Response = $ty;

                #[inline]
                fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind>
                {
                    Ok(response)
                }
            }
        )*
    };
}

/// Implements `IntoMessageRequest` for types that are already in a serializable format.
macro_rules! impl_into_request_for_serializable {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IntoMessageRequest for $ty {
                type Output = ReadyOutput<$ty>;

                #[inline]
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

impl_from_response_for_deserializable!(
    // Common types from webhook payloads
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
    // FlowCompletion

    // Common types from direct API calls
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
    BatchSubResponse,
    FlowInfo
);

impl_into_request_for_serializable!(
    Location,
    Reaction,
    Text,
    InteractiveAction,
    Button,
    Order,
    ErrorContent,
    // // GFM
    // FlowCompletion
);

/// A helper struct for unwrapping responses nested within a `{"data": ...}` object.
#[derive(Deserialize, Debug)]
pub(crate) struct UnnecessaryDataWrapper<T> {
    data: T,
}

impl<'a> FromResponse<'a> for TokenDebug {
    type Response = UnnecessaryDataWrapper<TokenDebug>;

    #[inline]
    fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind> {
        Ok(response.data)
    }
}

// --- Generic Implementations for Container Types ---

impl<'a, T: FromResponse<'a>> FromResponse<'a> for Vec<T> {
    type Response = Vec<T::Response>;

    #[inline]
    fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind> {
        response
            .into_iter()
            .map(T::from_response)
            .collect::<Result<Vec<T>, _>>()
    }
}

impl<T: IntoMessageRequest> IntoMessageRequest for Option<T> {
    type Output = Option<T::Output>;

    #[inline]
    fn into_request<'i, 't>(
        self,
        manager: &MessageManager<'i>,
        to: &'t IdentityRef,
    ) -> Self::Output {
        self.map(|item| item.into_request(manager, to))
    }
}

impl<T> IntoMessageRequestOutput for Option<T>
where
    T: IntoMessageRequestOutput,
{
    type Request = Option<T::Request>;

    #[inline]
    fn with_auth(self, auth: Cow<'_, Auth>) -> Self {
        self.map(|output| output.with_auth(auth))
    }

    #[inline]
    async fn execute(self) -> Result<Self::Request, Error> {
        match self {
            Some(output) => output.execute().await.map(Some),
            None => Ok(None),
        }
    }
}

impl<'a, T: FromResponse<'a>> FromResponse<'a> for Option<T> {
    type Response = Option<T::Response>;

    #[inline]
    fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind> {
        response.map(T::from_response).transpose()
    }
}

// This is because of the persistent compiler normalization issue
pub(crate) fn option_from_response_default<'a, T>() -> <Option<T> as FromResponse<'a>>::Response
where
    T: FromResponse<'a>,
{
    None
}

// --- Pagination Logic ---

derive! {
    /// Represents a single page of results from a paginated API endpoint.
    #[derive(#FromResponse, Debug)]
    pub(crate) struct Page<T> {
        data: Vec<T>,
        #![serde(default = "option_from_response_default::<Paging>")]
        paging: Option<Paging>,
    }
}

/// Contains the cursor information needed to fetch the next page.
#[derive(Deserialize, Debug)]
pub(crate) struct Paging {
    cursors: Cursors,
}

/// A cursor used to navigate between pages of results.
#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Cursors {
    /// The token pointing to the next page of results.
    after: Box<str>,
}

impl<T> Page<T> {
    /// Decomposes the page into its data and an optional token for the next page.
    pub(crate) fn into_parts(self) -> (Vec<T>, Option<Cursors>) {
        (self.data, self.paging.map(|p| p.cursors))
    }
}

/// A generic struct for handling paginated API endpoints, yielding items as a stream.
pub struct Pager<P, Q, Item>
where
    P: Serialize,
    Q: Serialize,
    Item: FromResponseOwned,
{
    pub(crate) request: PendingRequest<'static, P, Q>,
    pub(crate) item: PhantomData<Item>,
}

impl<P, Q, Item> Pager<P, Q, Item>
where
    P: Serialize + Clone,
    Q: Serialize + Clone,
    Item: FromResponseOwned,
{
    /// Consumes the pager and returns a stream that yields items from each page.
    pub(crate) fn stream(self) -> impl TryStream<Ok = Item, Error = Error> {
        Box::pin(try_stream! {
            // Start with an empty cursor for the first request.
            let mut next_page_token = Some(Cursors { after: "".into() });

            while let Some(cursor) = next_page_token {
                let request = self.request.clone().chain_query(cursor);

                // Fetch the next page of data.
                let page: Page<Item> = execute_request(request).await?;
                let (items, next_cursor) = page.into_parts();

                // Yield each item from the current page.
                for item in items {
                    yield item;
                }

                // Update the cursor for the next iteration. If it's None, the loop terminates.
                next_page_token = next_cursor;
            }
        })
    }
}

// --- Network Operation Helpers ---

/// A generic helper function to execute a standard JSON-based API request.
#[inline(always)]
pub(crate) fn execute_request<P, Q, T>(
    request: PendingRequest<'_, P, Q>,
) -> impl Future<Output = Result<T, Error>> + 'static
where
    P: Serialize,
    Q: Serialize,
    T: FromResponseOwned,
{
    #[cfg(debug_assertions)]
    let endpoint = request.endpoint.clone();
    let send_fut = request.send();

    async move {
        let response = send_fut.await?;
        Client::handle_response(
            response,
            #[cfg(debug_assertions)]
            endpoint.into(),
        )
        .await
    }
}

/// A generic helper function to execute a multipart form upload request.
#[inline(always)]
pub(crate) fn execute_multipart_request<'a, T>(
    request: PendingRequest<'_>,
    form: Form,
    #[cfg(debug_assertions)] handle: impl AsyncFnOnce(String, reqwest::Response) -> Result<T, Error>
    + 'a,
    #[cfg(not(debug_assertions))] handle: impl AsyncFnOnce(reqwest::Response) -> Result<T, Error> + 'a,
) -> impl Future<Output = Result<T, Error>> + 'a {
    #[cfg(debug_assertions)]
    let endpoint = request.endpoint.clone();
    let mut request_builder: RequestBuilder = request.into();
    request_builder = request_builder.multipart(form);

    async move {
        let response = request_builder.send().await?;

        handle(
            #[cfg(debug_assertions)]
            endpoint,
            response,
        )
        .await
    }
}

#[inline(always)]
pub(crate) fn execute_paginated_request<P, Q, T>(
    request: PendingRequest<'static, P, Q>,
) -> impl TryStream<Ok = T, Error = Error>
where
    P: Serialize + Clone,
    Q: Serialize + Clone,
    T: FromResponseOwned,
{
    let pager = Pager {
        request,
        item: PhantomData,
    };
    pager.stream()
}

// --- Field Selection Trait ---

/// A trait for enums that represent selectable fields in an API query.
///
/// This is used to construct comma-separated field lists, e.g., `fields=id,name,email`.
pub(crate) trait FieldsTrait: Eq + Hash + Copy + Sized + 'static {
    /// An array containing all possible variants of the enum.
    const ALL: &'static [Self];

    /// Returns the `snake_case` string representation of the field.
    fn as_snake_case(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    // A dummy struct for testing `FromResponse`.
    #[derive(Deserialize, Debug, PartialEq, Clone)]
    struct TestItem {
        id: i32,
        name: String,
    }

    // Manual `FromResponse` impl for the dummy struct.
    impl<'a> FromResponse<'a> for TestItem {
        type Response = TestItem;
        fn from_response(response: Self::Response) -> Result<Self, ServiceErrorKind> {
            Ok(response)
        }
    }

    #[test]
    fn test_from_response_for_vec() {
        let json_data = json!([
            { "id": 1, "name": "first" },
            { "id": 2, "name": "second" }
        ]);

        let response: Vec<TestItem> = serde_json::from_value(json_data).unwrap();
        let items = Vec::<TestItem>::from_response(response).unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            TestItem {
                id: 1,
                name: "first".to_string()
            }
        );
    }

    #[test]
    fn test_from_response_for_option() {
        // Test Some(T) case
        let json_some = json!({ "id": 42, "name": "some_item" });
        let response_some: Option<TestItem> = serde_json::from_value(json_some).unwrap();
        let item_some = Option::<TestItem>::from_response(response_some).unwrap();
        assert!(item_some.is_some());
        assert_eq!(item_some.unwrap().id, 42);

        // Test None case
        let json_none = json!(null);
        let response_none: Option<TestItem> = serde_json::from_value(json_none).unwrap();
        let item_none = Option::<TestItem>::from_response(response_none).unwrap();
        assert!(item_none.is_none());
    }

    #[test]
    fn test_page_into_parts() {
        let page = Page {
            data: vec![TestItem {
                id: 1,
                name: "item1".into(),
            }],
            paging: Some(Paging {
                cursors: Cursors {
                    after: "next_token".into(),
                },
            }),
        };

        let (data, next_cursor) = page.into_parts();
        assert_eq!(data.len(), 1);
        assert!(next_cursor.is_some());
        assert_eq!(next_cursor.unwrap().after.as_ref(), "next_token");
    }
}

#[cfg(test)]
mod ser_test {
    use serde_json::{from_str, json, to_value};

    use crate::{
        catalog::ProductRef,
        message::{
            Button, CatalogDisplayOptions,
            /*Flow, FlowActionType, FlowId, */ InteractiveAction, Keyboard, Location,
            MessageRef, OptionButton, OptionList, Order, OrderProduct, ProductList, Reaction,
            ReplyButton, Section, Text, UrlButton,
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

    // #[derive(serde::Serialize, Debug)]
    // struct FlowData {
    //     product_name: String,
    //     product_description: String,
    //     product_price: f64,
    // }

    // decl_test_enum! {
    //     |interactive_message_flow|
    //     InteractiveAction::Flow(
    //         Flow::new(FlowId::id("1"))
    //             .cta("Book!")
    //             .token("AQAAAAACS5FpgQ_cAAAAAD0QI3s")
    //             .action_type(FlowActionType::Navigate)
    //             .prefill("SCREEN_NAME", FlowData {
    //                 product_name: "name".into(),
    //                 product_description: "description".into(),
    //                 product_price: 100.0
    //             }),
    //     ) =>     {
    //       "name": "flow",
    //       "parameters": {
    //         "flow_message_version": "3",
    //         "flow_token": "AQAAAAACS5FpgQ_cAAAAAD0QI3s",
    //         "flow_id": "1",
    //         "flow_cta": "Book!",
    //         "flow_action": "navigate",
    //         "flow_action_payload": {
    //           "screen": "SCREEN_NAME",
    //           "data": {
    //             "product_name": "name",
    //             "product_description": "description",
    //             "product_price": 100.0
    //           }
    //         }
    //       }
    //     }
    // }

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
