//! A type-safe, fluent API for creating and executing batch API requests.
//!
//! This module provides the tools to group multiple independent or dependent API calls into a single
//! HTTP request, which can significantly reduce network latency and simplify complex
//! workflows. The design emphasizes compile-time safety to ensure that the shape of
//! the response you handle matches the shape of the requests you send.
//!
//! ## Core Concepts
//!
//! - **`Batch`**: The main entry point and builder struct. You start by creating a `Batch`
//!   from a `Client`.
//!
//! - **Fluent Interface**: You add requests to the batch using the `.include()` and
//!   `.include_iter()` methods in a chain.
//!
//! - **Type-Level Request Tracking**: As you add requests, the compiler tracks them at the
//!   type level. For example, adding two requests results in a `Batch<(Req1, Req2)>`.
//!   This ensures that when you process the response, you get a tuple of results,
//!   `(Result<Res1>, Result<Res2>)`, in the same order.
//!
//! ## Example Usage
//!
//! The primary workflow involves these steps:
//! 1. Create a `Client`.
//! 2. Start a new batch with `client.batch()`.
//! 3. Add individual requests or iterators of requests using `.include()` and `.include_iter()`.
//! 4. Send the batch request by calling `.execute().await`.
//! 5. Process the `BatchOutput`, often by using `.flatten()` to get a tuple of `Result`s.
//!
//! ```rust,no_run
//! # use whatsapp_business_rs::{Client, message::Media};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // 1. Create a client
//! let client = Client::new("YOUR-API-KEY").await?;
//!
//! // 2. Start a batch and 3. include requests
//! let output = client
//!     .batch()
//!     .include(client.message("SENDER").send("RECIPIENT", "Text message"))
//!     .include(client.message("SENDER").set_read("some_message"))
//!     // 4. Execute the batch
//!     .execute()
//!     .await?;
//!
//! // 5. Process the results
//! let (text_message_res, set_read_res) = output.flatten()?;
//!
//! if let Ok(res) = text_message_res {
//!     println!("Message sent: {}", res.message_id());
//! }
//! # Ok(())
//! # }
//! ```

use percent_encoding::NON_ALPHANUMERIC;
use reqwest::{
    header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE},
    multipart::{Form, Part},
    Method, StatusCode, Url,
};
use serde::{ser::SerializeSeq as _, Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;
use std::{borrow::Cow, collections::HashMap, fmt::Display, marker::PhantomData};

use crate::{
    add_auth_to_request, client::Endpoint, fut_net_op, Auth, Client, FromResponseOwned, IntoFuture,
    ToValue,
};

/// `Batch` is the entry point for grouping multiple requests to be sent as a single batch.
///
/// Use the `include` or `include_iter` methods to add requests to the batch, and `execute` to send it.
/// This approach can significantly reduce network overhead by sending multiple requests in one go,
/// and it also allows for complex operations like uploading binary data and referencing it within a message
/// in a single API call, leveraging powerful features of the underlying API.
///
/// # Authentication
///
/// By default, requests within a `Batch` will use the authentication configured with the `Client` that created the batch.
/// However, you can override this behavior by calling the `with_auth` method on individual requests before including them in the batch.
/// This allows you to use different authentication tokens for different operations within the same batch.
///
/// # Example
///
/// ```rust,no_run
/// # use whatsapp_business_rs::{Client, message::Media};
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Ensure the client is set up
/// let client = Client::new("YOUR-API-KEY").await?;
///
/// // For this example, we'll create a media object from a local file.
/// let media = Media::from_path("path/to/img")
///     .await?
///     .caption("Have a nice day!");
///
/// // Build the batch by including individual requests and iterators of requests.
/// // This demonstrates total type-safety without the need for macros.
/// let output = client
///     .batch()
///     .include(client.message("SENDER").send("RECIPIENT", "Text message"))
///     // `include_iter` is perfect for a list of requests of the same type.
///     .include_iter([
///         // The media and message will be sent in a single request,
///         // leveraging the batch API's ability to reference binary uploads.
///         // This avoids the two-trip process of uploading media and then sending the message.
///         client.message("SENDER").send("RECIPIENT", media),
///         client.message("SENDER1").send("RECIPIENT1", "Hello, Friend"),
///     ])
///     .include(
///         client
///             .app("1083258447013388")
///             .configure_webhook(("ver_tok", "https://example.com"))
///             .with_auth("APP_TOKEN"),
///     )
///     .include(client.message("SENDER").set_read("some_message"))
///     .execute()
///     .await?;
///
/// // The `execute` method handles the request formatting, network communication,
/// // and initial response parsing. Errors here would be related to the batch request itself
/// // (e.g., network issues, invalid credentials).
///
/// // You can destructure the results with individual error handling for each request.
/// // The `flatten` method provides a convenient way to handle the results as a tuple,
/// // with each element containing its own `Result`.
/// let (text_message_res, messages_res, app_config_res, set_read_res) = output.flatten()?;
///
/// // We can then handle the results of each individual request, logging any failures.
/// // The error here corresponds to issues with a specific request within the batch,
/// // such as an invalid message ID or a failed configuration.
///
/// match text_message_res {
///     Ok(res) => println!("Text message sent: {}", res.message_id()),
///     Err(e) => eprintln!("Failed to send text message: {e:#}"),
/// }
///
/// // `messages_res` is an iterator of results, so we can loop through and process each one.
/// for (i, message_res) in messages_res.into_iter().enumerate() {
///     match message_res {
///         Ok(res) => println!("Message {} sent: {}", i, res.message_id()),
///         Err(e) => eprintln!("Failed to send message {i}: {e:#}"),
///     }
/// }
///
/// match app_config_res {
///     Ok(_) => println!("App configured successfully."),
///     Err(e) => eprintln!("Failed to configure app: {e:#}"),
/// }
///
/// match set_read_res {
///     Ok(_) => println!("Set read for some message."),
///     Err(e) => eprintln!("Failed to set message as read: {e:#}"),
/// }
///
/// Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
#[must_use = "Batch does nothing until executed"]
pub struct Batch<Requests = ()> {
    /// The collection of requests, tracked at the type level (e.g., `()`, `Req1`, `(Req1, Req2)`).
    requests: Requests,
    /// A clone of the client used to create the batch, providing default authentication and HTTP capabilities.
    client: Client,
}

impl Batch {
    /// Creates a new, empty `Batch` associated with a `Client`.
    pub(crate) fn new(client: &Client) -> Self {
        Self {
            client: client.clone(),
            requests: (),
        }
    }

    /// Naively extracts the relative URL from a full URL.
    ///
    /// This assumes the `Client`'s base URL is the host and everything after that
    /// is the relative path.
    fn make_url_relative(url: &str) -> &str {
        // Find the position of the third slash
        let mut slash_count = 0;
        for (i, ch) in url.char_indices() {
            if ch == '/' {
                slash_count += 1;
                if slash_count == 3 {
                    return &url[i..];
                }
            }
        }
        "/" // no relative path found
    }
}

/// An internal helper macro to bubble up a `FormatError` during the batch
/// serialization process, immediately returning an `ExecuteBatch` in an error state.
macro_rules! tri_batch {
    ($expr:expr) => {
        match $expr {
            Err(err) => return BatchExecute::err(err),
            Ok(ok) => ok,
        }
    };
}

impl<Rs> Batch<Rs>
where
    Rs: Requests,
{
    /// Adds a single request to the batch.
    ///
    /// The type of the `Batch` will change to reflect the added request,
    /// ensuring that the response type will also match. To conditionally include
    /// a request, wrap it in an `Option`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use whatsapp_business_rs::{Client, batch::Requests as _};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = Client::new("YOUR_API_KEY").await?;
    /// let condition = true;
    ///
    /// let request = client.message("SENDER").set_read("some_id");
    ///
    /// let output = client
    ///     .batch()
    ///     .include(client.message("SENDER").send("RECIPIENT", "Always send this"))
    ///     // This request is only included if `condition` is true.
    ///     .include(if condition { Some(request) } else { None })
    ///     .execute()
    ///     .await?;
    ///
    /// // The response type will be `(Result<_, _>, Option<Result<_, _>>)`
    /// let (always_res, conditional_res) = output.flatten()?;
    ///
    /// if let Some(Ok(_)) = conditional_res {
    ///     println!("Conditional request was successful.");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn include<R>(self, r: R) -> Batch<Rs::Include<R>> {
        Batch {
            requests: self.requests.include(r),
            client: self.client,
        }
    }

    /// Adds an iterator of requests to the batch.
    ///
    /// This is useful for adding a dynamic number of requests of the same type.
    /// The results for these requests will be available as an iterator in the final output.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use whatsapp_business_rs::{Client};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = Client::new("YOUR_API_KEY").await?;
    /// let recipients = ["RECIPIENT_1", "RECIPIENT_2", "RECIPIENT_3"];
    ///
    /// let output = client
    ///     .batch()
    ///     .include_iter(
    ///         recipients
    ///             .iter()
    ///             .map(|&r| client.message("SENDER").send(r, "Bulk message"))
    ///     )
    ///     .execute()
    ///     .await?;
    ///
    /// let (bulk_results,) = output.flatten()?;
    /// for result in bulk_results {
    ///     match result {
    ///         Ok(res) => println!("Sent message: {}", res.message_id()),
    ///         Err(e) => eprintln!("Failed to send message: {e:#}"),
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn include_iter<I, R>(self, iter: I) -> Batch<Rs::Include<Many<I::IntoIter>>>
    where
        I: IntoIterator<Item = R>,
    {
        self.include(Many::new(iter.into_iter()))
    }

    /// Consumes the `Batch` builder and prepares it for execution.
    ///
    /// This method formats all included requests into a single multipart form
    /// payload and returns an `ExecuteBatch` future. You must `.await` this
    /// future to send the request.
    pub fn execute(self) -> BatchExecute<Rs::BatchHandler> {
        let (size, _) = self.requests.size_hint();

        // Use a conservative heuristic of 256 bytes per request for pre-allocation.
        const AVERAGE_REQ_SIZE: usize = 256;
        let out = Vec::with_capacity(size * AVERAGE_REQ_SIZE);

        let mut binding = serde_json::Serializer::new(out);
        let serializer = tri_batch!(binding
            .serialize_seq(Some(size)) // Useless to serde_json
            .map_err(FormatError::Serialization));

        let mut f = Formatter::new(serializer);
        let handler = tri_batch!(self.requests.into_batch(&mut f));

        // Finalize the serialization and handle any potential errors.
        let files = tri_batch!(f.finish_into_files());

        let batch_bytes = binding.into_inner();

        let batch_part = Part::bytes(batch_bytes);

        BatchExecute::from_parts(batch_part, files, handler, self.client)
    }
}

