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

use reqwest::{
    Method, StatusCode,
    multipart::{Form, Part},
};
use serde::{Deserialize, Serialize, Serializer, ser::SerializeSeq as _};
use std::{borrow::Cow, fmt::Display, marker::PhantomData};

use crate::{
    Auth, Client, FromResponseOwned, ToValue,
    client::{Endpoint, JsonObjectPayload, PendingRequest},
    rest::execute_multipart_request,
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
    #[inline]
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
            Err(err) => return BatchExecute::from_err(err),
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
    #[inline]
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
    #[inline]
    pub fn include_iter<I, R>(self, iter: I) -> Batch<Rs::Include<Many<I::IntoIter>>>
    where
        I: IntoIterator<Item = R>,
    {
        self.include(Many::new(iter.into_iter()))
    }

    /// Unwraps the Batch, returning the underlying requests.
    pub fn into_inner(self) -> Rs {
        self.requests
    }

    /// Consumes the `Batch` builder and prepares it for execution.
    ///
    /// This method formats all included requests into a single multipart form
    /// payload and returns an `ExecuteBatch` future. You must `.await` this
    /// future to send the request.
    #[inline]
    pub fn execute(self) -> BatchExecute<Rs::BatchHandler> {
        let (size_hint, _) = self.requests.size_hint();

        // Use a conservative heuristic of 256 bytes per request for pre-allocation.
        const AVERAGE_REQ_SIZE: usize = 256;
        let json_buffer = Vec::with_capacity(size_hint * AVERAGE_REQ_SIZE);

        // Set up the JSON serializer.
        let mut serializer = serde_json::Serializer::new(json_buffer);
        let seq_serializer = tri_batch!(
            serializer
                .serialize_seq(Some(size_hint))
                .map_err(FormatError::Serialization)
        );

        // Use the dedicated serializer to format the batch requests.
        let mut batch_serializer = BatchSerializer::new(seq_serializer);
        let handler = tri_batch!(self.requests.into_batch(&mut batch_serializer));

        // Finalize the serialization and retrieve the JSON bytes and attached files.
        let files = tri_batch!(batch_serializer.finish());
        let json_bytes = serializer.into_inner();

        // Create the multipart parts.
        let batch_part = Part::bytes(json_bytes);

        BatchExecute::build(batch_part, files, handler, self.client)
    }
}

/// Internal state for a pending batch execution.
struct BatchExecuteState<Handler> {
    request: PendingRequest<'static>,
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
    // Using Result here allows us to capture any errors that occur during
    // the build phase and report them only when executed.
    state: Result<BatchExecuteState<Handler>, FormatError>,
}

impl<Handler> BatchExecute<Handler> {
    /// Internal constructor to create a `BatchExecute` from its component parts.
    #[inline]
    fn build(
        batch_part: Part,
        files: Vec<(Cow<'static, str>, Part)>,
        handler: Handler,
        client: Client,
    ) -> Self {
        const BATCH_FIELD_NAME: &str = "batch";

        // Initialize the form with the main JSON batch payload.
        let mut form = Form::new().part(BATCH_FIELD_NAME, batch_part);

        // Add all attached files to the form with their specified names.
        for (name, file_part) in files {
            form = form.part(name, file_part);
        }

        // The batch API endpoint is at the root of the graph API.
        const ROOT_ENDPOINT: Endpoint<'static> = Endpoint::without_version();
        let request = client.post(ROOT_ENDPOINT);

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
    fn from_err(err: FormatError) -> Self {
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
        const INCLUDE_HEADERS_FIELD: &str = "include_headers";

        if let Ok(mut state) = self.state {
            state.form = state.form.text(
                INCLUDE_HEADERS_FIELD,
                if include_headers { "true" } else { "false" },
            );
            self.state = Ok(state);
            self
        } else {
            self
        }
    }

    /// Overrides the default authentication for the entire batch request.
    ///
    /// This sets the `access_token` for the top-level multipart request.
    /// This is different from calling `.with_auth()` on an individual request
    /// before adding it to the batch.
    pub fn with_auth<'a>(mut self, auth: impl ToValue<'a, Auth>) -> Self {
        if let Ok(state) = &mut self.state {
            state.request.auth = Some(Cow::Owned(auth.to_value().into_owned()));
        }
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
    pub fn execute(self) -> impl Future<Output = Result<BatchOutput<H>, crate::Error>> + 'static {
        #[cfg(debug_assertions)]
        let handle = async |endpoint: String, response| {
            Client::handle_response(response, endpoint.into()).await
        };

        #[cfg(not(debug_assertions))]
        let handle = async |response| {
            Client::handle_response(response).await
        };

        async move {
            let state = self.state?;
            let response
                = execute_multipart_request(state.request, state.form, handle).await?;


            Ok(BatchOutput::from_parts(state.handler, response))
        }
    }
}
}