/// Internal state for a pending batch execution.
struct BatchExecuteState<Handler> {
    request: reqwest::RequestBuilder,
    /// The multipart form containing the JSON batch payload and any attached files.
    form: Form,
    /// The handler responsible for parsing the API's response.
    handler: Handler,
}

/// Represents a batch request that has been built and is ready to be sent.
///
/// This struct is a future that, when `.await`ed, will send the HTTP request
/// and parse the top-level response. It is created by calling `.execute()` on a `Batch`.
#[must_use = "ExecuteBatch does nothing unless you `.await` or `.execute().await` it"]
pub struct BatchExecute<Handler> {
    state: Result<BatchExecuteState<Handler>, FormatError>,
}

// FIXME: The default auth on different clients won't work here
impl<Handler> BatchExecute<Handler> {
    /// Internal constructor to create an `ExecuteBatch` from its component parts.
    fn from_parts(batch: Part, files: Vec<Part>, handler: Handler, client: Client) -> Self {
        const BATCH: &str = "batch";
        let mut form = Form::new().part(BATCH, batch);

        for (i, file) in files.into_iter().enumerate() {
            let name = Formatter::get_binary_name(i);
            form = form.part(name, file);
        }

        const ROOT: Endpoint<'static> = Endpoint::without_version();

        let request = client.post(ROOT);
        let me = Self {
            state: Ok(BatchExecuteState {
                request,
                form,
                handler,
            }),
        };
        // By default, headers are not included in the response.
        me.include_headers(false)
    }

    /// Internal constructor for a failed batch creation.
    fn err(err: FormatError) -> Self {
        Self { state: Err(err) }
    }

    /// Toggles whether the API should include HTTP headers in the response for each sub-request.
    ///
    /// By default, headers are not included.
    ///
    /// # Example
    /// ```rust, no_run
    /// # use whatsapp_business_rs::{Client};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("YOUR_API_KEY").await?;
    /// let output = client
    ///     .batch()
    ///     .include(client.message("SENDER").send("RECIPIENT", "Hi"))
    ///     .execute()
    ///     .include_headers(true) // Request headers for sub-responses
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn include_headers(mut self, include_headers: bool) -> Self {
        const INCLUDE_HEADERS: &str = "include_headers";

        self.state = self.state.map(|mut state| {
            state.form = state.form.text(
                INCLUDE_HEADERS,
                if include_headers { "true" } else { "false" },
            );
            state
        });

        self
    }

    /// Overrides the default authentication for the entire batch request.
    ///
    /// This sets the `access_token` for the top-level multipart request.
    /// This is different from calling `.with_auth()` on an individual request
    /// before adding it to the batch.
    pub fn with_auth<'a>(mut self, auth: impl ToValue<'a, Auth>) -> Self {
        // Using the access_token field means getting a String... but with
        // the normal bearer way, we get to reuse the possibly parsed auth

        self.state = self.state.map(|mut state| {
            state.request = add_auth_to_request!(auth.to_value() => state.request);
            state
        });

        self
    }
}

IntoFuture! {
    impl<H> BatchExecute<H>
    [
    where
        H: Handler + Send + 'static,
        ]
    {
        /// Sends the batch request and returns a `BatchOutput`.
        ///
        /// This method performs the network operation and a preliminary parsing of the
        /// top-level batch response JSON. The returned `BatchOutput` can then be used
        /// to parse the individual results using `.flatten()`.
        pub async fn execute(self) -> Result<BatchOutput<H>, crate::Error> {
            let state = self.state?;
            // Meta's batch response quotes the inner JSON body, forcing us to
            // deserialize and potentially re-allocate, rather than borrowing
            // slices(bytes::Bytes with no life issue) from the original response body.

            // TODO: Use the static GRAPH_ENDPOINT for debug
            let response: Vec<Option<AResponse>> = fut_net_op(state.request.multipart(state.form)).await?;

            Ok(BatchOutput::from_parts(state.handler, response))
        }
    }
}

// --- Traits ---

/// A trait for objects that can be included in a batch request.
///
/// This trait defines the contract for how a request or group of requests is
/// formatted into the JSON payload sent to the API. It also specifies the
/// `Handler` type responsible for parsing the corresponding response.
///
/// **Note**: This is a sealed trait and not meant for external implementation.
pub trait Requests {
    /// The handler type responsible for parsing the response(s) for this request.
    type BatchHandler;

    /// A symbolic reference to a request's future response.
    ///
    /// This type acts as a placeholder for the response that a request will generate once
    /// executed. It does not contain any actual data. Instead, it holds the necessary
    /// instructions (like JSONPath expressions) to refer to the eventual response fields.
    ///
    /// Its sole purpose is to compose dependent requests within a single batch call using
    /// methods like `.then()` and `.then_nullable()`.
    ///
    /// ## How It Works: An Analogy
    ///
    /// Think of a `ResponseReference` like a spreadsheet formula. When you type `=A1+5`
    /// into a cell, you are not using the *current value* of cell A1, but creating a
    /// *reference* that the spreadsheet will resolve later. Similarly, this type creates a
    /// reference to a response that will only be resolved by the server during the
    /// execution of the batch request.
    ///
    /// ## Crucial Limitations
    ///
    /// This is a "fictive" response with a very restricted use case. Understanding these
    /// limitations is key to using it correctly.
    ///
    /// - **Cannot Be Inspected**: You **cannot** read data from this reference at runtime.
    ///   It has no actual value until the server processes the request. Any logic that
    ///   relies on checking its fields (e.g., in an `if` statement or `match` block) is
    ///   impossible, as the state of the actual response is unknown when building the batch.
    ///
    /// ```rust,ignore
    ///   // ❌ This is NOT possible
    ///   .then_nullable(|upload_ref| {
    ///       // You cannot inspect the value of `upload_ref.id` here.
    ///       if upload_ref.id.starts_with("error") {
    ///           // This logic cannot be executed.
    ///       }
    ///   })
    /// ```
    ///
    /// - **Batch Context Only**: A `ResponseReference` is only valid inside the closure passed
    ///   to `.then()` for the purpose of building another request. You should not store it,
    ///   return it from a function, or attempt to use it in a separate, non-batch API call.
    ///
    /// - **Opaque Type**: Its internal structure is deliberately hidden. It should only be used
    ///   by passing it to other request-building methods that are designed to accept it.
    ///
    /// This type should implement `Clone` to allow a single antecedent response to be used
    /// as a dependency for multiple subsequent requests.
    type ResponseReference;