// SECTION: Traits
// ================

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
    fn into_batch_ref(
        self,
        batch_serializer: &mut BatchSerializer,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError>;

    /// Formats the request(s) and writes them to the [`BatchSerializer`].
    ///
    /// This method is responsible for serializing the request's JSON representation
    /// and handling any binary attachments.
    //
    // Implementation NOTE: An optimized version should be implemented to avoid
    // bloating request with unused request names
    #[inline]
    fn into_batch(
        self,
        batch_serializer: &mut BatchSerializer,
    ) -> Result<Self::BatchHandler, FormatError>
    where
        Self: Sized,
    {
        let (h, _) = self.into_batch_ref(batch_serializer)?;
        Ok(h)
    }

    /// The resulting type when this request is joined with another.
    ///
    /// This is typically a nested tuple, like `(Self, R)`, which allows the compiler
    /// to track the sequence of requests.
    type Include<R>;

    /// Joins this request group with another.
    fn include<R>(self, r: R) -> Self::Include<R>;

    /// Provides a hint about the number of individual requests contained within this object.
    ///
    /// This is used for pre-allocating capacity to improve performance.
    #[inline]
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
    #[inline]
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
    #[inline]
    fn then_nullable<R, F>(self, f: F) -> ThenNullable<Self, R, F>
    where
        Self: Nullable + Sized,
        F: FnOnce(Self::ResponseReference) -> R,
        R: Requests,
    {
        Then::new(self.nullable(), f)
    }

    /// Transforms the successful output of a request after execution.
    ///
    /// Unlike `.then()`, which chains requests together *before* they are sent, `map`
    /// is used for post-processing the *result* of a request after the batch has
    /// been successfully executed and its response parsed.
    ///
    /// The closure `f` receives the final output of the request (typically a `Result`)
    /// and can transform it into any other type `U`. This allows you to adapt the
    /// response to your application's needs, perform side-effects like logging, or
    /// forward the result to other routines.
    ///
    /// ## Key Differences from `.then()`
    ///
    /// - **Execution Time**: `.map()` runs *after* the API call. `.then()` runs *before*,
    ///   during the construction of the batch request.
    /// - **Closure Input**: The closure for `.map()` receives the actual, parsed response
    ///   data. The closure for `.then()` receives a `ResponseReference`, a symbolic
    ///   placeholder for a future result.
    /// - **Purpose**: Use `.map()` to process a response. Use `.then()` to create a new
    ///   request that depends on a previous one within the same batch.
    ///
    /// ### Behavior on Batch Failure
    ///
    /// It's important to understand how `map` interacts with the two possible types of errors:
    ///
    /// - **Request-Level Error**: If the overall batch API call is successful but the
    ///   *specific request within it fails* (e.g., sending to an invalid number),
    ///   the `result` passed to your closure will be `Err(Error)`. **Your `.map()`
    ///   closure will still run.**
    ///
    /// - **Batch-Level Error**: If the entire batch request fails catastrophically
    ///   (e.g., due to a network error or invalid credentials), the top-level
    ///   `.execute()` method will return an `Err`. In this scenario, **your `.map()`
    ///   closure will not run at all.**
    ///
    /// This is especially crucial for patterns involving side-effects. For example, if
    /// you're sending a result to a channel, the receiving end must handle the channel
    /// being closed unexpectedly (`RecvError`), as this is the signal that a batch-level
    /// error occurred and the closure was dropped before it could execute.
    ///
    /// # Example: Sending a result to a channel
    ///
    /// This example shows how to send a message and, upon a successful response,
    /// forward the resulting message ID to another part of the application via a channel.
    /// The final output of the request itself is mapped to `()`, as we've already
    /// handled the data we care about.
    ///
    /// ```rust,no_run
    /// # use whatsapp_business_rs::{Client, Error, batch::Requests, message::MessageCreate};
    /// # use std::sync::mpsc::channel;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("YOUR_API_KEY").await?;
    /// // Create a channel to send results to another part of the application.
    /// let (tx, rx) = channel::<String>();
    ///
    /// let request = client.message("SENDER")
    ///     .send("RECIPIENT", "Hello from the batch!")
    ///     .map(move |result: Result<MessageCreate, Error>| {
    ///         // This closure runs after the request is executed and parsed.
    ///         if let Ok(response) = result {
    ///             println!("Successfully sent message, forwarding ID...");
    ///             // Send the ID to another thread.
    ///             tx.send(response.message_id().to_string()).unwrap();
    ///         }
    ///         // The original result is consumed and we return unit `()` instead.
    ///         ()
    ///     });
    ///
    /// // Execute the request. The output for this request in the batch will now be `()`.
    /// let _ = client.batch().include(request).execute().await?;
    ///
    /// // The other part of the app can now receive the message ID.
    /// let received_id = rx.try_recv().unwrap();
    /// assert!(!received_id.is_empty());
    /// println!("Received message ID via channel: {}", received_id);
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    fn map<U, F>(self, f: F) -> Map<Self, F>
    where
        F: FnOnce(<Self::BatchHandler as Handler>::Responses) -> U,
        Self: Sized,
        Self::BatchHandler: Handler,
    {
        Map::new(self, f)
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
    fn from_batch(
        self,
        response: &mut BatchResponse,
    ) -> Result<Self::Responses, ResponseProcessingError>;
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

// Static arrays for named file and request parts to avoid heap allocations for small batches.
static_names! {
    FILE_NAMES "file" [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]
}

static_names! {
    REQUEST_NAMES "request" [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]
}

// SECTION: Trait Implementations
// ==============================

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

        #[inline]
        fn into_batch_ref(
            self,
            f: &mut BatchSerializer,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            let (h1, r1) = self.0.into_batch_ref(f)?;
            let (h2, r2) = self.1.into_batch_ref(f)?;
            Ok(((h1, h2), (r1, r2)))
        }

        #[inline]
        fn into_batch(self, f: &mut BatchSerializer) -> Result<Self::BatchHandler, FormatError> {
            let h1 = self.0.into_batch(f)?;
            let h2 = self.1.into_batch(f)?;
            Ok((h1, h2))
        }

        #[inline]
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

        #[inline]
        fn from_batch(
            self,
            response: &mut BatchResponse,
        ) -> Result<Self::Responses, ResponseProcessingError> {
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

        #[inline]
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
            _: &mut BatchSerializer,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            Ok(((), ()))
        }

        fn into_batch(self, _f: &mut BatchSerializer) -> Result<Self::BatchHandler, FormatError> {
            Ok(())
        }

        /// When a request `R` is included in an empty batch, the batch becomes `R`.
        type Include<R> = R;

        fn include<R>(self, r: R) -> Self::Include<R> {
            r
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (0, None)
        }
    }

    /// Base case implementation for an empty handler (`()`).
    impl Handler for () {
        type Responses = ();

        fn from_batch(
            self,
            _response: &mut BatchResponse,
        ) -> Result<Self::Responses, ResponseProcessingError> {
            Ok(())
        }
    }

    impl Nullable for () {
        type Nullable = ();

        fn nullable(self) -> Self::Nullable {}
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

        #[inline]
        fn into_batch_ref(
            self,
            f: &mut BatchSerializer,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            match self {
                Some(r) => {
                    let (h, r) = r.into_batch_ref(f)?;
                    Ok((Some(h), Some(r)))
                }
                None => Ok((None, None)),
            }
        }

        #[inline]
        fn into_batch(self, f: &mut BatchSerializer) -> Result<Self::BatchHandler, FormatError> {
            match self {
                Some(r) => Ok(Some(r.into_batch(f)?)),
                None => Ok(None),
            }
        }

        #[inline]
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

        #[inline]
        fn from_batch(
            self,
            response: &mut BatchResponse,
        ) -> Result<Self::Responses, ResponseProcessingError> {
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

        #[inline]
        fn nullable(self) -> Self::Nullable {
            self.map(|t| t.nullable())
        }
    }

    SimpleOutputBatch! {
        Update <'a, [T: Serialize + Send + 'static], [U: FromResponseOwned + 'static]> => U
    }
}

impl<Rs> Requests for Batch<Rs>
where
    Rs: Requests,
{
    type BatchHandler = Rs::BatchHandler;

    type ResponseReference = Rs::ResponseReference;

    #[inline]
    fn into_batch_ref(
        self,
        batch_serializer: &mut BatchSerializer,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
        self.requests.into_batch_ref(batch_serializer)
    }

    #[inline]
    fn into_batch(
        self,
        batch_serializer: &mut BatchSerializer,
    ) -> Result<Self::BatchHandler, FormatError> {
        self.requests.into_batch(batch_serializer)
    }

    #[inline]
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

    #[inline]
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
    #[inline]
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

    #[inline]
    fn into_batch_ref(
        self,
        batch_serializer: &mut BatchSerializer,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
        let (size, _) = self.iter.size_hint();
        let mut handlers = Vec::with_capacity(size);
        let mut references = Vec::with_capacity(size);
        for request in self.iter {
            let (h, r) = request.into_batch_ref(batch_serializer)?;
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

    #[inline]
    fn into_batch(
        self,
        batch_serializer: &mut BatchSerializer,
    ) -> Result<Self::BatchHandler, FormatError> {
        // We should not have any allocation in most cases in release builds since most
        // handlers are zero sized
        let (size, _) = self.iter.size_hint();
        let mut handlers = Vec::with_capacity(size);
        for request in self.iter {
            handlers.push(request.into_batch(batch_serializer)?)
        }

        Ok(ManyHandlers { handlers })
    }

    #[inline]
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
    #[inline]
    pub fn nullable(self) -> Many<impl Iterator<Item = T::Nullable>> {
        Many {
            iter: self.iter.map(|t| t.nullable()),
        }
    }
}

#[cfg(nightly_rust)]
impl<I, T> Nullable for Many<I>
where
    I: Iterator<Item = T>,
    T: Nullable,
{
    type Nullable = Many<impl Iterator<Item = T::Nullable>>;

    #[inline]
    fn nullable(self) -> Self::Nullable {
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

    #[inline]
    fn from_batch(
        self,
        response: &mut BatchResponse,
    ) -> Result<Self::Responses, ResponseProcessingError> {
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

    #[inline]
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

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.responses.next()
    }
}

/// A dependent request where the initial request's response can be null.    
pub type ThenNullable<R1, R2, F = fn(<R1 as Requests>::ResponseReference) -> R2> =
    Then<<R1 as Nullable>::Nullable, R2, F>;

/// Used to create a dependent request chain for a batch operation.
/// It encapsulates a request (r1) and a closure (f) that will be executed later.
/// This allows you to build a graph of requests where the output of one request
/// (represented by a ResponseReference) becomes the input for a subsequent request.
/// The entire chain is then sent to the server in a single, efficient batch.
#[derive(Debug)]
pub struct Then<R1, R2, F = fn(<R1 as Requests>::ResponseReference) -> R2> {
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

    #[inline]
    fn into_batch_ref(
        self,
        batch_serializer: &mut BatchSerializer,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
        let (h, r_ref) = self.r1.into_batch_ref(batch_serializer)?;

        let (h1, r1_ref) = (self.f)(r_ref).into_batch_ref(batch_serializer)?;

        // R entered before R1
        Ok(((h, h1), r1_ref))
    }

    #[inline]
    fn into_batch(
        self,
        batch_serializer: &mut BatchSerializer,
    ) -> Result<Self::BatchHandler, FormatError> {
        let (h, r_ref) = self.r1.into_batch_ref(batch_serializer)?;

        let h1 = (self.f)(r_ref).into_batch(batch_serializer)?;

        // R entered before R1
        Ok((h, h1))
    }

    #[inline]
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
    #[inline]
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

#[cfg(nightly_rust)]
impl<R1, R2, F> Nullable for Then<R1, R2, F>
where
    R1: Nullable + Requests,
    F: FnOnce(R1::ResponseReference) -> R2,
    R2: Nullable,
{
    type Nullable =
        Then<R1::Nullable, R2::Nullable, impl FnOnce(R1::ResponseReference) -> R2::Nullable>;

    #[inline]
    fn nullable(self) -> Self::Nullable {
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
    #[inline]
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

/// A request combined with a function to transform its output.
///
/// This struct is created by the [`.map()`](Requests::map) method. It holds the original
/// request and the closure that will be applied to the response after execution.
/// You will likely not need to interact with this type directly.
#[derive(Debug)]
pub struct Map<R, F> {
    r: R,
    f: F,
}

impl<R, F> Map<R, F> {
    #[inline]
    pub fn new(r: R, f: F) -> Self {
        Self { r, f }
    }
}

impl<R, F, U> Requests for Map<R, F>
where
    R: Requests,
    F: FnOnce(<R::BatchHandler as Handler>::Responses) -> U,
    R::BatchHandler: Handler,
{
    type BatchHandler = MapHandler<R::BatchHandler, F>;

    // This doesn't change the actual response from the server
    type ResponseReference = R::ResponseReference;

    #[inline]
    fn into_batch_ref(
        self,
        batch_serializer: &mut BatchSerializer,
    ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
        let (h, r_ref) = self.r.into_batch_ref(batch_serializer)?;

        Ok((
            MapHandler {
                inner: h,
                f: self.f,
            },
            r_ref,
        ))
    }

    #[inline]
    fn into_batch(
        self,
        batch_serializer: &mut BatchSerializer,
    ) -> Result<Self::BatchHandler, FormatError> {
        let h = self.r.into_batch(batch_serializer)?;
        Ok(MapHandler {
            inner: h,
            f: self.f,
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.r.size_hint()
    }

    requests_batch_include! {}
}
// NOTE: Nullable for Map seems impossible... code

#[derive(Debug)]
pub struct MapHandler<H, F> {
    inner: H,
    f: F,
}

impl<H, F, U> Handler for MapHandler<H, F>
where
    H: Handler,
    F: FnOnce(H::Responses) -> U,
{
    type Responses = U;

    #[inline]
    fn from_batch(
        self,
        response: &mut BatchResponse,
    ) -> Result<Self::Responses, ResponseProcessingError> {
        // We don't expose the error to prevent error-swallowing
        let response = self.inner.from_batch(response)?;
        Ok((self.f)(response))
    }
}

// SECTION: Response Handling
// ===========================

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
    #[inline]
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
    #[inline]
    pub fn flatten<F>(self) -> Result<F, BatchResponseError>
    where
        F: FlattenFrom<H::Responses>,
    {
        self.result().map(FlattenFrom::flatten_from)
    }
}

impl<H> BatchOutput<H> {
    #[inline]
    fn from_parts(handler: H, response: Vec<Option<BatchSubResponse>>) -> Self {
        Self {
            handler,
            response: BatchResponse::new(response),
        }
    }
}

/// Represents the deserialized response from a batch request API call.
///
/// This struct acts as an iterator over the individual sub-responses contained
/// within the batch response payload.
#[derive(Debug)]
pub(crate) struct BatchResponse {
    /// An iterator over the potentially null sub-responses.
    responses: std::vec::IntoIter<Option<BatchSubResponse>>,
}

impl BatchResponse {
    /// Creates a new `BatchResponse` from a vector of deserialized sub-responses.
    pub fn new(responses: Vec<Option<BatchSubResponse>>) -> Self {
        Self {
            responses: responses.into_iter(),
        }
    }

    /// Processes the next sub-response, attempting to deserialize it into a specific type `T`.
    ///
    /// This is a convenience method that encapsulates the common pattern of handling the
    /// three possible states of a sub-response: a valid response, a `null` response, or an error.
    ///
    /// # Arguments
    /// * `endpoint`: The endpoint of the original sub-request. This is used to create
    ///   a more informative error message if something goes wrong.
    ///
    /// # Returns
    /// * `Ok(Ok(T))`: Successful deserialization.
    /// * `Ok(Err(crate::Error))`: The API returned a non-success status code (e.g., 404, 500).
    /// * `Err(ResponseProcessingError)`: An error occurred while processing the batch response
    ///   itself (e.g., not enough responses, or a `null` was received for a non-nullable request).
    #[inline]
    pub fn try_next<T: FromResponseOwned>(
        &mut self,
        #[cfg(debug_assertions)] endpoint: Cow<'static, str>,
    ) -> Result<Result<T, crate::Error>, ResponseProcessingError> {
        match self.next() {
            // A valid sub-response was found.
            Some(Some(sub_response)) => {
                let status = sub_response.status();
                let body = sub_response.body();

                // Delegate to the main client response handler.
                Ok(Client::handle_response_(
                    status,
                    body.as_bytes(),
                    #[cfg(debug_assertions)]
                    endpoint,
                ))
            }
            // A `null` was present in the response array.
            Some(None) => Err(ResponseProcessingError::new(
                endpoint,
                ResponseProcessingErrorKind::NullNotNullable,
            )),
            // The iterator is exhausted, but a response was expected.
            None => Err(ResponseProcessingError::new(
                #[cfg(debug_assertions)]
                endpoint,
                ResponseProcessingErrorKind::InsufficientResponses {
                    // Note: We can't know the `want` and `got` counts from here,
                    // but the context implies we wanted at least one more.
                    want: 1,
                    got: 0,
                },
            )),
        }
    }
}

impl Iterator for BatchResponse {
    /// The outcome of iterating over a single sub-response slot.
    ///
    /// - `Ok(Some(BatchSubResponse))`: A valid response object was present.
    /// - `Ok(None)`: A `null` was present, often because the request was a dependency for another.
    /// - `Err(ResponseProcessingError)`: A fundamental error in processing the response stream.
    ///
    /// NOTE: No longer returning Result. The former result was only symbolic but took 2x the space
    /// of what we have now.
    type Item = Option<BatchSubResponse>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.responses.next()
    }
}

/// A single response object within the main batch response array.
/// The `Body` is generic to allow for different response body types if needed.
#[derive(Deserialize, Debug)]
pub(crate) struct BatchSubResponse<Body = BatchResponseBody> {
    #[serde(rename = "code")]
    status_code: u16,
    body: Body,
}

/// The default type for the body of a sub-response, a heap-allocated string slice.
pub type BatchResponseBody = Box<str>;

impl<Body> BatchSubResponse<Body> {
    /// Returns the HTTP status code of the sub-response.
    /// Falls back to the default `StatusCode` (200 OK) if the code is invalid.
    pub fn status(&self) -> StatusCode {
        StatusCode::from_u16(self.status_code).unwrap_or_default()
    }

    /// Consumes the sub-response and returns its body.
    pub fn body(self) -> Body {
        self.body
    }
}

// SECTION: Serialization Logic for Batch Requests
// ===============================================

/// Handles the serialization of a sequence of sub-requests into a JSON array string.
/// It also collects all binary file parts that need to be attached to the final
/// multipart request.
#[must_use = "finish must be called to finalize serialization"]
pub(crate) struct BatchSerializer<'a> {
    // The serde serializer for the top-level JSON array of requests.
    seq_serializer: <&'a mut serde_json::Serializer<Vec<u8>> as Serializer>::SerializeSeq,
    // Counter to generate unique names for requests that need to be referenced.
    named_request_counter: usize,
    // Counter to generate unique names for attached files.
    file_attachment_counter: usize,
    // A collection of all file parts to be included in the multipart form.
    // Each tuple holds the form field name and the file `Part` itself.
    files: Vec<(Cow<'static, str>, Part)>,
}

impl<'a> BatchSerializer<'a> {
    fn new(
        serializer: <&'a mut serde_json::Serializer<Vec<u8>> as Serializer>::SerializeSeq,
    ) -> Self {
        Self {
            seq_serializer: serializer,
            named_request_counter: 0,
            file_attachment_counter: 0,
            files: Vec::new(),
        }
    }

    /// Finalizes the JSON array serialization and returns the collected files.
    fn finish(self) -> Result<Vec<(Cow<'static, str>, Part)>, FormatError> {
        // Correctly ends the JSON array `]`.
        self.seq_serializer
            .end()
            .map_err(FormatError::Serialization)?;
        Ok(self.files)
    }

    /// Serializes a single sub-request into the JSON array.
    #[inline]
    fn serialize_request<P, Q>(
        &mut self,
        request: &BatchSubRequest<'_, P, Q>,
    ) -> Result<(), FormatError>
    where
        P: Serialize,
        Q: Serialize,
    {
        self.seq_serializer
            .serialize_element(request)
            .map_err(FormatError::Serialization)
    }

    /// Adds a file part to the collection and returns the name assigned to it.
    /// The name will be used in the `attached_files` field of the sub-request JSON.
    #[inline]
    fn add_attachment(&mut self, name: Option<Cow<'static, str>>, part: Part) -> Cow<'static, str> {
        // If a name is provided, use it. Otherwise, generate a unique one.
        let file_name = name.unwrap_or_else(|| {
            let name = file_names(self.file_attachment_counter);
            self.file_attachment_counter += 1;
            name
        });

        self.files.push((file_name.clone(), part));
        file_name
    }

    /// Gets the next unique name for a request (e.g., "request0", "request1").
    #[inline]
    fn get_next_request_name(&mut self) -> Cow<'static, str> {
        let name = request_names(self.named_request_counter);
        self.named_request_counter += 1;
        name
    }

    /// Creates a formatter for a single sub-request.
    /// This is the entry point for formatting a `PendingRequest` into the batch.
    #[inline]
    pub fn format_request<'b, P, Q>(
        &'b mut self,
        req: impl Into<BatchSubRequest<'a, P, Q>>,
    ) -> SingleRequestFormatter<'b, 'a, P, Q> {
        SingleRequestFormatter {
            serializer: self,
            sub_request: req.into(),
        }
    }
}

/// A builder-style formatter for a *single* sub-request within the batch.
///
/// It allows for chaining modifications like adding a JSON body or attaching files
/// before finally calling `.finish()` to serialize it into the main batch array.
#[must_use = "must call .finish() to add the request to the batch"]
pub(crate) struct SingleRequestFormatter<'a, 'b: 'a, P, Q> {
    serializer: &'a mut BatchSerializer<'b>,
    sub_request: BatchSubRequest<'a, P, Q>,
}

impl<'a, 'b: 'a, P, Q> SingleRequestFormatter<'a, 'b, P, Q>
where
    P: Serialize,
    Q: Serialize,
{
    /// Attaches a binary file to this sub-request.
    ///
    /// You can provide an optional `name` if the API endpoint requires a specific
    /// field name (e.g., "file"). If `None`, a unique name will be generated.
    #[inline]
    pub fn attach_file(mut self, name: Option<Cow<'static, str>>, part: Part) -> Self {
        // Add the file to the main serializer's collection and get its assigned name.
        let file_name = self.serializer.add_attachment(name, part);

        // Append the file's name to the 'attached_files' field.
        // The API expects a comma-separated string of file names.
        match self.sub_request.attached_files {
            Some(ref mut names) => {
                // Already have attached files, so append with a comma.
                let mut mutable_names = names.to_string();
                mutable_names.push(',');
                mutable_names.push_str(&file_name);
                *names = Cow::Owned(mutable_names);
            }
            None => {
                // This is the first file for this request.
                self.sub_request.attached_files = Some(file_name);
            }
        };

        self
    }

    /// Sets the JSON body for this sub-request.
    #[inline]
    pub fn json_object_body<J>(
        self,
        body: J,
    ) -> SingleRequestFormatter<'a, 'b, JsonObjectPayload<J>, Q> {
        // This creates a new formatter with an updated payload type.
        SingleRequestFormatter {
            serializer: self.serializer,
            sub_request: BatchSubRequest {
                body: FormEncoded(JsonObjectPayload(body)),
                // Copy over all other fields from the previous state.
                method: self.sub_request.method,
                relative_url: self.sub_request.relative_url,
                name: self.sub_request.name,
                attached_files: self.sub_request.attached_files,
            },
        }
    }

    /// Ensures this request has a name and returns it.
    ///
    /// A name is required if other requests in the batch need to depend on this one.
    /// The name is generated on-demand to ensure uniqueness.
    #[inline]
    pub fn get_name(&mut self) -> Cow<'static, str> {
        self.sub_request
            .name
            .get_or_insert_with(|| self.serializer.get_next_request_name())
            .clone()
    }

    /// Finalizes this sub-request and serializes it into the batch.
    #[inline]
    pub fn finish(self) -> Result<(), FormatError> {
        self.serializer.serialize_request(&self.sub_request)
    }
}

// SECTION: Data Structures for Serialization
// ============================================

/// Represents a single sub-request that can be serialized into the batch JSON array.
#[derive(Serialize, Debug)]
pub(crate) struct BatchSubRequest<'a, P, Q> {
    #[serde(serialize_with = "serialize_method_as_str")]
    method: Method,
    #[serde(rename = "relative_url")]
    relative_url: RelativeUrlSerializer<'a, Q>,
    #[serde(skip_serializing_if = "FormEncoded::is_zst")]
    body: FormEncoded<P>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<Cow<'static, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attached_files: Option<Cow<'static, str>>,
}

/// Custom serializer for `http::Method` to serialize it as an uppercase string (e.g., "POST").
#[inline]
fn serialize_method_as_str<S: Serializer>(method: &Method, ser: S) -> Result<S::Ok, S::Error> {
    ser.serialize_str(method.as_str())
}

/// A helper struct responsible for serializing the URL, query parameters, and
/// access token into the final `relative_url` string.
#[derive(Debug)]
pub(crate) struct RelativeUrlSerializer<'a, Q> {
    endpoint_url: String,
    query: Q,
    auth: Option<Cow<'a, Auth>>,
}

impl<'a, Q> Serialize for RelativeUrlSerializer<'a, Q>
where
    Q: Serialize,
{
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut final_url = Batch::make_url_relative(&self.endpoint_url).to_string();

        let mut has_query_params = false;

        // NOTE: See PendingRequest::send
        // We may call serde_urlencoded::to_string and check for emptiness, but most
        // requests have no query so we keep wasting calls.
        if std::mem::size_of::<Q>() != 0 {
            let query_string =
                serde_urlencoded::to_string(&self.query).map_err(serde::ser::Error::custom)?;
            // Append the serialized query string if it's not empty.
            if !query_string.is_empty() {
                final_url.push('?');
                final_url.push_str(&query_string);
                has_query_params = true;
            }
        }

        if let Some(auth) = &self.auth {
            use std::fmt::Write;
            // Append '&' if query params already exist, otherwise '?'
            final_url.push(if has_query_params { '&' } else { '?' });
            // We assume the token is already in a format suitable for a URL.
            write!(final_url, "access_token={auth}").map_err(|_| {
                serde::ser::Error::custom("An error occured while formatting the access_token")
            })?;
        }

        // Serialize the final, constructed string.
        serializer.serialize_str(&final_url)
    }
}

/// A wrapper to handle the specific "form-encoded" style for the request body.
///
/// # Panic
/// Uses lots of fmt::Display under the hood. Most impls (inluding serde (by calling to_string) and
/// serde_json) panic on error from Display::fmt(..) not directly from the underlying fmt::Write.
/// This means an error during form serialization is gonna panic. This shouldn't occur if
/// we keep accepting just payloads that claim to be JsonObjectPayload and ().
///
/// The cost of avoiding this is countless heap-allocations when we can simply solve it cooperatively.
#[derive(Debug)]
struct FormEncoded<P>(P);

impl<P> FormEncoded<P> {
    #[inline]
    const fn is_zst(&self) -> bool {
        // NOTE: See PendingRequest::send
        std::mem::size_of::<P>() == 0
    }
}

impl<P: Serialize> Serialize for FormEncoded<P> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Man can't be too sure of himself.
        struct DisplayCatchSerError<'a, V> {
            value: &'a V,
            err: std::cell::Cell<Option<serde_metaform::error::Error>>
        }

        impl<'a, V: Serialize> Display for DisplayCatchSerError<'a, V> {
            #[inline]
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                if let Err(err) = serde_metaform::to_writer(f, self.value) {
                    self.err.set(Some(err));       
                }
                Ok(())
            }
        }

        let display = DisplayCatchSerError {
            value: &self.0,
            err: None.into()
        };
        let ok = serializer.collect_str(&display)?;

        if let Some(err) = display.err.into_inner() {
            Err(<S::Error as serde::ser::Error>::custom(format!("Error while formatting form body: {err}")))
        } else {
            Ok(ok)
        }
    }
}

// SECTION: Conversion from PendingRequest to BatchSubRequest
// ==========================================================

impl<'a, P, Q> From<PendingRequest<'a, JsonObjectPayload<P>, Q>>
    for BatchSubRequest<'a, JsonObjectPayload<P>, Q>
{
    #[inline]
    fn from(req: PendingRequest<'a, JsonObjectPayload<P>, Q>) -> Self {
        Self {
            method: req.method,
            relative_url: RelativeUrlSerializer {
                endpoint_url: req.endpoint,
                query: req.query,
                auth: req.auth,
            },
            body: FormEncoded(req.payload),
            name: None,
            attached_files: None,
        }
    }
}

impl<'a, Q> From<PendingRequest<'a, (), Q>> for BatchSubRequest<'a, (), Q> {
    #[inline]
    fn from(req: PendingRequest<'a, (), Q>) -> Self {
        Self {
            method: req.method,
            relative_url: RelativeUrlSerializer {
                endpoint_url: req.endpoint,
                query: req.query,
                auth: req.auth,
            },
            body: FormEncoded(()),
            name: None,
            attached_files: None,
        }
    }
}

// SECTION: Error Handling & Utilities
// ===================================

/// An error returned when processing a batch response fails.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct BatchResponseError {
    inner: ResponseProcessingError,
}