    /// Formats the request and produces its handler and response reference.
    ///
    /// This is the core method for serialization within a batch, called internally
    /// by `Batch::execute`.
    fn into_batch_ref(
        self,
        f: &mut Formatter,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError>;

    /// Formats the request(s) and writes them to the batch `Formatter`.
    ///
    /// This method is responsible for serializing the request's JSON representation
    /// and handling any binary attachments.
    //
    // Implementation NOTE: An optimized version should be implemented to avoid
    // bloating request with unused request names
    fn into_batch(self, f: &mut Formatter) -> Result<Self::BatchHandler, FormatError>
    where
        Self: Sized,
    {
        let (h, _) = self.into_batch_ref(f)?;
        Ok(h)
    }

    /// The resulting type when this request is joined with another.
    ///
    /// This is typically a nested tuple, like `(Self, R)`, which allows the compiler
    /// to track the sequence of requests.
    type Include<R>;
    // FIXME: Maybe this shouldn't be assoc... there's possibility
    // of many join techniques for one type. With macro, we may
    // make tuples join 'flatly' upto a certain amount...
    // Batch may join as (Batch, R) and as Batch<Before + R> though the former
    // seems more intuitive for unpacking

    /// Joins this request group with another.
    fn include<R>(self, r: R) -> Self::Include<R>;

    /// Provides a hint about the number of individual requests contained within this object.
    ///
    /// This is used for pre-allocating capacity to improve performance.
    fn size_hint(&self) -> (usize, Option<usize>) {
        (1, None)
    }

    /// Creates a dependent request that executes within the same batch.
    ///
    /// This method builds a chain of operations that are sent to the API in a single
    /// network request. It doesn't execute requests sequentially; instead, it constructs
    /// a dependency graph.
    ///
    /// ## How It Works
    ///
    /// The closure `f` receives a `ResponseReference`, which is **not the actual response data**.
    /// Think of it as a symbolic placeholder or a pointer to a future result. This reference
    /// can be used to build subsequent requests that depend on the output of the first one.
    /// The entire graph of requests is then sent and resolved by the server in one go.
    ///
    /// ## Why Use `then`?
    ///
    /// - **Efficiency**: It's perfect for "do-once, use-many-times" scenarios. For example,
    ///   you can upload a media file once and then use its resulting ID in multiple
    ///   messages to different recipients, all within a single batch. This saves bandwidth
    ///   and reduces latency compared to multiple round trips.
    /// - **Logical Atomicity**: It allows you to group related actions into a single, logical
    ///   operation. For example, sending a message, reacting to it, and setting the "replying"
    ///   status can be chained together to appear as a single, seamless interaction.
    ///
    /// ## Important: `then` vs. `then_nullable`
    ///
    /// When a request is used as a dependency, the API often returns a literal JSON `null`
    /// in its response slot. A standard parser would treat this as an error and halt,
    /// potentially failing the entire batch to avoid misaligning the response cursor.
    ///
    /// Because of this behavior, it is **strongly recommended to use `then_nullable` instead**.
    ///
    /// # Example: Uploading and sending the same media to multiple users
    /// ```rust,no_run
    /// # use whatsapp_business_rs::{Client, message::Media, batch::{Requests, Many}};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("YOUR_API_KEY").await?;
    /// # let (media_bytes, media_type, filename) = Media::from_path("/path/to/image.png")
    /// #     .await?
    /// #     .into_upload_parts()
    /// #     .unwrap();
    /// #
    /// # let recipients = ["RECIPIENT_1", "RECIPIENT_2"];
    /// #
    /// let sender = client.message("SENDER");
    ///
    /// // Using .then_nullable() is the recommended approach.
    /// let request = sender
    ///     .upload_media(media_bytes, media_type, filename)
    ///     .then_nullable(|upload_ref| {
    ///         // `upload_ref` is a cheap-to-clone reference to the future upload result.
    ///         let messages: Many<_> = recipients
    ///             .into_iter()
    ///             .map(move |recipient| {
    ///                 sender.send(
    ///                     recipient,
    ///                     Media::png(upload_ref.clone()).caption("Here is your image!"),
    ///                 )
    ///             })
    ///             .into();
    ///         messages
    ///     });
    ///
    /// // Execute the entire chain in a single batch request.
    /// let output = client.batch().include(request).execute().await?;
    ///
    /// // The output will contain the result for the media upload and an iterator of results for the messages.
    /// let (upload_result, message_results) = output.flatten()?;
    ///
    /// // upload_result is already `nulled` so it may have no actual response
    /// if let Some(Ok(upload)) = upload_result {
    ///     println!("Successfully uploaded media with ID: {}", upload);
    /// }
    ///
    /// for result in message_results {
    ///     if let Ok(msg) = result {
    ///         println!("Successfully sent message: {}", msg.message_id());
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    fn then<R, F>(self, f: F) -> Then<Self, R, F>
    where
        Self: Sized,
        F: FnOnce(Self::ResponseReference) -> R,
        R: Requests,
    {
        Then::new(self, f)
    }

    /// Creates a dependent request where the initial request's response can be null.
    ///
    /// ## The `null` Response Problem
    ///
    /// When a request `A` is used to satisfy a dependency for a request `B` within a batch,
    /// the API often returns a literal JSON `null` for `A`'s response. Our parser is strict
    /// by default and would see this `null` as a critical error, failing the entire batch
    /// to prevent a misalignment of the internal response cursor.
    ///
    /// ## The Solution
    ///
    /// This method transparently wraps the initial request with `.nullable()`. This signals
    /// to the parser that a `null` response is an expected, valid outcome for this specific
    /// request. The parser will then correctly interpret `null` as `Ok(None)` and continue
    /// processing the rest of the batch without issue.
    ///
    /// **This is the recommended and safest method for creating dependent requests.**    
    fn then_nullable<R, F>(self, f: F) -> Then<Self::Nullable, R, F>
    where
        Self: Nullable + Sized,
        F: FnOnce(Self::ResponseReference) -> R,
        R: Requests,
    {
        Then::new(self.nullable(), f)
    }
}

/// A trait for handling the response of a request from a batch operation.
///
/// Each `Requests` object has a corresponding `Handler` that knows how to
/// parse the JSON response returned by the API for that specific request.
///
/// **Note**: This is a sealed trait and not meant for external implementation.
pub trait Handler {
    /// The final type that the response(s) will be parsed into.
    ///
    /// For a single request, this might be `Result<SuccessType, ErrorType>`. For a
    /// group of requests, it could be an iterator or a `Vec`.
    type Responses;

    /// Parses the required response(s) from the batch response iterator.
    ///
    /// This method consumes one or more items from the `BatchResponse` iterator
    /// and deserializes them into the `Responses` type.
    #[allow(clippy::wrong_self_convention)]
    fn from_batch(self, response: &mut BatchResponse)
        -> Result<Self::Responses, FromResponseError>;
}

/// A trait for constructing a `ResponseReference` from a request ID.
///
/// This trait serves as the counterpart to the crate’s internal
/// `FromResponse` trait. While `FromResponse` converts raw server responses
/// (`RawT`) into higher-level types (`BrushedUpT`), this trait works in the
/// opposite direction: it generates placeholder values (`ResponseReference`)
/// that mimic the structure of a type, but with each field replaced by a
/// JSONPath expression derived from the `reference_id`.
///
/// This internal trait bridges the gap between a request being formatted and the
/// creation of its corresponding placeholder (`ResponseReference`) used for
/// dependent requests. The `reference_id` is typically a JSONPath expression
/// like `result=request-1:$.body.id`.
///
/// > **Note:** This is an internal API detail. It is not part of the
/// > public interface and may change at any time.
pub trait IntoResponseReference {
    /// The placeholder type for the response.
    type ResponseReference;

    /// Creates a response reference from a given ID.
    fn into_response_reference(reference_id: Cow<'static, str>) -> Self::ResponseReference;
}

/// A trait for requests whose responses can be null in a batch operation.
///
/// ## The Problem: Null Responses in Dependent Requests
///
/// When chaining requests where one depends on the output of another (e.g., using `.then()`),
/// the Facebook Graph API often returns `null` in the JSON response array for the
/// antecedent request. For example, a batch request for `request_A` followed by
/// `request_B(output_of_A)` might yield a response like `[null, { ... response_B ... }]`.
///
/// A standard parser would see `null` where it expects an object for `request_A` and fail,
/// causing the entire batch parsing to stop. This trait provides a mechanism to gracefully
/// handle this `null` value.
///
/// ---
///
/// ## Why a Trait? The Failure of a Generic Wrapper
///
/// An earlier solution used a generic wrapper struct, `Nullable<T>`, which would catch a
/// `NullNotNullable` parsing error and reinterpret it as a valid `None` result. While simple,
/// this approach had a critical flaw: **ambiguity**.
///
/// For a request that expects a **single** response object, "nullable" is clear. But for a
/// composite request expecting **multiple** responses (e.g., a tuple of handlers `(H1, H2)`),
/// it's ambiguous:
/// - Does `null` mean **all** component responses are null?
/// - Does it mean just **one** of them is?
/// - Which one?
///
/// This ambiguity makes it impossible for a single, generic wrapper to correctly advance
/// the response iterator, leading to a high risk of **deserialization misalignment**, where
/// subsequent handlers would parse the wrong parts of the response, corrupting the entire
/// batch result.
///
/// ---
///
/// ## The Solution: Compositional Nullability
///
/// This trait solves the problem by delegating the definition of "nullable" to the request
/// type itself. Instead of a one-size-fits-all wrapper, each request type can define
/// precisely what its nullable version looks like via the `type Nullable;` associated type.
///
/// This allows for an objective and compositional approach. For instance:
/// - A **single request** `H` can define its `Nullable` type to parse an `Option<H::Response>`.
/// - A **composite request** `(H1, H2)` can define its `Nullable` type as `(H1::Nullable, H2::Nullable)`.
///
/// This recursive structure ensures that the nullability of each component is handled
/// correctly, preserving the overall shape of the expected response and preventing any
/// parser misalignment. The `.nullable()` method is the entry point for this conversion,
/// wrapping the request in a type that instructs the parser on how to handle potential `null`
/// values in a safe, predictable, and structurally sound manner.
#[cfg_attr(
    not(no_diagnostic_namespace),
    diagnostic::on_unimplemented(
        note = "Check for a `.nullable()` method on `{Self}`. Some types in this module could not implement the `Nullable` trait due to restrictions related to `impl Trait`.",
    )
)]
pub trait Nullable {
    /// The nullable version of this request, which can handle `null` in the API response.
    type Nullable;

    /// Wraps the request, marking its response as potentially `null`.
    ///
    /// This should be called on requests that are used as a dependency for another
    /// request within the same batch.
    fn nullable(self) -> Self::Nullable;
}

/// A macro to implement `Nullable` for a type that is already nullable,
/// making the `.nullable()` call idempotent.
#[macro_export]
#[doc(hidden)]
macro_rules! impl_idempotent_nullable {
    ($already_nullable:ident <$($life:lifetime,)? $($T:ident),*>) => {
        impl<$($life,)? $($T),*> $crate::batch::Nullable for $already_nullable<$($life,)? $($T),*> {
            type Nullable = $already_nullable<$($life,)? $($T),*>;

            fn nullable(self) -> Self::Nullable {
                self
            }
        }
    };
}

/// A helper macro to create static arrays of strings for form part names,
/// avoiding heap allocations for common cases.
macro_rules! static_names {
    ($const_name:ident $main:literal [$($suffix:literal),*]) => {
        static $const_name: &[&str] = &[$main, $(
            concat!($main, $suffix)
        ),*];

        paste::paste! {
            #[inline]
            fn [<$const_name:lower>](index: usize) -> Cow<'static, str> {
                $const_name
                    .get(index)
                    .map(|s| Cow::Borrowed(*s))
                    .unwrap_or_else(|| format!("{}{}",$main, index).into())
            }
        }
    };
}

// FIXME: Somehow we have to have selection of names by code and somehow ensure
// uniqueness... Just found out the media enpoint won't take any file part name
// other than "file"
// Change to source?

// Static arrays for named file and request parts to avoid heap allocations for small batches.
static_names! {
    FILE_NAMES "file" [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]
}

static_names! {
    REQUEST_NAMES "request" [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]
}

/// Helper macro to implement the `Requests::include` method for tuples.
/// This is used internally to recursively build up the batch request types.
/// While default in assoc is unstable
#[macro_export]
#[doc(hidden)]
macro_rules! requests_batch_include {
    () => {
        // We need a name like this to avoid collision
        type Include<SomeR> = (Self, SomeR);

        fn include<SomeR>(self, r: SomeR) -> Self::Include<SomeR> {
            (self, r)
        }
    };
}

// --- Trait Implementations ---

pub(crate) use trait_impls::NullableUnit;

/// Internal module containing trait implementations for core types.
pub(crate) mod trait_impls {
    use crate::{FromResponseOwned, Update};

    use super::{Nullable, *};

    /// Implements `Requests` for a tuple `(R1, R2)`, allowing the recursive composition of requests.
    ///
    /// This is the core of the type-safe builder pattern. Each call to `.include()` on a batch
    /// wraps the existing requests in another layer of this tuple implementation.
    impl<R1, R2> Requests for (R1, R2)
    where
        R1: Requests,
        R2: Requests,
    {
        type BatchHandler = (R1::BatchHandler, R2::BatchHandler);

        type ResponseReference = (R1::ResponseReference, R2::ResponseReference);

        fn into_batch_ref(
            self,
            f: &mut Formatter,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            let (h1, r1) = self.0.into_batch_ref(f)?;
            let (h2, r2) = self.1.into_batch_ref(f)?;
            Ok(((h1, h2), (r1, r2)))
        }

        fn into_batch(self, f: &mut Formatter) -> Result<Self::BatchHandler, FormatError> {
            let h1 = self.0.into_batch(f)?;
            let h2 = self.1.into_batch(f)?;
            Ok((h1, h2))
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            let (lower_1, upper_1) = self.0.size_hint();
            let (lower_2, upper_2) = self.1.size_hint();
            let lower = lower_1.saturating_add(lower_2);
            let upper = match (upper_1, upper_2) {
                (Some(x), Some(y)) => x.checked_add(y),
                _ => None,
            };
            (lower, upper)
        }

        requests_batch_include! {}
    }

    /// Implements `Handler` for a tuple of handlers, processing responses in sequence.
    ///
    /// This mirrors the `Requests` implementation for tuples, ensuring that the response
    /// parsing logic follows the same structure as the request building logic.
    impl<H1, H2> Handler for (H1, H2)
    where
        H1: Handler,
        H2: Handler,
    {
        // #[must_use = "these `Responses` may contain errors, which should be handled"]
        type Responses = (H1::Responses, H2::Responses);

        fn from_batch(
            self,
            response: &mut BatchResponse,
        ) -> Result<Self::Responses, FromResponseError> {
            let r1 = self.0.from_batch(response)?;
            let r2 = self.1.from_batch(response)?;
            Ok((r1, r2))
        }
    }

    /// Implements `Nullable` for a tuple, applying nullability to each element.
    impl<T1, T2> Nullable for (T1, T2)
    where
        T1: Nullable,
        T2: Nullable,
    {
        type Nullable = (T1::Nullable, T2::Nullable);

        fn nullable(self) -> Self::Nullable {
            (self.0.nullable(), self.1.nullable())
        }
    }

    /// Base case implementation for an empty set of requests (`()`).
    impl Requests for () {
        type BatchHandler = ();
        type ResponseReference = ();

        fn into_batch_ref(
            self,
            _: &mut Formatter,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            Ok(((), ()))
        }

        fn into_batch(self, _f: &mut Formatter) -> Result<Self::BatchHandler, FormatError> {
            Ok(())
        }

        /// When a request `R` is included in an empty batch, the batch becomes `R`.
        type Include<R> = R;

        fn include<R>(self, r: R) -> Self::Include<R> {
            r
        }
    }

    /// Base case implementation for an empty handler (`()`).
    impl Handler for () {
        type Responses = ();

        fn from_batch(
            self,
            _response: &mut BatchResponse,
        ) -> Result<Self::Responses, FromResponseError> {
            Ok(())
        }
    }

    /// Implements `Requests` for `Option<R>`, allowing for the conditional
    /// inclusion of a request in a batch. If `None`, no request is added, and no
    /// response is expected.
    impl<R> Requests for Option<R>
    where
        R: Requests,
    {
        type BatchHandler = Option<R::BatchHandler>;

        type ResponseReference = Option<R::ResponseReference>;

        fn into_batch_ref(
            self,
            f: &mut Formatter,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            match self {
                Some(r) => {
                    let (h, r) = r.into_batch_ref(f)?;
                    Ok((Some(h), Some(r)))
                }
                None => Ok((None, None)),
            }
        }

        fn into_batch(self, f: &mut Formatter) -> Result<Self::BatchHandler, FormatError> {
            match self {
                Some(r) => Ok(Some(r.into_batch(f)?)),
                None => Ok(None),
            }
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            match self {
                Some(r) => r.size_hint(),
                None => (0, Some(0)),
            }
        }

        requests_batch_include! {}
    }

    // This is for; I might or might not have entered the request
    impl<T> Handler for Option<T>
    where
        T: Handler,
    {
        type Responses = Option<T::Responses>;

        fn from_batch(
            self,
            response: &mut BatchResponse,
        ) -> Result<Self::Responses, FromResponseError> {
            match self {
                Some(handler) => handler.from_batch(response).map(Some),
                None => Ok(None),
            }
        }
    }

    // If the request were present, it'd be able to handle
    // null response
    impl<T> Nullable for Option<T>
    where
        T: Nullable,
    {
        type Nullable = Option<T::Nullable>;

        fn nullable(self) -> Self::Nullable {
            self.map(|t| t.nullable())
        }
    }

    crate::SimpleOutputBatch! {
        Update <'a, [T: Serialize + Send + 'static], [U: FromResponseOwned + 'static]> => U
    }

    // This is what's become of the former Nullable struct
    // Building block for Nullable
    //
    // Using something like NullableUnit<T> would prevent
    // an operation from going multistep in the future without
    // breaking changes (somehow the handler is public).. because
    // that NullableUnit<T>... so we use macro instead
    #[macro_export]
    #[doc(hidden)]
    macro_rules! NullableUnit {
        ($T:ident <$($life:lifetime,)? $([$g:ty: $($b:tt)*]),*>) => {
            paste::paste!{
                impl<$($life,)? $($g: $($b)*),*> $crate::batch::Nullable for $T<$($life,)? $($g),*> {
                    type Nullable = [<Nullable $T>] <$($life,)? $($g),*>;

                    fn nullable(self) -> Self::Nullable {
                        Self::Nullable {
                            inner: self
                        }
                    }
                }

                $crate::impl_idempotent_nullable! {[<Nullable $T>] <$($life,)? $($g),*>}

                pub struct [<Nullable $T>] <$($life,)? $($g),*> {
                    inner: $T <$($life,)? $($g),*>
                }

                impl<$($life,)? $($g: $($b)*),*> $crate::batch::Requests for [<Nullable $T>] <$($life,)? $($g),*> {
                    type BatchHandler = [<Nullable $T Handler>] <$($life,)? $($g),*>;

                    type ResponseReference = <$T <$($life,)? $($g),*> as $crate::batch::Requests>::ResponseReference;

                    #[inline]
                    fn into_batch_ref(
                        self,
                        f: &mut $crate::batch::Formatter,
                    ) -> Result<(Self::BatchHandler, Self::ResponseReference), $crate::batch::FormatError> {
                        let (h, r) = self.inner.into_batch_ref(f)?;
                        Ok(([<Nullable $T Handler>]{inner: h}, r))
                    }

                    #[inline]
                    fn into_batch(self, f: &mut $crate::batch::Formatter) -> Result<Self::BatchHandler, $crate::batch::FormatError>
                    where
                        Self: Sized,
                    {
                        let h = self.inner.into_batch(f)?;
                        Ok([<Nullable $T Handler>]{inner: h})
                    }

                    fn size_hint(&self) -> (usize, Option<usize>) {
                        // (1, Some(1)) SendMessage breaks this assumption
                        self.inner.size_hint()
                    }

                    $crate::requests_batch_include! {}
                }

                pub struct [<Nullable $T Handler>] <$($life,)? $($g: $($b)*),*> {
                    inner: <$T <$($life,)? $($g),*> as $crate::batch::Requests>::BatchHandler
                }

                impl<$($life,)? $($g: $($b)*),*> $crate::batch::Handler for [<Nullable $T Handler>] <$($life,)? $($g),*> {
                    type Responses =
                        Option<<<$T <$($life,)? $($g),*> as $crate::batch::Requests>::BatchHandler as $crate::batch::Handler>::Responses>;

                    fn from_batch(
                        self,
                        response: &mut $crate::batch::BatchResponse,
                    ) -> Result<Self::Responses, $crate::batch::FromResponseError> {
                        match self.inner.from_batch(response) {
                            Ok(val) => Ok(Some(val)),
                            Err($crate::batch::FromResponseError {
                                kind: $crate::batch::FromResponseErrorKind::NullNotNullable,
                                ..
                            }) => Ok(None),
                            Err(err) => Err(err), // real error, propagate
                        }
                    }
                }
            }
        }
    }