impl BatchResponseError {
    /// Returns the API endpoint (if known) where this error occurred.
    pub fn endpoint(&self) -> Option<&str> {
        #[cfg(debug_assertions)]
        {
            Some(&self.inner.endpoint)
        }

        #[cfg(not(debug_assertions))]
        {
            None
        }
    }
}

/// An internal error type for failures during batch response processing.
/// This error indicates a problem with the structure or content of the response
/// from the API, rather than an API-level error (like a 404).
#[derive(Debug, thiserror::Error)]
#[cfg_attr(
    debug_assertions,
    error("Error processing batch response for endpoint '{endpoint}': {kind}")
)]
#[cfg_attr(
    not(debug_assertions),
    error("Error processing batch response: {kind}")
)]
pub(crate) struct ResponseProcessingError {
    /// The endpoint of the sub-request that this error corresponds to.
    #[cfg(debug_assertions)] // maybe not the best gate
    pub(crate) endpoint: Cow<'static, str>,
    /// The specific kind of error that occurred.
    #[source]
    pub(crate) kind: ResponseProcessingErrorKind,
}

impl ResponseProcessingError {
    pub(crate) fn new(
        #[cfg(debug_assertions)] endpoint: Cow<'static, str>,
        kind: ResponseProcessingErrorKind,
    ) -> Self {
        Self { endpoint, kind }
    }
}