    // For internals
    pub(crate) struct NullableUnit<T>(pub T);

    impl<T> Handler for NullableUnit<T>
    where
        T: Handler,
    {
        type Responses = Option<T::Responses>;

        fn from_batch(
            self,
            response: &mut BatchResponse,
        ) -> Result<Self::Responses, FromResponseError> {
            match self.0.from_batch(response) {
                Ok(val) => Ok(Some(val)),
                Err(FromResponseError {
                    kind: FromResponseErrorKind::NullNotNullable,
                    ..
                }) => Ok(None),
                Err(err) => Err(err), // real error, propagate
            }
        }
    }
}

impl<Rs> Requests for Batch<Rs>
where
    Rs: Requests,
{
    // TODO:
    type BatchHandler = Rs::BatchHandler;

    type ResponseReference = Rs::ResponseReference;

    fn into_batch_ref(
        self,
        f: &mut Formatter,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
        self.requests.into_batch_ref(f)
    }

    fn into_batch(self, f: &mut Formatter) -> Result<Self::BatchHandler, FormatError> {
        self.requests.into_batch(f)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.requests.size_hint()
    }

    requests_batch_include! {}
}

impl<T> Nullable for Batch<T>
where
    T: Nullable,
{
    type Nullable = Batch<T::Nullable>;

    fn nullable(self) -> Self::Nullable {
        Batch {
            client: self.client,
            requests: self.requests.nullable(),
        }
    }
}

/// A helper struct that wraps an iterator of requests to be included in a batch.
///
/// This is the concrete type used by `Batch::include_iter` to handle collections
/// of requests.
#[derive(Clone, Debug)]
pub struct Many<Iter> {
    iter: Iter,
}

impl<Iter> Many<Iter> {
    /// Creates a new `Many` from an iterator.
    pub fn new(iter: Iter) -> Self {
        Self { iter }
    }
}

impl<I, Rs> From<I> for Many<I::IntoIter>
where
    I: IntoIterator<Item = Rs>,
    Rs: Requests,
{
    fn from(value: I) -> Self {
        Self {
            iter: value.into_iter(),
        }
    }
}

impl<Iter, Rs> Requests for Many<Iter>
where
    Iter: Iterator<Item = Rs>,
    Rs: Requests,
{
    type BatchHandler = ManyHandlers<Rs::BatchHandler>;

    type ResponseReference = ManyReferences<Rs::ResponseReference>;

    fn into_batch_ref(
        self,
        f: &mut Formatter,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
        let (size, _) = self.iter.size_hint();
        let mut handlers = Vec::with_capacity(size);
        let mut references = Vec::with_capacity(size);
        for request in self.iter {
            let (h, r) = request.into_batch_ref(f)?;
            handlers.push(h);
            references.push(r)
        }

        Ok((
            ManyHandlers { handlers },
            ManyReferences {
                references: references.into_iter(),
            },
        ))
    }

    fn into_batch(self, f: &mut Formatter) -> Result<Self::BatchHandler, FormatError> {
        // We should not have any allocation in most cases in release builds since most
        // handlers are zero sized
        let (size, _) = self.iter.size_hint();
        let mut handlers = Vec::with_capacity(size);
        for request in self.iter {
            handlers.push(request.into_batch(f)?)
        }

        Ok(ManyHandlers { handlers })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Let's assume 1 request per `Requests`... Only `()` currently fails this
        let (iter_lower, _) = self.iter.size_hint();
        (iter_lower, None)
    }
    requests_batch_include! {}
}

// Restricted by impl Trait
impl<I, T> Many<I>
where
    I: Iterator<Item = T>,
    T: Nullable,
{
    pub fn nullable(self) -> Many<impl Iterator<Item = T::Nullable>> {
        Many {
            iter: self.iter.map(|t| t.nullable()),
        }
    }
}

#[cfg(feature = "nightly_rust")]
impl<I, T> Nullable for Many<I>
where
    I: Iterator<Item = T>,
    T: Nullable,
{
    type Nullable = Many<impl Iterator<Item = T::Nullable>>;

    pub fn nullable(self) -> Self::Nullable {
        self.nullable()
    }
}

/// A handler for a collection of requests.
pub struct ManyHandlers<H> {
    handlers: Vec<H>,
}

impl<H> Handler for ManyHandlers<H>
where
    H: Handler,
{
    type Responses = ManyResponses<H::Responses>;

    fn from_batch(
        self,
        response: &mut BatchResponse,
    ) -> Result<Self::Responses, FromResponseError> {
        // FIXME: Somehow support lazy parsing
        //
        // We stemmed from a list of list of request with only the handlers
        // knowing where to stop reading... so our expected calculation is
        // wrong... say we have Many<Many<SomeRequest>> created from 5 Many's
        // We'd think we should expect 5 individual items but we'd be wrong
        // because each Many may be expecting any number of requests.

        let mut all_responses = Vec::with_capacity(self.handlers.len());
        for handler in self.handlers {
            let responses = handler.from_batch(response)?;
            all_responses.push(responses);
        }
        Ok(ManyResponses {
            responses: all_responses.into_iter(),
        })
    }
}

pub struct ManyReferences<R> {
    references: <Vec<R> as IntoIterator>::IntoIter,
}

impl<R> Iterator for ManyReferences<R> {
    type Item = R;

    fn next(&mut self) -> Option<Self::Item> {
        self.references.next()
    }
}
/// An iterator over the parsed responses for a `Many` batch request group.
///
/// The items of the iterator are the `Responses` type of the inner handler,
/// allowing you to process each result individually.
#[must_use = "these `Responses` may contain errors, which should be handled"]
#[derive(Debug)]
pub struct ManyResponses<R> {
    responses: <Vec<R> as IntoIterator>::IntoIter,
}

impl<R> Iterator for ManyResponses<R> {
    type Item = R;

    fn next(&mut self) -> Option<Self::Item> {
        self.responses.next()
    }
}

/// Used to create a dependent request chain for a batch operation.
/// It encapsulates a request (r1) and a closure (f) that will be executed later.
/// This allows you to build a graph of requests where the output of one request
/// (represented by a ResponseReference) becomes the input for a subsequent request.
/// The entire chain is then sent to the server in a single, efficient batch.
#[derive(Debug)]
pub struct Then<R1, R2, F> {
    r1: R1,
    f: F,
    _r2: PhantomData<R2>,
}

impl<R1, R2, F> Then<R1, R2, F> {
    pub fn new(r: R1, f: F) -> Self {
        Self {
            r1: r,
            f,
            _r2: PhantomData,
        }
    }
}

impl<R1, R2, F> Requests for Then<R1, R2, F>
where
    R1: Requests,
    F: FnOnce(R1::ResponseReference) -> R2,
    R2: Requests,
{
    type BatchHandler = (R1::BatchHandler, R2::BatchHandler);

    type ResponseReference = R2::ResponseReference;

    fn into_batch_ref(
        self,
        f: &mut Formatter,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
        let (h, r_ref) = self.r1.into_batch_ref(f)?;

        let (h1, r1_ref) = (self.f)(r_ref).into_batch_ref(f)?;

        // R entered before R1
        Ok(((h, h1), r1_ref))
    }

    fn into_batch(self, f: &mut Formatter) -> Result<Self::BatchHandler, FormatError> {
        let (h, r_ref) = self.r1.into_batch_ref(f)?;

        let h1 = (self.f)(r_ref).into_batch(f)?;

        // R entered before R1
        Ok((h, h1))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (lower, _) = self.r1.size_hint();
        // Let's assume 1 request in the request to be produced
        (lower + 1, None)
    }

    requests_batch_include! {}
}

// We can't implement the Nullable trait because of impl Trait restriction
impl<R1, R2, F> Then<R1, R2, F>
where
    R1: Nullable + Requests,
    F: FnOnce(R1::ResponseReference) -> R2,
    R2: Nullable,
{
    pub fn nullable(
        self,
    ) -> Then<R1::Nullable, R2::Nullable, impl FnOnce(R1::ResponseReference) -> R2::Nullable> {
        Then {
            r1: self.r1.nullable(),
            f: move |args| {
                let r1 = (self.f)(args);
                r1.nullable()
            },
            _r2: PhantomData,
        }
    }
}

#[cfg(feature = "nightly_rust")]
impl<R1, R2, F> Nullable for Then<R1, R2, F>
where
    R1: Nullable + Requests,
    F: FnOnce(R1::ResponseReference) -> R2,
    R2: Nullable,
{
    type Nullable =
        Then<R1::Nullable, R2::Nullable, impl FnOnce(R1::ResponseReference) -> R2::Nullable>;

    fn nullable(
        self,
    ) -> Then<R1::Nullable, R2::Nullable, impl FnOnce(R1::ResponseReference) -> R2::Nullable> {
        self.nullable()
    }
}

// Partial nullable... This is for when the left is already nulled and the nulled
// doesn't implement nullable or some issues with incompleteness
impl<R1, R2, F> Then<R1, R2, F>
where
    R1: Nullable + Requests,
    F: FnOnce(R1::ResponseReference) -> R2,
    R2: Nullable,
{
    pub fn right_nullable(
        self,
    ) -> Then<R1, R2::Nullable, impl FnOnce(R1::ResponseReference) -> R2::Nullable> {
        Then {
            r1: self.r1,
            f: move |args| {
                let r1 = (self.f)(args);
                r1.nullable()
            },
            _r2: PhantomData,
        }
    }
}

// --- Response Handling ---

/// The result of a successful batch request execution, containing the response data.
///
/// This struct holds the response parsing logic (`Handler`) and the raw response
/// iterator. It is an intermediate step; you must call a method like `result()` or
/// `flatten()` to perform the final parsing of individual responses.
#[derive(Debug)]
#[must_use = "this `BatchOutput` may contain errors, which should be handled"]
pub struct BatchOutput<H> {
    handler: H,
    response: BatchResponse,
}

impl<H> BatchOutput<H>
where
    H: Handler,
{
    /// Consumes the `BatchOutput` and parses all responses.
    ///
    /// This drives the `Handler` to consume the response iterator and produce the
    /// final, often nested result type.
    pub fn result(mut self) -> Result<H::Responses, BatchResponseError> {
        self.handler
            .from_batch(&mut self.response)
            .map_err(|err| BatchResponseError { inner: err })
    }

    /// Consumes the `BatchOutput`, parses the responses, and flattens the result.
    ///
    /// This is a convenience method that is especially useful when dealing with the
    /// nested tuple structure created by the `Batch` builder. It unnests the tuples
    /// for easier destructuring.
    ///
    /// ### Example
    /// If your responses are `((Res1, Res2), Res3)`, flattening produces `(Res1, Res2, Res3)`.
    pub fn flatten<F>(self) -> Result<F, BatchResponseError>
    where
        F: FlattenFrom<H::Responses>,
    {
        self.result().map(FlattenFrom::flatten_from)
    }
}

impl<H> BatchOutput<H> {
    fn from_parts(handler: H, response: Vec<Option<AResponse>>) -> Self {
        Self {
            handler,
            response: BatchResponse::from_part(response),
        }
    }
}

#[derive(Debug)]
pub(crate) struct BatchResponse {
    // If we could get some deserialize_seq, it'd be smooth
    responses: <Vec<Option<AResponse>> as IntoIterator>::IntoIter,
}

impl BatchResponse {
    fn from_part(iter: Vec<Option<AResponse>>) -> Self {
        Self {
            responses: iter.into_iter(),
        }
    }

    #[inline]
    pub fn handle_next_typical<T>(
        &mut self,
        #[cfg(debug_assertions)] endpoint: Cow<'static, str>,
    ) -> Result<Result<T, crate::Error>, FromResponseError>
    where
        T: FromResponseOwned,
    {
        match self.next() {
            Some(Ok(Some(me))) => {
                let status = me.status();
                let body = me.body();
                Ok(Client::handle_response_(
                    status,
                    body.as_bytes(),
                    #[cfg(debug_assertions)]
                    endpoint,
                ))
            }
            Some(Ok(None)) => Err(FromResponseError::null_not_nullable().error(
                #[cfg(debug_assertions)]
                endpoint,
            )),
            Some(Err(err)) => Err(err),
            None => Err(FromResponseError::insufficient(1, 0).error(
                #[cfg(debug_assertions)]
                endpoint,
            )), // it'd be good if we could track positions,
        }
    }
}

impl Iterator for BatchResponse {
    // It seems meta completely eats the response of a request
    // that'd been depended on... like they swapped it with null
    // and moved it... this is a speculation... they may do it for
    // some other reasons... so if you know you're not that important
    // just be Ok(None)

    /// `Ok(Some(AResponse))` → real response
    ///
    /// `Ok(None)` → slot was literal null
    ///
    /// `Err(_)` → genuine parse/transport error
    type Item = Result<Option<AResponse>, FromResponseError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.responses.next().map(Ok)
    }
}

#[derive(Deserialize, Debug)]
pub(crate) struct AResponse<Body = BatchResBody> {
    code: u16,
    body: Body,
}

pub(crate) type BatchResBody = Box<str>;

impl<Body> AResponse<Body> {
    pub fn status(&self) -> StatusCode {
        StatusCode::from_u16(self.code).unwrap_or_default()
        // .expect("Valid status from meta")
    }

    pub fn body(self) -> Body {
        self.body
    }
}

// --- Formatter & Builders ---

#[must_use = "don't forget to call finish_into_files"]
pub(crate) struct Formatter<'a> {
    batch_serializer: <&'a mut serde_json::Serializer<Vec<u8>> as Serializer>::SerializeSeq,
    named_batch_counter: usize,
    files: Vec<Part>,
}

impl<'a> Formatter<'a> {
    fn new(
        serializer: <&'a mut serde_json::Serializer<Vec<u8>> as Serializer>::SerializeSeq,
    ) -> Self {
        Self {
            batch_serializer: serializer,
            named_batch_counter: 0,
            files: Vec::new(),
        }
    }

    fn finish_into_files(self) -> Result<Vec<Part>, FormatError> {
        self.batch_serializer
            .end()
            .map_err(FormatError::Serialization)?;
        Ok(self.files)
    }

    #[inline]
    fn add_binary_(&mut self, bin: Part) -> Cow<'static, str> {
        self.files.push(bin);

        let added = self.files.len();
        // Use saturating_sub to avoid panic on index
        Self::get_binary_name(added.saturating_sub(1))
    }

    #[inline]
    fn add_request_<B>(&mut self, req: &ARequest<B>) -> Result<(), FormatError>
    where
        B: AsRef<[u8]>,
    {
        self.batch_serializer
            .serialize_element(req)
            .map_err(FormatError::Serialization)
    }

    // Must only be called for non-stream bodies.
    #[inline]
    pub fn add_request<'b, R, B>(
        &'b mut self,
        req: R,
    ) -> Result<FormatAddRequest<'b, 'a, B>, FormatError>
    where
        R: TryInto<ARequest<B>>,
        FormatError: From<R::Error>,
        B: AsRef<[u8]>,
    {
        Ok(FormatAddRequest {
            f: self,
            request: req.try_into()?,
        })
    }

    #[inline]
    fn get_next_request_name(&mut self) -> Cow<'static, str> {
        let name = request_names(self.named_batch_counter);
        self.named_batch_counter += 1;
        name
    }

    #[inline]
    fn get_binary_name(index: usize) -> Cow<'static, str> {
        file_names(index)
    }
}

#[derive(Serialize, Default, Debug)]
pub(crate) struct ARequest<Body: AsRef<[u8]>> {
    #[serde(serialize_with = "serialize_method")]
    method: Method,

    #[serde(serialize_with = "serialize_url")]
    relative_url: UrlAccessTokenPair,

    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "encode_request_body"
    )]
    body: Option<Body>,

    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<Cow<'static, str>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    attached_files: Option<Cow<'static, str>>,
}

// This is done to avoid heap-allocations while making the url relative
// and while applying the access_token to the url using the sweet
// collect_str method
#[derive(Default, Debug)]
struct UrlAccessTokenPair {
    url: String,

    // keep the whole thing... we only need to read and that costs no allocation
    // unlike removing
    headers: reqwest::header::HeaderMap,
}

impl UrlAccessTokenPair {
    fn from_parts(url: Url, headers: HeaderMap) -> Self {
        Self {
            url: url.into(),
            headers,
        }
    }
}

impl Display for UrlAccessTokenPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(Batch::make_url_relative(&self.url))?;

        if let Some(access_token) = self.headers.get(AUTHORIZATION) {
            f.write_str("?access_token=")?;
            if let Ok(val) = access_token.to_str() {
                // Strip bearer prefix at display time
                if let Some(stripped) = val.strip_prefix("Bearer ") {
                    // No need for url encoding(the auth is already well-formatted)
                    f.write_str(stripped)?
                } else {
                    f.write_str(val)?
                }
            } else {
                // Rare case: non-UTF8, fall back to lossy
                let lossy = String::from_utf8_lossy(access_token.as_bytes());
                f.write_str(&lossy)?
            }
        }
        Ok(())
    }
}

fn serialize_url<S: Serializer>(url: &UrlAccessTokenPair, ser: S) -> Result<S::Ok, S::Error> {
    ser.collect_str(url)
}

fn serialize_method<S: Serializer>(method: &Method, ser: S) -> Result<S::Ok, S::Error> {
    ser.serialize_str(method.as_str())
}

/// Converts a top-level JSON object into a form-encoded string like `key=value&key2=value2`.
/// Borrows where possible, allocates only when JSON string unescaping is required.
fn encode_request_body<S: Serializer, Body: AsRef<[u8]>>(
    v: &Option<Body>,
    ser: S,
) -> Result<S::Ok, S::Error> {
    struct Params<'a> {
        // Using something as lazy as MapAcess here
        // would be the best
        outer_map: HashMap<&'a str, &'a RawValue>,
    }
    impl Display for Params<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.outer_map
                .iter()
                .enumerate()
                .try_for_each(|(i, (key, value))| {
                    let s = value.get();
                    let val: Cow<'_, str> = if s.starts_with('"') {
                        // Allocate only if there's need to unescape data
                        serde_json::from_str(s).unwrap_or(Cow::Borrowed(""))
                    } else {
                        Cow::Borrowed(s)
                    };
                    if i != 0 {
                        f.write_str("&")?;
                    }

                    let url_encoded = percent_encoding::utf8_percent_encode(&val, NON_ALPHANUMERIC);
                    f.write_fmt(format_args!("{key}={url_encoded}"))?;
                    Ok(())
                })
        }
    }

    match v {
        None => ser.serialize_none(),
        Some(v) => {
            let outer_map: HashMap<&str, &RawValue> =
                serde_json::from_slice(v.as_ref()).map_err(serde::ser::Error::custom)?;
            ser.collect_str(&Params { outer_map })
        }
    }
}

impl TryFrom<reqwest::RequestBuilder> for ARequest<JsonReqwestBody> {
    // mm not right
    type Error = FormatError;