/// The specific categories of response processing errors.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ResponseProcessingErrorKind {
    #[error("Expected {want} response(s), but only got {got}")]
    InsufficientResponses { want: usize, got: usize },
    #[error(
        "The API returned a null response where a valid one was required. \
         If this request can be null, consider using `nullable()` or `then_nullable()`."
    )]
    NullNotNullable,
}

/// A custom error type for failures during batch request formatting/serialization.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FormatError {
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    // #[error("Reqwest error: {0}")]
    // Reqwest(#[from] reqwest::Error),
    // #[error("The request body is not a valid JSON object")]
    // InvalidBody,
}

// Conversions to the main library error type
impl From<ResponseProcessingError> for crate::Error {
    fn from(value: ResponseProcessingError) -> Self {
        crate::Error::internal(value.into())
    }
}

// Conversions to the main library error type
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
                let impl_flatten_base! {
                    @stack
                    |Rest|: $($R)*,
                    |Stack|: $T
                } = from;
                ($T, $($R),*)
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        app::SubscriptionField,
        client::{ChainQuery, FieldsQuery, TupleArrayShorthand},
        waba::PhoneNumberMetadataField,
    };

    use super::*;

    /// Tests the robust URL path and query extraction.
    #[test]
    fn test_extract_path_and_query() {
        assert_eq!(
            Batch::make_url_relative("https://graph.facebook.com/v18.0/me/messages"),
            "/v18.0/me/messages"
        );
        assert_eq!(
            Batch::make_url_relative("https://example.com/some/path?query=1&another=2"),
            "/some/path?query=1&another=2"
        );
        assert_eq!(Batch::make_url_relative("https://example.com:8080/"), "/");
        assert_eq!(Batch::make_url_relative("https://example.com"), "/");
        assert_eq!(Batch::make_url_relative("http://localhost/api"), "/api");
    }

    /// Tests the serialization of `RelativeUrlSerializer`.
    #[test]
    fn test_relative_url_serializer() {
        // Case 1: With query parameters only
        let url_parts_query = RelativeUrlSerializer {
            endpoint_url: "https://graph.facebook.com/v18.0/12344939919/subscription".to_string(),
            query: ChainQuery {
                a: FieldsQuery::from([SubscriptionField::Messages, SubscriptionField::Security]),
                b: TupleArrayShorthand([("limit", 10)]),
            },
            auth: None,
        };
        let serialized_query = serde_json::to_string(&url_parts_query).unwrap();
        assert_eq!(
            serialized_query,
            "\"/v18.0/12344939919/subscription?fields=messages%2Csecurity&limit=10\""
        );

        // Case 2: With auth token only
        let url_parts_auth = RelativeUrlSerializer {
            endpoint_url: "https://graph.facebook.com/v18.0/me".to_string(),
            query: (), // Empty query
            auth: Some(Cow::Owned(Auth::token("TEST_TOKEN"))),
        };
        let serialized_auth = serde_json::to_string(&url_parts_auth).unwrap();
        assert_eq!(serialized_auth, "\"/v18.0/me?access_token=TEST_TOKEN\"");

        // Case 3: With both query and auth token
        let url_parts_both = RelativeUrlSerializer {
            endpoint_url: "https://graph.facebook.com/v18.0/21231993936/phone_numbers".to_string(),
            query: ChainQuery {
                a: FieldsQuery::from([PhoneNumberMetadataField::VerifiedName]),
                b: TupleArrayShorthand([("limit", 5)]),
            },
            auth: Some(Cow::Owned(Auth::token("ANOTHER_TOKEN"))),
        };
        let serialized_both = serde_json::to_string(&url_parts_both).unwrap();
        assert_eq!(
            serialized_both,
            "\"/v18.0/21231993936/phone_numbers?fields=verified_name&limit=5&access_token=ANOTHER_TOKEN\""
        );

        // Case 4: With no query and no auth
        let url_parts_none = RelativeUrlSerializer {
            endpoint_url: "https://graph.facebook.com/v18.0/me/feed".to_string(),
            query: (),
            auth: None,
        };
        let serialized_none = serde_json::to_string(&url_parts_none).unwrap();
        assert_eq!(serialized_none, "\"/v18.0/me/feed\"");
    }

    /// Tests the `FormEncoded` body serialization.
    #[test]
    fn test_form_encoded_body_serialization() {
        #[derive(Serialize)]
        struct Body {
            message: &'static str,
        }

        // Test with a serializable struct
        let form_encoded_body = FormEncoded(Body { message: "hello" });
        let serialized = serde_json::to_string(&form_encoded_body).unwrap();
        assert_eq!(serialized, "\"message=hello\"");

        // Test with deep json

        #[derive(Serialize)]
        struct BodyJson {
            inner: Body,
        }

        let form_encoded_empty = FormEncoded(BodyJson {
            inner: Body { message: "hello" },
        });

        let serialized_empty = serde_json::to_string(&form_encoded_empty).unwrap();
        assert_eq!(
            serialized_empty,
            r#""inner=%7B%22message%22%3A%22hello%22%7D""#
        );
    }

    /// Tests the full serialization of a `BatchSubRequest`.
    #[test]
    fn test_full_sub_request_serialization() {
        let sub_request = BatchSubRequest {
            method: Method::POST,
            relative_url: RelativeUrlSerializer {
                endpoint_url: "https://graph.facebook.com/v18.0/me/messages".to_string(),
                query: (),
                auth: None,
            },
            body: FormEncoded(()),
            name: Some(Cow::Borrowed("create-message")),
            attached_files: Some(Cow::Borrowed("file0,file1")),
        };

        let json_string = serde_json::to_string(&sub_request).unwrap();
        let expected = r#"{"method":"POST","relative_url":"/v18.0/me/messages","name":"create-message","attached_files":"file0,file1"}"#;
        assert_eq!(json_string, expected);
    }

    /// Tests the file attachment logic and naming conventions.
    #[test]
    fn test_file_attachment_and_naming() {
        let buffer = Vec::new();
        let mut serializer = serde_json::Serializer::new(buffer);
        let seq_serializer = serializer.serialize_seq(None).unwrap();
        let mut batch_serializer = BatchSerializer::new(seq_serializer);

        // Mock a pending request
        let pending_req = BatchSubRequest {
            method: Method::POST,
            relative_url: RelativeUrlSerializer {
                endpoint_url: "/me/photos".to_string(),
                query: (),
                auth: None,
            },
            body: FormEncoded(()),
            name: None,
            attached_files: None,
        };

        // Create a request formatter and attach files
        batch_serializer
            .format_request(pending_req)
            .attach_file(None, Part::bytes(b"...")) // Should be named "file0"
            .attach_file(Some(Cow::Borrowed("source")), Part::bytes(b"...")) // Specifically named "source"
            .attach_file(None, Part::bytes(b"...")) // Should be named "file1"
            .finish()
            .unwrap();

        let files = batch_serializer.finish().unwrap();

        // Check that the attached files have the correct names in the multipart collection.
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].0, "file");
        assert_eq!(files[1].0, "source");
        assert_eq!(files[2].0, "file0");

        // Now check the serialized JSON to ensure `attached_files` field is correct.
        // The buffer contains the full JSON array: `[{...}]`
        let buffer = serializer.into_inner();
        let json_output = String::from_utf8(buffer).unwrap();
        let parsed_json: serde_json::Value = serde_json::from_str(&json_output).unwrap();

        let attached_files_field = parsed_json[0]
            .get("attached_files")
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(attached_files_field, "file,source,file0");
    }

    // Helper to create a mock BatchSubResponse
    fn create_sub_response(status_code: u16, body: &str) -> Option<BatchSubResponse> {
        Some(BatchSubResponse {
            status_code,
            body: body.to_string().into_boxed_str(),
        })
    }

    #[test]
    fn batch_response_iterator_works_correctly() {
        let responses_vec = vec![
            create_sub_response(200, "{\"id\": 1}"),
            None, // A null response
            create_sub_response(404, "Not Found"),
        ];
        let mut batch_response = BatchResponse::new(responses_vec);

        // First item should be Ok(Some(...))
        let first = batch_response.next().unwrap();
        assert!(first.is_some());
        assert_eq!(first.unwrap().status(), StatusCode::OK);

        // Second item should be Ok(None)
        let second = batch_response.next().unwrap();
        assert!(second.is_none());

        // Third item should be Ok(Some(...)) with a 404
        let third = batch_response.next().unwrap();
        assert!(third.is_some());
        let third_unwrapped = third.unwrap();
        assert_eq!(third_unwrapped.status(), StatusCode::NOT_FOUND);
        assert_eq!(third_unwrapped.body(), "Not Found".into());

        // Iterator should be exhausted
        assert!(batch_response.next().is_none());
    }

    #[test]
    fn batch_sub_response_methods() {
        let sub_response = create_sub_response(201, "Created").unwrap();
        assert_eq!(sub_response.status(), StatusCode::CREATED);

        // Test invalid status code fallback
        let invalid_code_response = BatchSubResponse {
            status_code: 9999, // Invalid code
            body: "test".to_string().into_boxed_str(),
        };
        assert_eq!(invalid_code_response.status(), StatusCode::OK); // Falls back to default

        // Test body consumption
        let body = sub_response.body();
        assert_eq!(body, "Created".into());
    }

    #[test]
    fn try_next_handles_null_response() {
        let responses_vec = vec![None];
        let mut batch_response = BatchResponse::new(responses_vec);
        let endpoint = Cow::from("/users/query");

        // () is used for success so it expects something
        let result = batch_response.try_next::<()>(endpoint);

        match result {
            Err(e) => {
                assert_eq!(e.endpoint, "/users/query");
                assert!(matches!(
                    e.kind,
                    ResponseProcessingErrorKind::NullNotNullable
                ));
            }
            _ => panic!("Expected a NullNotNullable error"),
        }
    }

    #[test]
    fn try_next_handles_insufficient_responses() {
        let responses_vec = vec![]; // No responses
        let mut batch_response = BatchResponse::new(responses_vec);
        let endpoint = Cow::from("/users/1");

        // () is used for success so it expects something
        let result = batch_response.try_next::<()>(endpoint);

        match result {
            Err(e) => {
                assert_eq!(e.endpoint, "/users/1");
                assert!(matches!(
                    e.kind,
                    ResponseProcessingErrorKind::InsufficientResponses { .. }
                ));
            }
            _ => panic!("Expected an InsufficientResponses error"),
        }
    }

    #[derive(Clone)]
    struct SomeRequest;

    impl Requests for SomeRequest {
        type BatchHandler = (); // should be !

        type ResponseReference = ();

        fn into_batch_ref(
            self,
            _: &mut BatchSerializer,
        ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
            panic!()
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

        // TODO: Waba
        // assert_is_requests::<crate::waba::ListCatalog>();
        // assert_is_requests::<crate::waba::ListNumber>();
        // assert_is_requests::<crate::waba::ListApp>();

        assert_is_requests::<(SomeRequest, SomeRequest)>();
        assert_is_requests::<super::Batch<SomeRequest>>();
        assert_is_requests::<super::Many<<Vec<SomeRequest> as IntoIterator>::IntoIter>>();
        assert_is_requests::<super::Then<SomeRequest, SomeRequest, fn(()) -> SomeRequest>>();
        assert_is_requests::<super::Map<SomeRequest, fn(()) -> i32>>();
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

    pub struct RawRequest(PendingRequest<'static, serde_json::Value>);

    impl RawRequest {
        fn request(self) -> PendingRequest<'static, JsonObjectPayload<serde_json::Value>> {
            PendingRequest {
                method: self.0.method,
                endpoint: self.0.endpoint,
                auth: self.0.auth,
                payload: JsonObjectPayload(self.0.payload),
                query: (),
                client: self.0.client,
            }
        }
    }

    SimpleOutputBatch! {
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
        let get_req = PendingRequest {
            method: Method::GET,
            endpoint: "http://example.com/api/v1/users/1".into(),
            auth: None,
            payload: serde_json::to_value(()).unwrap(),
            query: (),
            client: http_client.clone(),
        };

        let post_req = PendingRequest {
            method: Method::POST,
            endpoint: "http://example.com/api/v1/posts".into(),
            auth: None,
            payload: json!({ "title": "Test Post&=" }),
            query: (),
            client: http_client.clone(),
        };

        // Create a batch and join the requests.
        let batch = client
            .batch()
            .include(RawRequest(get_req))
            .include(RawRequest(post_req));

        // Execute the batch to get the serialized bytes.
        let request_form = batch.execute().state.unwrap().form;

        let expected_form = batch_part!(
            r#"[{"method":"GET","relative_url":"/api/v1/users/1","body":""},{"method":"POST","relative_url":"/api/v1/posts","body":"title=Test%20Post%26%3D"}]"#
        ).text("include_headers", "false");

        assert_multipart_eq!(request_form, expected_form)
    }

    /// Verifies that authentication is correctly added per requests
    #[tokio::test]
    async fn test_individual_auth() {
        let client = mock_client().await;
        let http_client = mock_http_client();

        // Define two different requests.
        let get_req = PendingRequest {
            method: Method::GET,
            endpoint: "http://example.com/api/v1/users/1".into(),
            auth: Some(Cow::Owned(Auth::token("TOKEN"))),
            payload: serde_json::to_value(()).unwrap(),
            query: (),
            client: http_client.clone(),
        };

        let post_req = PendingRequest {
            method: Method::POST,
            endpoint: "http://example.com/api/v1/posts".into(),
            auth: Some(Cow::Owned(Auth::token("ParsedTOKEN").parsed().unwrap())),
            payload: json!({ "title": "Test Post" }),
            query: (),
            client: http_client.clone(),
        };

        // Create a batch and join the requests.
        let batch = client
            .batch()
            .include(RawRequest(get_req))
            .include(RawRequest(post_req));

        // Execute the batch to get the serialized bytes.
        let request_form = batch.execute().state.unwrap().form;
        let expected_form = batch_part!(
            r#"[{"method":"GET","relative_url":"/api/v1/users/1?access_token=TOKEN","body":""},{"method":"POST","relative_url":"/api/v1/posts?access_token=ParsedTOKEN","body":"title=Test%20Post"}]"#
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
            RawRequest(PendingRequest {
                method: Method::GET,
                endpoint: format!("http://example.com/api/v1/users/{}", id),
                auth: None,
                payload: serde_json::to_value(()).unwrap(),
                query: (),
                client: http_client.clone(),
            })
        });

        let batch = client.batch().include_iter(requests);

        let request_form = batch.execute().state.unwrap().form;
        let expected_form = batch_part!(
            r#"[{"method":"GET","relative_url":"/api/v1/users/1","body":""},{"method":"GET","relative_url":"/api/v1/users/2","body":""},{"method":"GET","relative_url":"/api/v1/users/3","body":""}]"#
        ).text("include_headers", "false");

        assert_multipart_eq!(request_form, expected_form)
    }

    /// Verifies that binary files are correctly attached and referenced.
    #[tokio::test]
    async fn test_binary_file_attachment() {
        let client = mock_client().await;
        let http_client = mock_http_client();

        struct UploadMedia {
            request: PendingRequest<'static, (), ()>,
            media: Part,
        }

        impl Requests for UploadMedia {
            type BatchHandler = ();

            type ResponseReference = ();

            fn into_batch_ref(
                self,
                batch_serializer: &mut BatchSerializer,
            ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
                batch_serializer
                    .format_request(self.request)
                    .attach_file(None, self.media)
                    .finish()?;
                Ok(((), ()))
            }

            requests_batch_include! {}
        }

        let request = PendingRequest {
            method: Method::POST,
            endpoint: "http://example.com/upload".into(),
            auth: None,
            payload: (),
            query: (),
            client: http_client.clone(),
        };
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

        let get_req = PendingRequest {
            method: Method::GET,
            endpoint: "http://example.com/ok".into(),
            auth: None,
            payload: serde_json::to_value(()).unwrap(),
            query: (),
            client: http_client.clone(),
        };

        // We need to simulate a serialization failure, which is hard with valid data.
        // Let's pass a non-object json
        let post_req_with_bad_body = PendingRequest {
            method: Method::GET,
            endpoint: "http://example.com/bad".into(),
            auth: None,
            payload: serde_json::to_value(1).unwrap(),
            query: (),
            client: http_client.clone(),
        };

        // Join one good request and one bad one.
        let batch = client
            .batch()
            .include(RawRequest(get_req))
            .include(RawRequest(post_req_with_bad_body));

        // The execute call should fail because of the invalid request.
        let result = batch.execute().state;

        // The result should be an error.
        match result {
            Err(FormatError::Serialization(_)) => {} // We expect a serialization error.
            _ => panic!("Expected a Serialization error, but got a different one."),
        }
    }
}