    fn try_from(value: reqwest::RequestBuilder) -> Result<Self, Self::Error> {
        let request = value.build().map_err(FormatError::Reqwest)?;

        #[allow(dead_code)]
        pub struct Request {
            method: Method,
            url: Url,
            headers: HeaderMap,
            body: Option<reqwest::Body>,
            version: reqwest::Version,
            extensions: axum::http::Extensions,
        }

        let request = unsafe { std::mem::transmute::<reqwest::Request, Request>(request) };

        // This assumes the body is json.
        Ok(ARequest {
            method: request.method,
            body: if let Some(body) = request.body {
                // Slight check that we have bytes and json
                if body.as_bytes().is_some()
                    && request
                        .headers
                        .get(CONTENT_TYPE)
                        .is_some_and(|v| v == "application/json")
                {
                    Some(JsonReqwestBody(body))
                } else {
                    return Err(FormatError::InvalidBody);
                }
            } else {
                None
            },
            relative_url: UrlAccessTokenPair::from_parts(request.url, request.headers),
            ..Default::default()
        })
    }
}

#[derive(Default, Debug)]
pub(crate) struct JsonReqwestBody(reqwest::Body);

impl AsRef<[u8]> for JsonReqwestBody {
    fn as_ref(&self) -> &[u8] {
        // The check already occured in try_from
        self.0.as_bytes().unwrap()
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct RequestDebug {
    pub url: String,
}

#[must_use = "must eventually call `finish()`"]
pub(crate) struct FormatAddRequest<'a, 'b: 'a, B: AsRef<[u8]>> {
    f: &'a mut Formatter<'b>,
    request: ARequest<B>,
}

impl<'a, 'b: 'a, B: AsRef<[u8]>> FormatAddRequest<'a, 'b, B> {
    // Adds this to the list of binaries
    pub fn binary(mut self, part: Part) -> Self {
        let filename = self.f.add_binary_(part);

        // Reference the file
        self.request.attached_files = match self.request.attached_files {
            Some(names) => Some(format!("{names},{filename}").into()),
            None => Some(filename),
        };

        self
    }

    // Must be json... if you had key=value pass as {"key": "value"}... we can't
    // keep track of which is json or not so it's better to have json all-through
    // since it's the most used.
    pub fn raw_json<J>(self, body: J) -> FormatAddRequest<'a, 'b, J>
    where
        J: AsRef<[u8]>,
    {
        FormatAddRequest {
            f: self.f,
            request: ARequest {
                body: Some(body),
                method: self.request.method,
                relative_url: self.request.relative_url,
                name: self.request.name,
                attached_files: self.request.attached_files,
            },
        }
    }

    pub fn get_name(&mut self) -> Cow<'static, str> {
        self.request
            .name
            .get_or_insert_with(|| self.f.get_next_request_name())
            .clone()
    }

    #[cfg(not(debug_assertions))]
    pub fn finish(self) -> Result<(), FormatError> {
        self.f.add_request_(&self.request)
    }

    #[cfg(debug_assertions)]
    pub fn finish(self) -> Result<RequestDebug, FormatError> {
        self.f.add_request_(&self.request)?;
        Ok(RequestDebug {
            // Exclude auth
            url: self.request.relative_url.url,
        })
    }
}

// --- Error Handling & Utilities ---

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct BatchResponseError {
    inner: FromResponseError,
}

// A custom error type for failures during batch response reading.
// NOTE: This isn't the error encountered during actual parsing into your expected
#[derive(Debug, thiserror::Error)]
#[cfg_attr(debug_assertions, error("Error at endpoint '{endpoint}': {kind}"))]
#[cfg_attr(not(debug_assertions), error("{kind}"))]
pub(crate) struct FromResponseError {
    #[cfg(debug_assertions)]
    pub(crate) endpoint: Cow<'static, str>,
    pub(crate) kind: FromResponseErrorKind,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FromResponseErrorKind {
    #[error("Expected {want} response(s), but only got {got}")]
    InsufficientResponses { want: usize, got: usize },
    #[error(
        "The API returned a null response where a valid one was required. \
            If this request can be null, consider using `nullable()` or `then_nullable()`."
    )]
    NullNotNullable,
}

impl FromResponseError {
    pub(crate) fn insufficient(want: usize, got: usize) -> FromResponseErrorKind {
        FromResponseErrorKind::InsufficientResponses { want, got }
    }

    pub(crate) fn null_not_nullable() -> FromResponseErrorKind {
        FromResponseErrorKind::NullNotNullable
    }
}

impl FromResponseErrorKind {
    pub(crate) fn error(
        self,
        #[cfg(debug_assertions)] endpoint: Cow<'static, str>,
    ) -> FromResponseError {
        FromResponseError {
            #[cfg(debug_assertions)]
            endpoint,
            kind: self,
        }
    }
}

impl From<FromResponseError> for crate::Error {
    fn from(value: FromResponseError) -> Self {
        crate::Error::internal(value.into())
    }
}

// A custom error type for failures during batch formatting.
// TODO: Split appropriately
#[derive(Debug, thiserror::Error)]
pub(crate) enum FormatError {
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("The request body is not a valid JSON object")]
    InvalidBody,
}

impl From<std::convert::Infallible> for FormatError {
    fn from(value: std::convert::Infallible) -> Self {
        match value {}
    }
}

impl From<FormatError> for crate::Error {
    fn from(value: FormatError) -> Self {
        crate::Error::internal(value.into())
    }
}

#[cfg_attr(
    not(no_diagnostic_namespace),
    diagnostic::on_unimplemented(
        note = "Ensure the destructured parts match the group requests in amount/shape.",
    )
)]
pub trait FlattenFrom<FlattenFrom> {
    fn flatten_from(from: FlattenFrom) -> Self;
}

macro_rules! impl_flatten {
    {
        $(($($T:ident)+))*
    } => {
        $(
            impl_flatten_base!{$($T)+}
        )*
    }
}

macro_rules! impl_flatten_base {
    (
        $T:ident $($R:ident)*
    ) => {
        #[doc = "This trait is implemented for tuples up to sixteen items long."]
        impl<$T, $($R, )*> FlattenFrom<impl_flatten_base! {
            @stack
            |Rest|: $($R)*,
            |Stack|: $T
        }> for ($T, $($R, )*) {
            #[allow(non_snake_case)]
            #[inline]
            fn flatten_from(from: impl_flatten_base! {
                    @stack
                    |Rest|: $($R)*,
                    |Stack|: $T
                }) -> Self {
                // FIXME: Compile speed (done this twice)
                let impl_flatten_base! {
                    @stack
                    |Rest|: $($R)*,
                    |Stack|: $T
                } = from;
                ($T, $($R),*)

                // unsafe {
                //     std::mem::transmute(from)
                // }
            }
        }
    };
    {
        @stack
        |Rest|: $T:ident $($R:ident)*,
        |Stack|: $($s:tt)+
    } => {
        impl_flatten_base!{
            @stack
            |Rest|: $($R)*,
            |Stack|: ($($s)+, $T)
        }
    };
    {
        @stack
        |Rest|: ,
        |Stack|: $($s:tt)+
    } => {
        $($s)+
    }
}

impl_flatten! {
    (T0)
    (T0 T1)
    (T0 T1 T2)
    (T0 T1 T2 T3)
    (T0 T1 T2 T3 T4)
    (T0 T1 T2 T3 T4 T5)
    (T0 T1 T2 T3 T4 T5 T6)
    (T0 T1 T2 T3 T4 T5 T6 T7)
    (T0 T1 T2 T3 T4 T5 T6 T7 T8)
    (T0 T1 T2 T3 T4 T5 T6 T7 T8 T9)
    (T0 T1 T2 T3 T4 T5 T6 T7 T8 T9 T10)
    (T0 T1 T2 T3 T4 T5 T6 T7 T8 T9 T10 T11)
    (T0 T1 T2 T3 T4 T5 T6 T7 T8 T9 T10 T11 T12)
    (T0 T1 T2 T3 T4 T5 T6 T7 T8 T9 T10 T11 T12 T13)
    (T0 T1 T2 T3 T4 T5 T6 T7 T8 T9 T10 T11 T12 T13 T14)
    (T0 T1 T2 T3 T4 T5 T6 T7 T8 T9 T10 T11 T12 T13 T14 T15)
}

#[macro_export]
#[doc(hidden)]
macro_rules! reference {
    ($req_name:tt => $target_struct:ty => $([$($field_chain:tt)*])*) => {{
        #[allow(dead_code)]
        fn check_field(v: $target_struct) {
            let _ = v.$($($field_chain)*).*;
        }
        format!("{{result={}:$.{}}}", $req_name, stringify!($($($field_chain)*).*))
    }}
}

#[cfg(test)]
mod tests {
    use reqwest::RequestBuilder;
    use serde_json::json;

    use super::*;

    #[derive(Clone)]
    struct SomeRequest;

    impl Requests for SomeRequest {
        type BatchHandler = (); // should be !

        type ResponseReference = ();

        fn into_batch_ref(
            self,
            _: &mut Formatter,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            todo!()
        }

        requests_batch_include! {}
    }

    #[test]
    fn compile_check() {
        async fn _test(batch: Batch) {
            fn assert_is_requests<R: Requests>(_r: &R) {}

            assert_is_requests(&batch);

            let batch = batch.include(SomeRequest);
            assert_is_requests(&batch);

            let batch = batch.include(SomeRequest);
            assert_is_requests(&batch);

            let batch = batch.include(SomeRequest);
            assert_is_requests(&batch);

            let batch = batch.include(SomeRequest);
            assert_is_requests(&batch);

            let batch1 = batch.clone();
            let batch = batch.include(batch1);
            assert_is_requests(&batch);

            let batch = batch.include_iter([SomeRequest, SomeRequest]);
            assert_is_requests(&batch);
        }
    }

    #[test]
    fn test_traits() {
        // Here we check all Requests
        fn assert_is_requests<R>()
        where
            R: Requests,
            R::Include<SomeRequest>: Requests,
            R::BatchHandler: Handler,
        {
        }

        // App
        assert_is_requests::<crate::app::ConfigureWebhook>();

        // Catalog
        assert_is_requests::<crate::catalog::CreateProduct>();
        // assert_is_requests::<crate::catalog::ListProduct>();
        assert_is_requests::<crate::Update<'_, crate::catalog::ProductData>>();

        // Client
        assert_is_requests::<crate::client::SendMessage<'_>>();
        assert_is_requests::<crate::client::SetReplying<'_>>();
        assert_is_requests::<crate::client::SetRead<'_>>();
        assert_is_requests::<crate::client::UploadMedia>();
        assert_is_requests::<crate::client::DeleteMedia>();

        // Waba
        // assert_is_requests::<crate::waba::ListCatalog>();
        // assert_is_requests::<crate::waba::ListNumber>();
        // assert_is_requests::<crate::waba::ListApp>();

        // TODO: Add module wrappers
    }

    #[test]
    fn test_make_url_relative() {
        assert_eq!(
            Batch::make_url_relative("https://graph.facebook.com/v0.1/something"),
            "/v0.1/something"
        );
        assert_eq!(
            Batch::make_url_relative("https://graph.facebook.com/v0.12/something"),
            "/v0.12/something"
        );
        assert_eq!(Batch::make_url_relative("https://graph.facebook.com/"), "/");
        assert_eq!(Batch::make_url_relative("https://graph.facebook.com"), "/");
    }

    pub struct RawRequest {
        pub request: reqwest::RequestBuilder,
    }

    impl RawRequest {
        fn new(request: reqwest::RequestBuilder) -> Self {
            Self { request }
        }

        fn request(self) -> reqwest::RequestBuilder {
            self.request
        }
    }

    crate::SimpleOutputBatch! {
        RawRequest <> => ()
    }

    #[derive(Clone)]
    pub struct RawRequestResponseReference {
        _priv: (),
    }

    impl crate::batch::IntoResponseReference for RawRequest {
        type ResponseReference = RawRequestResponseReference;

        fn into_response_reference(_: Cow<'static, str>) -> Self::ResponseReference {
            Self::ResponseReference { _priv: () }
        }
    }

    macro_rules! batch_part {
        ($json:literal) => {{
            let mut form = Form::new();
            form = form.part("batch", Part::bytes(Cow::Borrowed($json.as_bytes())));
            form
        }};
    }

    use futures::{AsyncReadExt as _, TryStreamExt as _};

    fn unify_boundary(form1: Form, form2: Form) -> (Form, Form) {
        #[allow(dead_code)]
        pub struct FormR {
            inner: FormPartsR<Part>,
        }
        #[allow(dead_code)]
        pub(crate) struct FormPartsR<P> {
            pub(crate) boundary: String,
            pub(crate) computed_headers: Vec<Vec<u8>>,
            pub(crate) fields: Vec<(Cow<'static, str>, P)>,
            pub(crate) percent_encoding: PercentEncoding,
        }
        #[allow(dead_code)]
        pub(crate) enum PercentEncoding {
            PathSegment,
            AttrChar,
            NoOp,
        }

        let mut form1 = unsafe { std::mem::transmute::<Form, FormR>(form1) };

        let mut form2 = unsafe { std::mem::transmute::<Form, FormR>(form2) };

        form1.inner.boundary = "boundary".to_owned();
        form2.inner.boundary = "boundary".to_owned();

        let form1 = unsafe { std::mem::transmute::<FormR, Form>(form1) };

        let form2 = unsafe { std::mem::transmute::<FormR, Form>(form2) };

        (form1, form2)
    }

    macro_rules! assert_multipart_eq {
        ($a:expr, $b:expr) => {{
            let (a, b) = unify_boundary($a, $b);
            let mut a = a
                .into_stream()
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
                .into_async_read();

            let mut b = b
                .into_stream()
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
                .into_async_read();

            let mut a_out = Vec::new();
            a.read_to_end(&mut a_out).await.unwrap();

            let mut b_out = Vec::new();
            b.read_to_end(&mut b_out).await.unwrap();

            assert_eq!(a_out, b_out);
        }};
    }

    // Helper function to create a mock Client instance for testing.
    async fn mock_client() -> Client {
        // A dummy client is sufficient, as we won't be making real network calls.
        Client::new("Auth").await.unwrap()
    }

    fn mock_http_client() -> reqwest::Client {
        reqwest::Client::new()
    }

    /// Verifies that multiple requests are correctly serialized into a JSON array.
    #[tokio::test]
    async fn test_batch_serialization_with_multiple_requests() {
        let client = mock_client().await;
        let http_client = mock_http_client();

        // Define two different requests.
        let get_req = http_client.get("http://example.com/api/v1/users/1");
        let post_req = http_client
            .post("http://example.com/api/v1/posts")
            .json(&json!({ "title": "Test Post&=" }));

        // Create a batch and join the requests.
        let batch = client
            .batch()
            .include(RawRequest::new(get_req))
            .include(RawRequest::new(post_req));

        // Execute the batch to get the serialized bytes.
        let request_form = batch.execute().state.unwrap().form;

        let expected_form = batch_part!(
            r#"[{"method":"GET","relative_url":"/api/v1/users/1"},{"method":"POST","relative_url":"/api/v1/posts","body":"title=Test%20Post%26%3D"}]"#
        ).text("include_headers", "false");

        assert_multipart_eq!(request_form, expected_form)
    }

    /// Verifies that authentication is correctly added per requests
    #[tokio::test]
    async fn test_individual_auth() {
        let client = mock_client().await;
        let http_client = mock_http_client();

        // Define two different requests.
        let get_req = http_client
            .get("http://example.com/api/v1/users/1")
            .bearer_auth("TOKEN");
        let post_req = http_client
            .post("http://example.com/api/v1/posts")
            .json(&json!({ "title": "Test Post" }));

        // Create a batch and join the requests.
        let batch = client
            .batch()
            .include(RawRequest::new(get_req))
            .include(RawRequest::new(post_req));

        // Execute the batch to get the serialized bytes.
        let request_form = batch.execute().state.unwrap().form;
        let expected_form = batch_part!(
            r#"[{"method":"GET","relative_url":"/api/v1/users/1?access_token=TOKEN"},{"method":"POST","relative_url":"/api/v1/posts","body":"title=Test%20Post"}]"#
        ).text("include_headers", "false");

        assert_multipart_eq!(request_form, expected_form)
    }

    /// Checks that the Many iterator correctly handles a collection of requests.
    #[tokio::test]
    async fn test_many_request_handling() {
        let client = mock_client().await;
        let http_client = mock_http_client();

        let user_ids = vec![1, 2, 3];
        let requests = user_ids.into_iter().map(|id| {
            RawRequest::new(http_client.get(format!("http://example.com/api/v1/users/{}", id)))
        });

        let batch = client.batch().include_iter(requests);

        let request_form = batch.execute().state.unwrap().form;
        let expected_form = batch_part!(
            r#"[{"method":"GET","relative_url":"/api/v1/users/1"},{"method":"GET","relative_url":"/api/v1/users/2"},{"method":"GET","relative_url":"/api/v1/users/3"}]"#
        ).text("include_headers", "false");

        assert_multipart_eq!(request_form, expected_form)
    }

    /// Verifies that binary files are correctly attached and referenced.
    #[tokio::test]
    async fn test_binary_file_attachment() {
        let client = mock_client().await;
        let http_client = mock_http_client();

        struct UploadMedia {
            request: RequestBuilder,
            media: Part,
        }

        impl Requests for UploadMedia {
            type BatchHandler = ();

            type ResponseReference = ();

            fn into_batch_ref(
                self,
                f: &mut Formatter,
            ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
                f.add_request(self.request)?.binary(self.media).finish()?;
                Ok(((), ()))
            }

            requests_batch_include! {}
        }

        let request = http_client.post("http://example.com/upload");
        let media = Part::bytes(b"some file content").file_name("test_file.txt");
        let batch = client.batch().include(UploadMedia { media, request });

        // Execute the batch to get the serialized bytes.
        let request_form = batch.execute().state.unwrap().form;
        let expected_form =
            batch_part!(r#"[{"method":"POST","relative_url":"/upload","attached_files":"file"}]"#)
                .part(
                    "file",
                    Part::bytes(b"some file content").file_name("test_file.txt"),
                )
                .text("include_headers", "false");

        assert_multipart_eq!(request_form, expected_form)
    }

    /// Ensures serialization failures are caught and propagated.
    #[tokio::test]
    async fn test_error_propagation_on_serialization_failure() {
        let client = mock_client().await;
        let http_client = mock_http_client();

        let get_req = http_client.get("http://example.com/ok");

        // We need to simulate a serialization failure, which is hard with valid data.
        // Let's pass a non-object json
        let post_req_with_bad_body = http_client.post("http://example.com/bad").json(&1);

        // Join one good request and one bad one.
        let batch = client
            .batch()
            .include(RawRequest::new(get_req))
            .include(RawRequest::new(post_req_with_bad_body));

        // The execute call should fail because of the invalid request.
        let result = batch.execute().state;

        // The result should be an error.
        match result {
            Err(FormatError::Serialization(_)) => {} // We expect a serialization error.
            _ => panic!("Expected a Serialization error, but got a different one."),
        }
    }
}
