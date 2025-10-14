/// A macro to generate `serde` implementations for `Section<T>`.
///
/// The API expects a `Section` to be serialized with a specific field name
/// for its items, which depends on the item type `T`. For example, a `Section<ProductRef>`
/// needs a `product_items` field, while a `Section<OptionButton>` needs a `rows` field.
///
/// This macro generates a helper struct and `Serialize`/`Deserialize` impls
/// to handle this mapping.
macro_rules! serde_section {
    (
        // Conjoin
        $(#[$meta:meta])*
        pub struct $name:ident<$Item:ident> {
            $(#[$f:meta])*
            pub $title:ident: $tty:ty,
            $(#[$g:meta])*
            pub $items:ident: $ity:ty,
        }
    ) => {
        $(#[$meta])*
        pub struct $name<$Item> {
            $(#[$f])*
            pub $title: $tty,
            $(#[$g])*
            pub $items: $ity,
        }

        serde_section! {
            product_items => ProductRef,
            rows => OptionButton
        }
    };
    ($($item_name:ident => $ty:path),*) => {
    paste::paste!{
        $(
            /// A helper struct for serializing/deserializing `Section<` a_ty `>`.
            #[derive(Serialize, Deserialize)]
            struct [<$ty Section>] {
                title: String,
                $item_name: Vec<$ty>
            }

            impl serde::Serialize for $crate::message::Section<$ty> {
                #[inline]
                fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                    where
                        S: serde::Serializer {
                    // SAFETY: These are literally thesame structs under different names so they'll always
                    // have thesame layout.
                    //
                    // Trying to avoid this will likely involve heavy unnecessary cloning since we're behind
                    // a reference.
                    let helper: &[<$ty Section>] = unsafe { std::mem::transmute(self) };
                    helper.serialize(serializer)
                }
            }

            impl<'de> serde::Deserialize<'de> for $crate::message::Section<$ty> {
                #[inline]
                fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                    where
                        D: serde::Deserializer<'de> {
                    let helper = [<$ty Section>]::deserialize(deserializer)?;
                    // SAFETY: These are literally thesame structs under different names so they'll always
                    // have thesame layout.
                    Ok(unsafe { std::mem::transmute::<[<$ty Section>], $crate::message::Section<$ty>>(helper) })
                }
            }
        )*
    }
    }
}

/// A macro to implement `Nullable` for a type that is already nullable,
/// making the `.nullable()` call idempotent.
#[cfg(feature = "batch")]
macro_rules! impl_idempotent_nullable {
    ($already_nullable:ident <$($life:lifetime,)? $($T:ident),*>) => {
        impl<$($life,)? $($T),*> $crate::batch::Nullable for $already_nullable<$($life,)? $($T),*> {
            type Nullable = $already_nullable<$($life,)? $($T),*>;

            #[inline]
            fn nullable(self) -> Self::Nullable {
                self
            }
        }
    };
}

/// Helper macro to implement the `Requests::include` method for tuples.
/// This is used internally to recursively build up the batch request types.
/// While default in assoc is unstable
#[cfg(feature = "batch")]
macro_rules! requests_batch_include {
    () => {
        // We need a name like this to avoid collision
        type Include<SomeR> = (Self, SomeR);

        fn include<SomeR>(self, r: SomeR) -> Self::Include<SomeR> {
            (self, r)
        }
    };
}

// This is what's become of the former Nullable struct
// Building block for Nullable
//
// Using something like NullableUnit<T> would prevent
// an operation from going multistep in the future without
// breaking changes (somehow the handler is public).. because
// that NullableUnit<T>... so we use macro instead
#[cfg(feature = "batch")]
macro_rules! NullableUnit {
        ($T:ident <$($life:lifetime,)? $([$g:ty: $($b:tt)*]),*>) => {
            paste::paste!{
                impl<$($life,)? $($g: $($b)*),*> $crate::batch::Nullable for $T<$($life,)? $($g),*> {
                    type Nullable = [<Nullable $T>] <$($life,)? $($g),*>;

                    #[inline]
                    fn nullable(self) -> Self::Nullable {
                        Self::Nullable {
                            inner: self
                        }
                    }
                }

                impl_idempotent_nullable! {[<Nullable $T>] <$($life,)? $($g),*>}

                pub struct [<Nullable $T>] <$($life,)? $($g),*> {
                    inner: $T <$($life,)? $($g),*>
                }

                impl<$($life,)? $($g: $($b)*),*> $crate::batch::Requests for [<Nullable $T>] <$($life,)? $($g),*> {
                    type BatchHandler = [<Nullable $T Handler>] <$($life,)? $($g),*>;

                    type ResponseReference = <$T <$($life,)? $($g),*> as $crate::batch::Requests>::ResponseReference;

                    #[inline]
                    fn into_batch_ref(
                        self,
                        batch_serializer: &mut $crate::batch::BatchSerializer,
                    ) -> Result<(Self::BatchHandler, Self::ResponseReference), $crate::batch::FormatError> {
                        let (h, r) = self.inner.into_batch_ref(batch_serializer)?;
                        Ok(([<Nullable $T Handler>]{inner: h}, r))
                    }

                    #[inline]
                    fn into_batch(self, batch_serializer: &mut $crate::batch::BatchSerializer) -> Result<Self::BatchHandler, $crate::batch::FormatError>
                    where
                        Self: Sized,
                    {
                        let h = self.inner.into_batch(batch_serializer)?;
                        Ok([<Nullable $T Handler>]{inner: h})
                    }

                    #[inline]
                    fn size_hint(&self) -> (usize, Option<usize>) {
                        // (1, Some(1)) SendMessage breaks this assumption
                        self.inner.size_hint()
                    }

                    requests_batch_include! {}
                }

                pub struct [<Nullable $T Handler>] <$($life,)? $($g: $($b)*),*> {
                    inner: <$T <$($life,)? $($g),*> as $crate::batch::Requests>::BatchHandler
                }

                impl<$($life,)? $($g: $($b)*),*> $crate::batch::Handler for [<Nullable $T Handler>] <$($life,)? $($g),*> {
                    type Responses =
                        Option<<<$T <$($life,)? $($g),*> as $crate::batch::Requests>::BatchHandler as $crate::batch::Handler>::Responses>;

                    #[inline]
                    fn from_batch(
                        self,
                        response: &mut $crate::batch::BatchResponse,
                    ) -> Result<Self::Responses, $crate::batch::ResponseProcessingError> {
                        match self.inner.from_batch(response) {
                            Ok(val) => Ok(Some(val)),
                            Err($crate::batch::ResponseProcessingError {
                                kind: $crate::batch::ResponseProcessingErrorKind::NullNotNullable,
                                ..
                            }) => Ok(None),
                            Err(err) => Err(err), // real error, propagate
                        }
                    }
                }
            }
        }
    }

/// WATCH OUT FOR RENAMED FIELDS
#[cfg(feature = "batch")]
macro_rules! reference {
    ($req_name:tt => $target_struct:ty => $([$($field_chain:tt)*])*) => {{
        #[allow(dead_code)]
        fn check_field(v: $target_struct) {
            let _ = v.$($($field_chain)*).*;
        }
        // TODO: We might be able to make this static if we work more on request names
        format!("{{result={}:$.{}}}", $req_name, stringify!($($($field_chain)*).*))
    }}
}

/// A higher-order macro that applies a given macro to a list of common string types.
///
/// This is a utility to reduce boilerplate when a macro needs to be implemented
/// for `&str`, `String`, `Cow<'_, str>`, etc.
///
/// # Usage
///
/// ```rust,ignore
/// macro_rules! my_macro {
///     ($ty:ty) => { /* implementation for stringy type */ };
/// }
///
/// // Expands to my_macro!{&'_ str}, my_macro!{String}, etc.
/// impl_common_strings!(my_macro);
/// ```
macro_rules! impl_common_strings {
    // Entry point: Takes a macro identifier and forwards it to the implementation arm.
    ($mac:ident) => {
        impl_common_strings! {
            @impl $mac [&'_life str, &'_life mut str, &'_life String, String, Box<str>, Cow<'_life, str>]
        }
    };
    // Implementation arm: Iterates through the list of types and applies the macro to each.
    {@impl $mac:ident [$($ty:ty),*]} => {
        $(
            $mac!{$ty}
        )*
    }
}

/// Implements a helper method for constructing an API endpoint URL.
///
/// This macro generates a private `endpoint` function that takes a final path segment
/// and joins it with a base path stored in the struct (e.g., `self.phone_number_id`).
///
/// # Usage
///
/// ```rust,ignore
/// struct SomeApiNode {
///     client: Client,
///     phone_number_id: String,
/// }
///
/// impl SomeApiNode {
///     // This will generate a method like:
///     // fn endpoint<'a>(&'a self, path: &'a str) -> Endpoint<'a, 2> {
///     //     self.client.endpoint().join(&self.phone_number_id).join(path)
///     // }
///     Endpoint!(phone_number_id);
/// }
/// ```
macro_rules! Endpoint {
    ($($node:tt)*) => {
        /// Constructs a complete API endpoint from the node's ID and a final path segment.
        #[inline(always)]
        pub(crate) fn endpoint<'a>(&'a self, path: &'a str) -> $crate::client::Endpoint<'a, 2>
        {
            self.client.endpoint().join(&self.$($node)*).join(path)
        }
    };
}

/// Implements `From<T>` and `PartialEq<T>` for enum variants that wrap type `T`.
///
/// This macro reduces the boilerplate needed to ergonomically create an enum from its
/// inner types and to compare an enum instance with a potential inner value.
///
/// # Usage
///
/// ```rust,ignore
/// enum MessageContent {
///     Text(TextContent),
///     Image(ImageContent),
/// }
///
/// // Generates `From<TextContent> for MessageContent`, `PartialEq<TextContent> for MessageContent`, etc.
/// enum_traits!(|MessageContent| TextContent => Text, ImageContent => Image);
/// ```
// TODO: Make derive
macro_rules! enum_traits {
    (|$target:ident| $($content:ident => $variant:ident),*) => {
        $(
          // Implement `From` to allow easy conversion from the inner type to the enum wrapper.
            impl From<$content> for $target {
                #[inline]
                fn from(value: $content) -> Self {
                    Self::$variant(value)
                }
            }

            // Implement `PartialEq` to allow comparing the enum with a potential inner value.
            impl PartialEq<$content> for $target {
                #[inline]
                fn eq(&self, other: &$content) -> bool {
                    // Check if `self` is the correct variant and if its inner value equals `other`.
                    if let Self::$variant(inner) = self {
                        inner.eq(other)
                    } else {
                        false
                    }
                }
            }
        )*
    }
}

/// # ⚠️ Safety Warning ⚠️
///
/// **Do not use this function unless you are absolutely certain about memory layouts.**
///
/// This function performs an unsafe pointer cast to view(slice) a reference of type `&M`
/// as a reference of type `&R`. This is a highly dangerous operation that is equivalent to
/// `std::mem::transmute`, but for references.
///
/// ## Safety Preconditions
///
/// The caller **must guarantee** that:
/// 1.  `M` and `R` have the **exact same memory layout**. This is typically only true if
///     both are `#[repr(C)]` structs with compatible field layouts. The layout of default
///     `#[repr(Rust)]` types is not guaranteed.
/// 2.  The alignment of `R` is less than or equal to the alignment of `M`.
///
/// Failure to uphold these invariants will result in **undefined behavior**, which can
/// lead to memory corruption, segmentation faults, or other critical program failures.
#[inline]
#[allow(clippy::needless_lifetimes)]
pub(crate) unsafe fn view_ref<'r, M, R>(m: &'r M) -> &'r R {
    // The safety burden is entirely on the caller of this function.
    // This performs a raw pointer cast and dereferences the result.
    unsafe { &*(m as *const M as *const R) }
}

/// Implements `std::future::IntoFuture` for a type.
///
/// This macro takes a struct's `execute`-like method (which returns a Future)
/// and uses it to implement the `IntoFuture` trait. This allows instances of the
/// struct to be `.await`ed directly.
///
/// It handles both `nightly` and `stable` Rust compilers:
/// - On **nightly**, it uses `impl Future` for a zero-cost abstraction.
/// - On **stable**, it uses `Pin<Box<dyn Future>>` to type-erase the future, which
///   incurs a small heap allocation.
macro_rules! IntoFuture {
    (
        // The `impl` block for the type.
        impl $(<$($lt:lifetime $(,)?)? $($gen:ident),*>)? $name:ident $(<$($lt2:lifetime $(,)?)? $($gen2:ident),*>)?
        $([where $($wheres:tt)*])?
        {
            // The `execute` function to be wrapped.
            $(#[$meta:meta])*
            pub fn $func:ident ( $($args:tt)* ) -> impl Future<Output = $ret:ty> + $fut_life:lifetime $body:block
        }
    ) => {
        // First, emit the original function implementation as-is.
        impl $(<$($lt,)? $($gen),*>)? $name $(<$($lt2,)? $($gen2),*>)?
        $(where $($wheres)*)?
        {
            $(#[$meta])*
            pub fn $func($($args)*) -> impl ::std::future::Future<Output = $ret> + $fut_life $body
        }

        // Implementation for nightly Rust, which allows `impl Trait` in type aliases.
        #[cfg(nightly_rust)]
        impl $(<$($lt,)? $($gen),*>)? ::std::future::IntoFuture for $name $(<$($lt2,)? $($gen2),*>)?
        $(where $($wheres)*)?
        {
            type Output = $ret;
            type IntoFuture = impl ::std::future::Future<Output = Self::Output> + $fut_life;

            fn into_future(self) -> Self::IntoFuture {
                self.$func()
            }
        }

        // Implementation for stable Rust, which requires boxing the future.
        #[cfg(not(nightly_rust))]
        impl $(<$($lt,)? $($gen),*>)? ::std::future::IntoFuture for $name $(<$($lt2,)? $($gen2),*>)?
        $(where $($wheres)*)?
        {
            type Output = $ret;
            type IntoFuture = ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Self::Output> + Send + $fut_life>>;

            fn into_future(self) -> Self::IntoFuture {
                Box::pin(self.$func())
            }
        }
    };
}

/// Implements the `Requests` trait for a request builder to support batching.
///
/// This macro generates the necessary boilerplate to allow a "simple output"
/// request builder (one that sends a JSON body or no body) to be included in a
/// batch request.
///
/// It creates a `[Name]Handler` struct responsible for parsing the response
/// for this specific request out of a larger batch response.
macro_rules! SimpleOutputBatch {
    ($name:ident <$($life:lifetime,)? $([$g:ty: $($b:tt)*]),*> => $out:ty) => {
        paste::paste!{
            impl<$($life,)? $($g: $($b)*),*> $crate::batch::Requests for $name <$($life,)? $($g),*> {
                // The handler type that will process the response from the batch.
                type BatchHandler = [<$name Handler>] <$($g),*>;
                // The type that represents the placeholder in the batch response JSON.
                type ResponseReference =
                    <Self as $crate::batch::IntoResponseReference>::ResponseReference;

                #[inline]
                fn into_batch_ref(
                    self,
                    batch_serializer: &mut $crate::batch::BatchSerializer,
                ) -> Result<(Self::BatchHandler, Self::ResponseReference), $crate::batch::FormatError> {
                    // Extract the underlying request object.
                    let request = self.request();

                    #[cfg(debug_assertions)]
                    let endpoint = request.endpoint.clone();

                    // Use the batch serializer to format the request and get its unique name.
                    let mut formatted_req = batch_serializer.format_request(request);
                    let name = formatted_req.get_name();
                    let response_reference =
                        <Self as $crate::batch::IntoResponseReference>::into_response_reference(name);

                    // Finalize the request serialization.
                    formatted_req.finish()?;

                    let handler = [<$name Handler>] {
                        #[cfg(debug_assertions)]
                        endpoint: endpoint.into(),
                        // Add PhantomData markers for any generic parameters.
                        $(
                            [<_marker _ $g:lower>]: std::marker::PhantomData
                        ),*
                    };

                    Ok((handler, response_reference))
                }

                #[inline]
                fn into_batch(self, batch_serializer: &mut $crate::batch::BatchSerializer) -> Result<Self::BatchHandler, $crate::batch::FormatError> {
                    // Extract the underlying request object.
                    let request = self.request();

                    #[cfg(debug_assertions)]
                    let endpoint = request.endpoint.clone();

                    // Use the batch serializer to format the request.
                    batch_serializer.format_request(request).finish()?;

                    Ok([<$name Handler>] {
                        #[cfg(debug_assertions)]
                        endpoint: endpoint.into(),
                        // Add PhantomData markers for any generic parameters.
                        $(
                            [<_marker _ $g:lower>]: std::marker::PhantomData
                        ),*
                    })
                }

                requests_batch_include! {}
            }

            // Make nullable
            NullableUnit! {$name <$($life,)? $([$g: $($b)*]),*>}

            // The handler struct for this request type within a batch.
            #[non_exhaustive] // Some have no fields in release making them constructable from user code
            pub struct [<$name Handler>] <$($g),*> {
                #[cfg(debug_assertions)]
                endpoint: std::borrow::Cow<'static, str>,
                $(
                    [<_marker _ $g:lower>]: std::marker::PhantomData<$g>
                ),*
            }

            impl<$($g: $($b)*),*> $crate::batch::Handler for [<$name Handler>] <$($g),*> {
                type Responses = Result<$out, $crate::Error>;

                #[inline]
                fn from_batch(self, response: &mut $crate::batch::BatchResponse) ->
                    Result<Self::Responses, $crate::batch::ResponseProcessingError> {
                    // Delegate response parsing to the batch response handler.
                    response.try_next(#[cfg(debug_assertions)] self.endpoint)
                }
            }
        }
    };
}

/// Generates a request builder that produces an asynchronous stream of results.
///
/// This is used for API endpoints that return paginated lists of data. The macro
/// creates a builder struct with an `into_stream` method, which abstracts away
/// the complexity of fetching subsequent pages.
macro_rules! SimpleStreamOutput {
    (
        $({Query: $query:ty})?
        $(#[$cont_attr:meta])*
        $name:ident => $stream:ty
    ) => {
        /// This struct represents a pending API request.
        ///
        /// It provides methods to configure the request and then convert it into
        /// an asynchronous stream of results.
        ///
        /// The network request is performed when the generated stream is iterated over.
        $(#[$cont_attr])*
        pub struct $name {
            pub(crate) request: $crate::client::PendingRequest<'static, () $(,$query)?>,
        }

        impl $name {
            /// Specifies the authentication token to use for this API request.
            ///
            /// This is particularly useful for applications managing multiple WhatsApp Business Accounts (WBAs)
            /// where different API tokens might be required for various listing operations.
            /// It allows you to reuse a manager instance and apply the appropriate token for each specific
            /// request without re-initializing the `Client`.
            ///
            /// If not called, the request will use the authentication configured with the `Client`
            /// that initiated this operation.
            ///
            /// # Parameters
            /// - `auth`: [`Auth`] to use for this specific request.
            ///
            /// # Returns
            /// The updated builder instance.
            ///
            /// [`Auth`]: crate::client::Auth
            #[inline]
            pub fn with_auth<'a>(mut self, auth:  impl $crate::ToValue<'a, $crate::client::Auth>) -> Self {
                self.request = self.request.auth(auth.to_value().into_owned());
                self
            }

            paste::paste! {
                /// Converts the builder into an asynchronous stream of items.
                ///
                /// This method initiates the network request and yields each item as it is fetched.
                /// The stream abstracts away pagination and automatically fetches subsequent pages.
                ///
                /// # Item
                ///
                #[doc = "The stream yields `Result<T, Error>` where `T` is `"[<$stream>]"`."]
                ///
                /// # Returns
                ///
                /// A `futures::TryStream` that yields individual items from the API.
                #[inline]
                pub fn into_stream(self) -> impl futures::TryStream<Ok = $stream, Error = $crate::error::Error> {
                    $crate::rest::execute_paginated_request(self.request)
                }
            }

            // TODO: Implement `next_page` and batch support for streams.
        }
    }
}

/// Generates a "pending request" struct that acts as a builder.
///
/// This macro creates a public struct that wraps a request object. It's designed
/// to be configured with methods (like `.with_auth()`) and then executed by
/// being `.await`ed.
///
/// # Macro Parameters
/// - `Payload`: The optional type of the request body.
/// - `Query`: The optional type of the URL query parameters.
/// - `cont_attr`: Attributes to apply to the generated struct.
/// - `name`: The name of the builder struct to generate.
/// - `life`: An optional lifetime parameter for the struct.
/// - `execute`: The type that the request's future will resolve to on success.
macro_rules! SimpleOutput {
    (
        $({Payload: $payload:ty})?
        $({Query: $query:ty})?
        $(#[$cont_attr:meta])*
        $name:ident $(<$life:lifetime>)? => $execute:ty
    ) => {
        $(#[$cont_attr])*
        /// This struct represents a pending API request.
        ///
        /// It provides methods to configure the request before executing it. It does not perform any
        /// network operations until it is `.await`ed (due to its `IntoFuture` implementation) or
        /// its `execute().await` method is called.
        ///
        /// ## Lifetimes and Spawning
        ///
        /// This request builder is designed for flexibility. It may accept data which
        /// can be either borrowed or owned.
        ///
        /// You might think that providing only owned (`'static`) data is necessary to spawn the
        /// request in a separate task (e.g., with `tokio::spawn`). **This is not the case.**
        ///
        /// The builder itself may have a limited lifetime when using borrowed data. However,
        /// when you're ready to send the request, calling `.await` or `.execute()` consumes the
        /// builder and returns a `'static` future. This future contains the fully serialized
        /// request and no longer depends on the original borrowed data. You can freely `spawn`
        /// this final future.
        ///
        /// Note that this process consumes the builder. You cannot, for example, call `.execute()`
        /// and then continue to use the same builder instance to configure a different request.
        #[must_use = concat!(stringify!($name), " does nothing unless you `.await` or `.execute().await` it")]
        pub struct $name $(<$life>)? {
            pub(crate) request:
                $crate::client::PendingRequest<'static, SimpleOutput!(@get_payload $($payload)?) $(,$query)?>,
            // PhantomData to associate the builder with the specified lifetime for future use.
            $(_marker: std::marker::PhantomData<&$life ()>)?
        }

        impl$(<$life>)? $name$(<$life>)? {
            /// Specifies the authentication token to use for this API request.
            ///
            /// This is particularly useful for applications managing multiple WhatsApp Business Accounts (WBAs)
            /// where different API tokens might be required for various operations.
            /// It allows you to reuse a manager instance and apply the appropriate token
            /// for each specific request without re-initializing the `Client`.
            ///
            /// If not called, the request will use the authentication configured with the `Client`
            /// that initiated this operation.
            ///
            /// # Parameters
            /// - `auth`: [`Auth`] to use for this specific request.
            ///
            /// [`Auth`]: crate::client::Auth
            #[inline]
            pub fn with_auth<'with_auth>(mut self, auth: impl $crate::ToValue<'with_auth, $crate::client::Auth>) -> Self {
                // TODO: We did not bind auth from onset... Do something... we can't just keep cloning.
                self.request = self.request.auth(auth.to_value().into_owned());
                self
            }
        }

        paste::paste! {
            IntoFuture! {
                impl$(<$life>)? $name$(<$life>)? {
                    /// Executes the API request and returns the result.
                    ///
                    #[doc = "This method performs the network operation. Because `" $name "`"]
                    /// implements `IntoFuture`, you can also simply `.await` the
                    #[doc = "`" $name "` instance directly, which will call this method internally."]
                    ///
                    /// # Example
                    /// ```rust,no_run
                    /// # use whatsapp_business_rs::{app::ConfigureWebhook, Error};
                    #[doc = "# async fn _example(" [<$name:snake>] ": ConfigureWebhook<'_>) -> Result<(), Error> {"]
                    /// // Using .await directly (preferred)
                    #[doc = " " [<$name:snake>] ".await?;"]
                    /// # Ok(()) }
                    #[doc = "# async fn _example_exe(" [<$name:snake>] ": ConfigureWebhook<'_>) -> Result<(), Error> {"]
                    /// // Using .execute().await
                    #[doc = " " [<$name:snake>] ".execute().await?;"]
                    /// # Ok(()) }
                    /// ```
                    #[inline]
                    pub fn execute(self) -> impl Future<Output = Result<$execute, $crate::error::Error>> + 'static /*$(+ use<$life>)?*/{
                        $crate::rest::execute_request(self.request)
                    }
                }
            }
        }

        impl$(<$life>)? $name$(<$life>)? {
            #[inline(always)]
            pub(crate) fn request(self) ->
                $crate::client::PendingRequest<'static, SimpleOutput!(@get_payload $($payload)?) $(,$query)?> {
                self.request
            }
        }

        // Feature-gate the batching implementation.
        #[cfg(feature = "batch")]
        SimpleOutputBatch! {
            $name <$($life,)?> => $execute
        }
    };

    // Internal rule to determine the payload type.
    // If a payload type is given, wrap it in `JsonObjectPayload`.
    (@get_payload $payload:ty) => {
        // everyone is object
        $crate::client::JsonObjectPayload<$payload>
    };
    // If no payload type is given, the payload is an empty unit type.
    (@get_payload ) => {
        ()
    };
}

/// Implements the ToValue trait to ergonomically convert owned (`T`) and borrowed (`&T`)
/// values into a `Cow<T>`.
///
/// This is useful for API builder methods that need to accept either owned or
/// borrowed data without forcing the caller to clone.
macro_rules! to_value {
    ($($name:ty)*) => {
        $(
            // Implement for the owned type `T`.
            impl $crate::ToValue<'_, $name> for $name {
                #[inline]
                fn to_value(self) -> std::borrow::Cow<'static, $name> {
                    std::borrow::Cow::Owned(self)
                }
            }

            // Implement for the borrowed type `&'a T`.
            impl<'a> $crate::ToValue<'a, $name> for &'a $name {
                #[inline]
                fn to_value(self) -> std::borrow::Cow<'a, $name> {
                    std::borrow::Cow::Borrowed(self)
                }
            }
        )*
    }
}

/// Implements common traits for a struct which represents a node that wraps a `String`.
///
/// It generates:
/// - `new()` and a getter method.
/// - `From<T: Into<String>>`.
/// - `Serialize` and `Deserialize` implementations that treat the type as a plain string.
macro_rules! NodeImpl {
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        |Field_Vis|: $vis:vis,
        |Field_Attr|: $($field_attr:meta)*,
        |Field_Name|: $field:ident,
        |Field_Type|: $string:ty,
    } => {
        impl $name {
            /// Creates a new instance from anything convertible into a `String`
            #[inline]
            pub fn new($field: impl Into<String>) -> Self {
                Self {$field: $field.into()}
            }

            /// Returns a string slice of the inner value.
            #[inline]
            pub fn $field(&self) -> &str {
                &self.$field
            }
        }

        // Allow creating the newtype from strings.
        impl<T: Into<String>> From<T> for $name {
            #[inline]
            fn from(value: T) -> Self {
                Self::new(value)
            }
        }

        // Serialize as a plain string.
        impl serde::Serialize for $name {
            #[inline]
            fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
                ser.serialize_str(&self.$field)
            }
        }

        // Deserialize from a plain string.
        impl<'de> serde::Deserialize<'de> for $name {
            #[inline]
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Ok(Self { $field: String::deserialize(d)? })
            }
        }

        // Implement the `ToValue` trait for ergonomic use in builders.
        to_value! { $name }
    }
}

/// A complex macro for building a type-safe, multi-step workflow or state machine.
///
/// This macro uses a "TT muncher" pattern to recursively generate `impl` blocks.
/// Each function call advances the state of the builder struct, which is tracked
/// using generic parameters that default to `()`. This ensures that steps can
/// only be called in the correct order.
macro_rules! flow {
    // Entry point: Defines the main struct with generic parameters for each step.
    {
        $(#[$cont_attr:meta])*
        $vis:vis struct $name:ident <$($life:lifetime,)* $($generic:ident),* $(,)?> {
            $(
                $(#[$field_attr:meta])*
                $field_vis:vis $field:ident: $ty:ty,
            )+
        }
        // The sequence of steps to be implemented.
        $($step:tt)*
    } => {
        // Define the struct, defaulting all generic "state" parameters to the unit type `()`.
        $(#[$cont_attr])*
        $vis struct $name <$($life,)* $($generic = (),)*> {
            $(
                $(#[$field_attr])*
                $field_vis $field: $ty,
            )+
        }

        // Kick off the recursive "TT muncher" to generate the implementation for each step.
        flow! {
            @munch
            // The constant name of the struct.
            |Name|: $name,
            // The constant lifetimes.
            |Lives|: $($life)*,
            // Generic parameters for states that have been completed. Starts empty.
            |Params|: > ,
            // Generic parameters for states that are yet to be completed.
            |LeftParams|: $($generic)*,
            // All struct fields in order.
            |Fields|: > > [$($field,)*],
            // The steps to implement.
            |Steps|: $($step)*
        }
    };
    // Recursive arm (the "TT muncher"): Processes one step at a time.
    {
        @munch
        |Name|: $name:ident,
        |Lives|: $($life:lifetime)*,
        |Params|: $($prev:ty)* > $($just_now:ty)?,
        |LeftParams|: $to_shed:ident $($generic:ident)*,
        |Fields|: $($prev_field:ident)* > $($just_now_field:ident)? > [$to_be_added:ident, $($other_field:ident,)*],
        |Steps|: $(#[$func_attr:meta])* $(#![$optional:ident])? $vis:vis $async:ident fn $next:ident$(<$f_life:lifetime>)?($($arg:tt)*) -> $step_ty:ty $body:block $($other:tt)*
    } => {
        // Implement the function for the current state.
        impl<$($life),*> $name <$($life,)* $($prev,)* $($just_now)?> {
            $(#[$func_attr])*
            $vis $async fn $next$(<$f_life>)?($($arg)*) ->
                Result<$name <$($life,)* $($prev,)* $($just_now,)? $step_ty>, $crate::Error> {
                // The body is expected to return the value for the new field.
                let (to_be_added, state) = $body;
                Ok($name {
                    // Carry over the fields from previous steps.
                    $(
                        $prev_field: state.$prev_field,
                    )*
                    $(
                        $just_now_field: state.$just_now_field,
                    )?
                    // Add the new field from this step.
                    $to_be_added: to_be_added,
                    // Initialize remaining fields to their default state.
                    $(
                        $other_field: state.$other_field,
                    )*
                })
            }

            $vis fn previous(self) -> $name <$($life,)* $($prev),*> {
                $name {
                    $(
                        $prev_field: self.$prev_field,
                    )*
                    $(
                        $just_now_field: (),
                    )?
                    $to_be_added: (),
                    $(
                        $other_field: self.$other_field,
                    )*
                }
            }

            paste::paste! {
                /// Manually sets the data for this step, bypassing the sequential flow.
                ///
                /// This is useful when the data for a step has been obtained through
                /// other means (e.g., from a previous session) and you wish to "jump"
                /// to the next step in the flow.
                $vis fn [<set_ $to_be_added>](self, $to_be_added: impl Into<$step_ty>) -> $name <$($life,)* $($prev,)* $($just_now,)? $step_ty> {
                    $name {
                        // Carry over the fields from previous steps.
                        $(
                            $prev_field: self.$prev_field,
                        )*
                        $(
                            $just_now_field: self.$just_now_field,
                        )?
                        // Add the new field from this step.
                        $to_be_added: $to_be_added.into(),
                        // Initialize remaining fields to their default state.
                        $(
                            $other_field: self.$other_field,
                        )*
                    }
                }
            }

            // More like FlowStepRuined rn... the following step and steps should anticipate this by accepting
            // any T (which may scatter the steps) or a specific one that indicates skip
            // (We introduced FlowStepSkipped for this)... but this just snowballs except maybe we use traits.
            // Since only share_credit_line needs this for now and it's the last... we concede for now.
            // We won't use Option to avoid an unneccessary runtime check
            flow! {
                @optional
                $($optional)?|
                $vis fn skip(self) -> $name <$($life,)* $($prev,)* $($just_now,)? $crate::FlowStepSkipped> {
                    $name {
                        $(
                            $prev_field: self.$prev_field,
                        )*
                        $(
                            $just_now_field: self.$just_now_field,
                        )?
                        $to_be_added: $crate::FlowStepSkipped,
                        $(
                            $other_field: self.$other_field,
                        )*
                    }
                }
            }
        }

        // `Unlock` the previous data `globally`
        flow! {
            @optional
            $($just_now_field)?|
            impl<$($life),*, $to_shed, $($generic),*>
                $name <$($life,)* $($prev,)* $($just_now,)? $to_shed, $($generic),*> {
                // FIXME: Horrible internal names get exposed
                $vis fn $($just_now_field)?(&self) -> &$($just_now)? {
                    &self.$($just_now_field)?
                }
            }
        }

        flow! {
            @munch
            |Name|: $name,
            |Lives|: $($life)*,
            |Params|:  $($prev)* $($just_now)? > $step_ty,
            |LeftParams|: $($generic)*,
            |Fields|: $($prev_field)* $($just_now_field)? > $to_be_added > [$($other_field,)*],
            |Steps|: $($other)*
        }
    };
    // Base case: No steps left to implement, so we stop the recursion.
    {
        @munch
        |Name|: $name:ident,
        |Lives|: $($life:lifetime)*,
        |Params|: $($prev:ty)* > $($just_now:ty)?,
        |LeftParams|: $($generic:ident)*,
        |Fields|: $($prev_field:ident)* > $($just_now_field:ident)? > [$($residue:ident,)*],
        |Steps|:
    } => {};
    {@optional $exists:tt| $($imp:tt)*} => {
        $($imp)*
    };
    {@optional | $($imp:tt)*} => {}
    // {
    //     $($whatever:tt)*
    // } => {}
}

// Only for enum
macro_rules! FieldsTrait {
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        $(
            |Variant_Attr|: $($variant_attr:meta)*,
            |Variant_Name|: $variant:ident,
            |Variant_Type|: ,
        )*
    } => {
        impl $crate::rest::FieldsTrait for $name {
            const ALL: &'static [Self] = &[
                $(
                    Self::$variant,
                )*
            ];

            #[inline]
            fn as_snake_case(&self) -> &'static str {
                match self {
                    $(
                        Self::$variant => paste::paste!{stringify!([<$variant:snake>])},
                    )*
                }
            }
        }
    }
}

/// Implements `Deserialize` for a struct where one field can have several different names.
///
/// For example, a JSON object might contain either a `"text"` field or a `"body"` field,
/// but they should both map to the same struct field.
///
/// This macro generates a custom `Visitor` and deserializes into a helper struct
/// before mapping the fields to the final struct.
macro_rules! AnyField {
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        // The first field is the special "any" field.
        |Field_Vis|: $any_field_vis:vis,
        |Field_Attr|: anyfield($($any_field:literal),*) $($any_field_attr:meta)*,
        |Field_Name|: $any_field_ident:ident,
        |Field_Type|: $any_field_ty:ty,
        // The rest of the fields are standard.
        $(
            |Field_Vis|: $field_vis:vis,
            |Field_Attr|: $($field_attr:meta)*,
            |Field_Name|: $field:ident,
            |Field_Type|: $ty:ty,
        )*
    } => {
        paste::paste! {
            // Implement `Deserialize` for the user's struct.
            impl<'a> serde::Deserialize<'a> for $name {
                #[inline]
                fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                    where
                        D: serde::Deserializer<'a> {
                    // Step 1: Deserialize into the intermediate helper struct.
                    let helper = [<__AnyFieldHelperFor $name>]::deserialize(deserializer)?;
                    // SAFETY: These are literally thesame structs under different names so they'll always
                    // have thesame layout
                    Ok(unsafe { std::mem::transmute::<[<__AnyFieldHelperFor $name>], $name>(helper) })
                }
            }

            // A private helper struct used for the intermediate deserialization.
            // Its layout matches the target struct, which is why `transmute` is safe.
            #[derive(serde::Deserialize)]
            // Pass through any container attributes (#[repr(whatever)] happens to the twin too)
            $(#[$cont_attr])*
            #[doc(hidden)]
            #[allow(dead_code)]
            pub(crate) struct [<__AnyFieldHelperFor $name>]$(<$($life),* $($generic),*>)? {
                // The "any" field is flattened from a special newtype.
                #[serde(flatten)]
                $(#[$any_field_attr])*
                $any_field_ident: [<__AnyFieldPayloadFor $name>],
                // The rest of the fields are deserialized normally.
                $(
                    $(#[$field_attr])*
                    $field: $ty,
                )*
            }

            // We could use enum but we want to make the helper the same size
            // as the main so we could transmute...
            #[allow(dead_code)]
            struct [<__AnyFieldPayloadFor $name>]($any_field_ty);

            // Custom `Deserialize` for the newtype to implement the "any field" logic.
            impl<'a> serde::Deserialize<'a> for [<__AnyFieldPayloadFor $name>] {
                #[inline]
                fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                    where
                        D: serde::Deserializer<'a> {
                    deserializer.deserialize_map([<__AnyFieldVisitorFor $name>])
                }
            }

            // A Serde visitor to find one of the allowed field names.
            #[doc(hidden)]
            struct [<__AnyFieldVisitorFor $name>];

            impl<'de> serde::de::Visitor<'de> for [<__AnyFieldVisitorFor $name>]  {
                type Value = [<__AnyFieldPayloadFor $name>];

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    write!(formatter, "struct {} with any of the fields: {:#?}.",
                           stringify!([<__AnyFieldPayloadFor $name>]),
                           [$($any_field),*])
                }

                #[inline]
                fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                    where
                        A: serde::de::MapAccess<'de>, {
                    #[derive(serde::Deserialize)]
                    #[allow(non_camel_case_types)]
                    enum Field {
                        $(
                            [<$any_field>],
                        )*
                    }

                    // Find the first key-value pair that matches one of our expected fields.
                    let (_, v): (Field, $any_field_ty) = map.next_entry()?.ok_or_else(|| {
                        <A::Error as serde::de::Error>::custom(format!("missing one of the fields: {:#?}", [$($any_field),*]))
                    })?;
                    Ok([<__AnyFieldPayloadFor $name>](v))
                }
            }
        }
    };
}

/// Implements a builder pattern for a struct through Update.
///
/// It generates a setter method for each field of the struct. These setters
/// consume and return `Update` to allow for chaining.
///
/// This macro relies on the `BuilderInto` trait to automatically handle
/// converting input values into `Option<T>` fields.
macro_rules! Update {
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        $(
            |Field_Vis|: $field_vis:vis,
            |Field_Attr|: $($field_attr:meta)*,
            |Field_Name|: $field:ident,
            |Field_Type|: $ty:ty,
        )*
    } => {
    	paste::paste! (
            impl<U> $crate::Update<'_, $name, U> {
                $(
                    #[doc = "Sets the `" $field "` field on the update payload."]
                    ///
                    /// You may chain multiple field setters before calling `.await`.
                    ///
                    /// # Example
                    /// ```ignore (too-stressful)
                    #[doc = "let update = update." $field "(" [<new_ $field>] ");"]
                    /// ```
                    #[inline]
                    $field_vis fn $field<V>(mut self, $field: V) -> Self
                    where
                        // Use the helper trait to convert the input value.
                        V: BuilderInto<$ty>
                    {
                        self.request.payload.0.$field = $field.builder_into();
                        self
                    }
                )*
            }
        );
    }
}

pub(crate) trait BuilderInto<T>: Sized {
    fn builder_into(self) -> T;
}

impl<T, I: Into<T>> BuilderInto<Option<T>> for I {
    #[inline]
    fn builder_into(self) -> Option<T> {
        Some(self.into())
    }
}

macro_rules! Builder {
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        $(
            |Field_Vis|: $field_vis:vis,
            |Field_Attr|: $($field_attr:meta)*,
            |Field_Name|: $field:ident,
            |Field_Type|: $ty:ty,
        )*
    } => {
        // We can't filter ty for Option so let's use trait for now
        paste::paste!(
            impl $name {
                $(
                    #[doc = "Sets the `" $field "` field of `" $name "`."]
                    #[inline]
                    $field_vis fn $field<V>(mut self, $field: V) -> Self
                    where
                        V: BuilderInto<$ty>
                    {
                        self.$field = $field.builder_into();
                        self
                    }
                )*
            }
        );
    }
}

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

macro_rules! Fields {
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        $(
            |Field_Vis|: $field_vis:vis,
            |Field_Attr|: $($field_attr:meta)*,
            |Field_Name|: $field:ident,
            |Field_Type|: $ty:ty,
        )*
    } => {
        paste::paste!(
            derive! {
                #[derive(#FieldsTrait, Serialize, PartialEq, Eq, Hash, Clone, Copy, Debug)]
                #[serde(rename_all = "snake_case")]
                pub enum [<$name Field>] {
                    $(
                        [<$field:camel>],
                    )*
                }
            }
        );
    };
}

macro_rules! ContentTraits {
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        |Variant_Attr|: $($media_attr:meta)*,
        |Variant_Name|: Media,
        |Variant_Type|: $media_ty:ty,
        $(
            |Variant_Attr|: $($variant_attr:meta)*,
            |Variant_Name|: $variant:ident,
            |Variant_Type|: $($ty:ty)?,
        )*
    } => {
        ContentTraits! {
            |Name| => $name
            // to avoid ambiguity
            |Variants| => $($(#[$variant_attr])* |variant|: $variant, |data|: $($ty)?)*
            |Medias| => Image Audio Video Document Sticker, |data|: $media_ty
        }
    };
    (
        |Name| => $name:ident
        |Variants| => $($(#[$variant_attr:meta])* |variant|: $variant:ident, |data|: $($data:ty)?)*
        |Medias| => $($media_type:ident)*, |data|: $media_ty:ty
    ) => {
        paste::paste!(
            impl $crate::rest::IntoMessageRequest for $name {
                type Output = [<$name Output>];

                #[inline]
                fn into_request<'i, 't>(
                    self,
                    manager: &$crate::client::MessageManager<'i>,
                    to: &'t $crate::IdentityRef,
                ) -> Self::Output {
                    match self {
                        Self::Media(inner) => {
                            match inner.media_type {
                                $(
                                    MediaType::$media_type(_) =>
                                        Self::Output::$media_type(inner.into_request(manager, to)),
                                )+
                            }
                        },
                        $(
                            Self::$variant $(([<$data:snake>]))? =>
                                Self::Output::$variant $(([<$data:snake>]
                                        .into_request(manager, to)))?,
                        )*
                    }
                }
            }

            pub(crate) enum [<$name Output>] {
                $(
                    $media_type(<$media_ty as $crate::rest::IntoMessageRequest>::Output),
                )+
                 $(
                    $variant$((<$data as $crate::rest::IntoMessageRequest>::Output))?,
                )*
            }

            impl $crate::rest::IntoMessageRequestOutput for [<$name Output>] {
                type Request = [<$name Request>];

                #[inline]
                fn with_auth(self, auth: std::borrow::Cow<'_, $crate::client::Auth>) -> Self {
                    match self {
                        $(
                            Self::$media_type(media) => Self::$media_type(media.with_auth(auth)),
                        )*
                        $(
                            Self::$variant $(([<$data:snake>]))? =>
                            Self::$variant $(([<$data:snake>].with_auth(auth)))?,
                        )*
                    }
                }

                #[inline]
                async fn execute(self) -> Result<Self::Request, $crate::error::Error> {
                    Ok(match self {
                        $(
                            Self::$media_type(media) => Self::Request::$media_type(media.execute().await?),
                        )*
                        $(
                            Self::$variant $(([<$data:snake>]))? =>
                            Self::Request::$variant $(([<$data:snake>].execute().await?))?,
                        )*
                    })
                }
            }

            derive! {
                #[derive(Debug, #SerializeAdjacent)]
                // I think the implicit puppetteering (transumting) in impl_adjacent_serde made the
                // linter think we're never reading or constructing the variants
                #[allow(dead_code)]
                pub(crate) enum [<$name Request>] {
                    $(
                        $media_type(<<$media_ty as $crate::rest::IntoMessageRequest>::Output as $crate::rest::IntoMessageRequestOutput>::Request),
                    )+

                    $(
                        $(#![$variant_attr])*
                        $variant $((<<$data as $crate::rest::IntoMessageRequest>::Output as $crate::rest::IntoMessageRequestOutput>::Request))?,
                    )*
                }
            }

            #[cfg(feature = "batch")]
            impl $crate::batch::Requests for [<$name Output>] {
                type BatchHandler = [<$name Handler>];

                type ResponseReference = <Self as $crate::rest::IntoMessageRequestOutput>::Request;

                #[inline]
                fn into_batch_ref(
                    self,
                    batch_serializer: &mut $crate::batch::BatchSerializer,
                ) -> Result<(Self::BatchHandler, Self::ResponseReference), $crate::batch::FormatError> {
                    Ok(match self {
                        $(
                            Self::$media_type(media) => {
                                let (h, r) = media.into_batch_ref(batch_serializer)?;
                                (Self::BatchHandler::$media_type(h), Self::ResponseReference::$media_type(r))
                            },
                        )*
                        $(
                            Self::$variant $(([<$data:snake>]))? => {
                                let (h, r) = $([<$data:snake>].into_batch_ref(batch_serializer)?)?; // FIXME: None is empty for now
                                (Self::BatchHandler::$variant(h), Self::ResponseReference::$variant(r))
                            }
                        )*
                    })
                }

                #[inline]
                fn size_hint(&self) -> (usize, Option<usize>) {
                    match self {
                        $(
                            Self::$media_type(media) => media.size_hint(),
                        )*
                        $(
                            Self::$variant $(([<$data:snake>]))? => $([<$data:snake>].size_hint())?,
                        )*
                    }
                }

                requests_batch_include! {}
            }

            #[cfg(feature = "batch")]
            pub(crate) enum [<$name Handler>] {
                $(
                    $media_type(<<$media_ty as $crate::rest::IntoMessageRequest>::Output as $crate::batch::Requests>::BatchHandler),
                )+
                 $(
                    $variant$((<<$data as $crate::rest::IntoMessageRequest>::Output as $crate::batch::Requests>::BatchHandler))?,
                )*
            }

            #[cfg(feature = "batch")]
            impl $crate::batch::Handler for [<$name Handler>] {
                // We don't need the response... mm stressed
                type Responses = ();

                #[inline]
                fn from_batch(self, response: &mut $crate::batch::BatchResponse)
                    -> Result<Self::Responses, $crate::batch::ResponseProcessingError> {
                     match self {
                        $(
                            Self::$media_type(media_res) => {
                                let _ = media_res.from_batch(response)?;
                                Ok(())
                            },
                        )+

                        $(
                            Self::$variant(v) => {
                                let _ = v.from_batch(response)?;
                                Ok(())
                            },
                        )*
                     }
                }
            }

            impl<'from_response> $crate::rest::FromResponse<'from_response> for $name {
                type Response = [<$name Response>]<'from_response>;

                #[inline]
                fn from_response(response: Self::Response) -> Result<Self, $crate::error::ServiceErrorKind> {
                    match response {
                        $(
                            Self::Response::$media_type(media_res) => {
                                Ok(Self::Media(<$media_ty as $crate::rest::FromResponse>::from_response(media_res)?))
                            },
                        )+

                        $(
                            Self::Response::$variant $(([<$data:snake>]))? => {
                                Ok(Self::$variant $((<$data as $crate::rest::FromResponse>::from_response([<$data:snake>])?))?)
                            },
                        )*
                    }
                }
            }

            // practically thesame as the request counterpart
            derive! {
                #[derive(Debug, #DeserializeAdjacent)]
                #[allow(dead_code)]
                pub(crate) enum [<$name Response>]<'from_response> {
                    $(
                        #![serde(borrow)]
                        $media_type(<$media_ty as $crate::rest::FromResponse<'from_response>>::Response),
                    )+

                    $(
                        #![serde(borrow)]
                        $(#![$variant_attr])*
                        $variant $((<$data as $crate::rest::FromResponse<'from_response>>::Response))?,
                    )*
                }
            }

            impl From<$media_ty> for $name {
                #[inline]
                fn from(value: $media_ty) -> Self {
                    $name::Media(value)
                }
            }

            impl PartialEq<$media_ty> for $name {
                fn eq(&self, other: &$media_ty) -> bool {
                    if let $name::Media(media) = self {
                        media.eq(other)
                    } else {
                        false
                    }
                }
            }

            $(
                $(
                    impl From<$data> for $name {
                        fn from(value: $data) -> Self {
                            $name::$variant(value)
                        }
                    }

                    impl PartialEq<$data> for $name {
                        fn eq(&self, other: &$data) -> bool {
                            if let $name::$variant(inner) = self {
                                inner.eq(other)
                            } else {
                                false
                            }
                        }
                    }
                )?
            )*

            // FIXME: We should have Into<Text> here
            impl<T: Into<String> + Send> From<T> for $name {
                #[inline]
                fn from(value: T) -> Self {
                    $name::Text(value.into().into())
                }
            }

            impl<T> PartialEq<T> for $name
            where
                String: PartialEq<T>
            {
                fn eq(&self, other: &T) -> bool {
                    if let $name::Text(text) = self {
                        text.eq(other)
                    } else {
                        false
                    }
                }
            }

            ContentTraits! {
                @IntoDraft $name|
                // this is only for the main media
                impl $crate::message::IntoDraft for $media_ty {
                    #[inline]
                    fn into_draft(self) -> Draft {
                        Draft::media(self)
                    }
                }

                $(
                    $(
                        impl $crate::message::IntoDraft for $data {
                            #[inline]
                            fn into_draft(self) -> $crate::Draft {
                                $crate::message::Draft {
                                    content: $name::$variant(self),
                                    ..Default::default()
                                }
                            }
                        }
                    )?
                )*

                macro_rules! into_draft_string {
                    ($ty:ty) => {
                        impl<'_life > IntoDraft for $ty {
                            #[inline]
                            fn into_draft(self) -> Draft {
                                Draft::text(self)
                            }
                        }
                    }
                }
                impl_common_strings! {
                    into_draft_string
                }
            }
        );
  };
    {
        @IntoDraft InteractiveHeader| $($conflicting:tt)*
    } => {};
    {
        @IntoDraft Content| $($first:tt)*
    } => {$($first)*};
}

// A helper struct for meta's adjacently tagged enum representation.
// This pattern serializes to `{"type": "variant_name", "variant_name": ...}`.
#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct AdjacentHelper<'a, T> {
    // The 'type' field holds the name of the enum variant.
    #[serde(borrow, default)]
    pub(crate) r#type: Cow<'a, str>,
    // The 'inner' field holds the actual data of the variant.
    // `#[serde(flatten)]` embeds the fields of `inner` directly into this struct.
    #[serde(flatten)]
    pub(crate) inner: T,
}

/// Macro for adjacent enum deserialization
///
/// Use only when enum name matches the intended serialization
/// tag
macro_rules! DeserializeAdjacent {
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        $(
            |Variant_Attr|: $($variant_attr:meta)*,
            |Variant_Name|: $variant:ident,
            |Variant_Type|: $($ty:ty)?,
        )*
    } => {
        paste::paste!(
            // This is a private helper enum generated by the macro.
            // It has the same variants and structure as the target enum, allowing
            // it to be used with `#[derive(Serialize, Deserialize)]` to prevent
            // double implementations of these traits for the target under a different
            // name.
            #[derive(serde::Deserialize)]
            // Pass through any container attributes (#[repr(whatever)] happens to the twin too)
            $(#[$cont_attr])*
            #[serde(rename_all = "snake_case")]
            #[doc(hidden)]
            #[allow(dead_code)]
            pub(crate) enum [<$name DeAdjacentHelper>] $(<$($life),* $($generic),*>)? {
                $(
                    $(#[$variant_attr])*
                    // Define the variant, including its data type if it has one.
                    $variant $(($ty))?,
                )+
            }

            impl<'de, $($($life),* $($generic),*)?> Deserialize<'de> for $name $(<$($life),* $($generic),*>)? {
                #[inline]
                fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                    // First, deserialize into the generic AdjacentHelper containing our generated helper enum.
                    let helper = $crate::rest::AdjacentHelper::<[<$name DeAdjacentHelper>]>::deserialize(d)?;

                    // SAFETY: These are literally thesame enums under different names so they'll always
                    // have thesame layout
                    Ok(unsafe { std::mem::transmute::<[<$name DeAdjacentHelper>], $name>(helper.inner) })
                }
            }
        );
    }
}

/// Macro for adjacent enum serialization
///
/// Use only when enum name matches the intended serialization
/// tag
macro_rules! SerializeAdjacent {
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        $(
            |Variant_Attr|: $($variant_attr:meta)*,
            |Variant_Name|: $variant:ident,
            |Variant_Type|: $($ty:ty)?,
        )*
    } => {
        paste::paste!(
            // This is a private helper enum generated by the macro.
            // It has the same variants and structure as the target enum, allowing
            // it to be used with `#[derive(Serialize, Deserialize)]` to prevent
            // double implementations of these traits for the target under a different
            // name.
            #[derive(serde::Serialize)]
            // Pass through any container attributes (#[repr(whatever)] happens to the twin too)
            $(#[$cont_attr])*
            #[serde(rename_all = "snake_case")]
            #[doc(hidden)]
            #[allow(dead_code)]
            pub(crate) enum [<$name SerAdjacentHelper>] $(<$($life),* $($generic),*>)? {
                $(
                    $(#[$variant_attr])*
                    // Define the variant, including its data type if it has one.
                    $variant$(($ty))?,
                )+
            }

            impl$(<$($life),* $($generic),*>)? Serialize for $name $(<$($life),* $($generic),*>)? {
                #[inline]
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    // Determine the string for the 'type' field based on the variant.
                    let type_str = match self {
                        $(
                            Self::$variant {..} => {
                                // NOTE: this does expand to the intended value and not
                                // "[<$variant:snake>]" as long as paste::paste! call is above.
                                // it searches for the pattern expands and stringify sees the
                                // actual value
                                stringify!([<$variant:snake>])
                            }
                        )+
                    };

                    let e = $crate::rest::AdjacentHelper {
                        r#type: std::borrow::Cow::Borrowed(type_str),
                        // SAFETY: These are literally thesame enums under different names so they'll always
                        // have thesame layout.
                        //
                        // Trying to avoid this will likely involve heavy unnecessary cloning since we're behind
                        // a reference.
                        inner: unsafe {
                            std::mem::transmute::<&$name, &[<$name SerAdjacentHelper>]>(self)
                        },
                    };

                    e.serialize(s)
                }
            }
        );
    }
}

/// Implements the `FromResponse` trait for a struct or enum.
///
/// This macro generates a corresponding `[Name]Response` type that can be deserialized
/// from a raw API response, and then converts from that intermediate type to the final
/// user-facing type
macro_rules! FromResponse {
    // Macro arm for structs.
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        $(
            |Field_Vis|: $field_vis:vis,
            |Field_Attr|: $($field_attr:meta)*,
            |Field_Name|: $field:ident,
            |Field_Type|: $ty:ty,
        )*
    } => {
        paste::paste! {
            // Implement the primary conversion trait for the user's struct.
            impl<'from_response, $($($life,)* $($generic,)*)?> $crate::rest::FromResponse<'from_response> for $name $(<$($life,)* $($generic),*>)?
            where
                // Add trait bounds for all field types.
                $($ty: $crate::rest::FromResponse<'from_response>,)*
            {
                // Associate the generated response helper struct.
                type Response = [<$name Response>]<'from_response, $($($life,)* $($generic,)*)?>;

                #[inline]
                fn from_response(
                    response: Self::Response,
                ) -> Result<Self, $crate::error::ServiceErrorKind> {
                    // Convert from the response helper to the target struct by converting each field.
                    Ok(Self {
                        $(
                            $field: <$ty as $crate::rest::FromResponse>::from_response(response.$field)?,
                        )+
                    })
                }
            }

            // The intermediate response helper struct. This is what Serde deserializes into.
            #[derive(::serde::Deserialize)]
            $(#[$cont_attr])*
            #[serde(bound(deserialize = "'de: 'from_response, 'from_response: 'de"))]
            #[doc(hidden)]
            pub(crate) struct [<$name Response>]<'from_response, $($($life,)* $($generic),*)?>
            where
                $($ty: $crate::rest::FromResponse<'from_response>,)*
            {
                $(
                    $(#[$field_attr])*
                    // Each field in the helper is the `Response` type of the corresponding field in the target struct.
                    $field: <$ty as $crate::rest::FromResponse<'from_response>>::Response,
                )+
            }

            // Manually implementing Debug to avoid unnecessary `T: Debug` bounds from a derive.
            impl<'from_response, $($($life,)* $($generic),*)?> std::fmt::Debug for [<$name Response>]<'from_response, $($($life,)* $($generic),*)?>
            where
                $($ty: $crate::rest::FromResponse<'from_response>,)*
            {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    let mut debug_struct = f.debug_struct(stringify!([<$name Response>]));
                    $(
                        debug_struct.field(stringify!($field), &self.$field);
                    )+
                    debug_struct.finish()
                }
            }
        }
    };

    // Macro arm for enums.
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        $(
            |Variant_Attr|: $($variant_attr:meta)*,
            |Variant_Name|: $variant:ident,
            |Variant_Type|: $ty:ty,
        )*
    } => {
        paste::paste! {
            impl<'from_response, $($($life,)* $($generic,)*)?> $crate::rest::FromResponse<'from_response> for $name $(<$($life,)* $($generic),*>)?
            where
                $($ty: $crate::rest::FromResponse<'from_response>,)*
            {
                type Response = [<$name Response>]<'from_response, $($($generic),*)?>;

                #[inline]
                fn from_response(
                    response: Self::Response,
                ) -> Result<Self, $crate::error::ServiceErrorKind> {
                    // Convert by matching on the response enum and converting the inner value.
                    Ok(match response {
                        $(
                            Self::Response::$variant(inner) =>
                                Self::$variant(<$ty as $crate::rest::FromResponse>::from_response(inner)?),
                        )*
                    })
                }
            }

            // The intermediate response helper enum.
            #[derive(Debug, ::serde::Deserialize)]
            $(#[$cont_attr])*
            #[doc(hidden)]
            pub(crate) enum [<$name Response>]<'from_response, $($($life,)* $($generic),*)?>
            where
                $($ty: $crate::rest::FromResponse<'from_response>,)*
            {
                $(
                    #[serde(borrow)]
                    $(#[$variant_attr])*
                    $variant(<$ty as $crate::rest::FromResponse<'from_response>>::Response),
                )+
            }
        }
    }
}

macro_rules! IntoRequest {
    // Macro arm for structs
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        $(
            |Field_Vis|: $field_vis:vis,
            |Field_Attr|: $($field_attr:meta)*,
            |Field_Name|: $field:ident,
            |Field_Type|: $ty:ty,
        )*
    } => {
        paste::paste! {
            // Stage 1: Convert the user's type into the intermediate 'Output' type.
            impl<$($($life,)* $($generic,)*)?> $crate::rest::IntoMessageRequest for $name $(<$($life),* $($generic),*>)?
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
            {
                type Output = [<$name Output>]<$($($generic),*)?>;

                #[inline]
                fn into_request<'i, 't>(
                    self,
                    manager: &$crate::client::MessageManager<'i>,
                    to: &'t $crate::IdentityRef,
                ) -> Self::Output {
                    Self::Output {
                        $(
                            $field: self.$field.into_request(manager, to),
                        )+
                    }
                }
            }

            // The intermediate "Output" struct. This is the pre-execution stage.
            #[doc(hidden)]
            pub(crate) struct [<$name Output>]<$($($life),* $($generic),*)?>
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
            {
                $(
                    $field: <$ty as $crate::rest::IntoMessageRequest>::Output,
                )*
            }

            // Stage 2: Add configuration methods and the final `execute` method to the 'Output' type.
            impl $crate::rest::IntoMessageRequestOutput for [<$name Output>] {
                type Request = [<$name Request>]<$($($life),* $($generic),*)?>;

                #[inline]
                fn with_auth(self, auth: std::borrow::Cow<'_, $crate::client::Auth>) -> Self {
                    Self {
                        $(
                            // Delegate auth application to each field.
                            $field: self.$field.with_auth(std::borrow::Cow::Borrowed(&*auth)),
                        )*
                    }
                }

                #[inline]
                async fn execute(self) -> Result<Self::Request, $crate::error::Error> {
                    Ok(Self::Request {
                        $(
                            // Await the execution of each field's request logic.
                            $field: self.$field.execute().await?,
                        )*
                    })
                }
            }

            // Stage 3: The final, serializable "Request" struct.
            #[derive(Debug, ::serde::Serialize)]
            $(#[$cont_attr])*
            #[doc(hidden)]
            pub(crate) struct [<$name Request>]<$($($life),* $($generic),*)?>
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
            {
                $(
                    $(#[$field_attr])*
                    $field: <<$ty as $crate::rest::IntoMessageRequest>::Output as $crate::rest::IntoMessageRequestOutput>::Request,
                )+
            }

            // Batching implementation, guarded by the "batch" feature flag.
            #[cfg(feature = "batch")]
            mod [<batch_impl_ $name:snake>] {
                // We need to bring types and traits into scope for the macro expansion.
                use super::*;
                use $crate::batch::{Requests, Handler, BatchSerializer, BatchResponse, FormatError, ResponseProcessingError};

                impl Requests for [<$name Output>] {
                    type BatchHandler = [<$name Handler>];
                    type ResponseReference = <Self as $crate::rest::IntoMessageRequestOutput>::Request;

                    #[inline]
                    fn into_batch_ref(
                        self,
                        batch_serializer: &mut BatchSerializer,
                    ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
                        $(
                            let $field = self.$field.into_batch_ref(batch_serializer)?;
                        )*

                        let handler = Self::BatchHandler {
                            $(
                                $field: $field.0,
                            )*
                        };

                        let request = Self::ResponseReference {
                            $(
                                $field: $field.1,
                            )*
                        };

                        Ok((handler, request))
                    }

                    #[inline]
                    fn size_hint(&self) -> (usize, Option<usize>) {
                        ($( self.$field.size_hint().0 + )* 0, None)
                    }

                    requests_batch_include! {}
                }

                #[doc(hidden)]
                pub(crate) struct [<$name Handler>]
                where
                    $($ty: $crate::rest::IntoMessageRequest,)*
                {
                    $(
                        $field: <<$ty as $crate::rest::IntoMessageRequest>::Output as Requests>::BatchHandler,
                    )+
                }

                impl Handler for [<$name Handler>] {
                    type Responses = ();

                    #[inline]
                    fn from_batch(self, response: &mut BatchResponse)
                        -> Result<Self::Responses, ResponseProcessingError> {
                        $(
                            let _ = self.$field.from_batch(response)?;
                        )*
                        Ok(())
                    }
                }
            }
        }
    };
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        $(
            |Variant_Attr|: $($variant_attr:meta)*,
            |Variant_Name|: $variant:ident,
            |Variant_Type|: $ty:ty,
        )*
    } => {
        paste::paste! {
            // Stage 1: Convert the user's type into the intermediate 'Output' type.
            impl<$($($life,)* $($generic,)*)?> $crate::rest::IntoMessageRequest for $name $(<$($life),* $($generic),*>)?
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
            {
                type Output = [<$name Output>]<$($($generic),*)?>;

                #[inline]
                fn into_request<'i, 't>(
                    self,
                    manager: &$crate::client::MessageManager<'i>,
                    to: &'t $crate::IdentityRef,
                ) -> Self::Output {
                    match self {
                        $(
                            Self::$variant(inner) =>
                            Self::Output::$variant(inner.into_request(manager, to)),
                        )*
                    }
                }
            }

            // The intermediate "Output" enum. This is the pre-execution stage.
            #[doc(hidden)]
            pub(crate) enum [<$name Output>]<$($($life),* $($generic),*)?>
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
            {
                $(
                    $variant(<$ty as $crate::rest::IntoMessageRequest>::Output),
                )*
            }

            // Stage 2: Add configuration methods and the final `execute` method to the 'Output' type.
            impl $crate::rest::IntoMessageRequestOutput for [<$name Output>] {
                type Request = [<$name Request>]<$($($life),* $($generic),*)?>;

                #[inline]
                fn with_auth(self, auth: std::borrow::Cow<'_, $crate::client::Auth>) -> Self {
                    match self {
                        $(
                            // Delegate auth application to each variant.
                            Self::$variant(inner) =>
                            Self::$variant(inner.with_auth(auth)),
                        )*
                    }
                }

                #[inline]
                async fn execute(self) -> Result<Self::Request, $crate::error::Error> {
                    Ok(match self {
                        $(
                            // Await the execution of each variant's request logic.
                            Self::$variant(inner) =>
                            Self::Request::$variant(inner.execute().await?),
                        )*
                    })
                }
            }

            // Stage 3: The final, serializable "Request" struct.
            #[derive(Debug, ::serde::Serialize)]
            $(#[$cont_attr])*
            #[doc(hidden)]
            pub(crate) enum [<$name Request>]<$($($life),* $($generic),*)?>
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
            {
                $(
                    $(#[$variant_attr])*
                    $variant(<<$ty as $crate::rest::IntoMessageRequest>::Output as $crate::rest::IntoMessageRequestOutput>::Request),
                )+
            }


            // Batching implementation, guarded by the "batch" feature flag.
            #[cfg(feature = "batch")]
            mod [<batch_impl_ $name:snake>] {
                // We need to bring types and traits into scope for the macro expansion.
                use super::*;
                use $crate::batch::{Requests, Handler, BatchSerializer, BatchResponse, FormatError, ResponseProcessingError};

                impl Requests for [<$name Output>] {
                    type BatchHandler = [<$name Handler>];
                    type ResponseReference = <Self as $crate::rest::IntoMessageRequestOutput>::Request;

                    #[inline]
                    fn into_batch_ref(
                        self,
                        batch_serializer: &mut BatchSerializer,
                    ) -> Result<(Self::BatchHandler, Self::ResponseReference), FormatError> {
                        Ok(match self {
                            $(
                                Self::$variant(inner) => {
                                    let (h, r) = inner.into_batch_ref(batch_serializer)?;
                                    (Self::BatchHandler::$variant(h), Self::ResponseReference::$variant(r))
                                },
                            )*
                        })
                    }

                    #[inline]
                    fn size_hint(&self) -> (usize, Option<usize>) {
                        match self {
                            $(
                                Self::$variant(inner) => inner.size_hint(),
                            )*
                        }
                    }

                    requests_batch_include! {}
                }

                #[doc(hidden)]
                pub(crate) enum [<$name Handler>] {
                    $(
                        $variant(<<$ty as $crate::rest::IntoMessageRequest>::Output as $crate::batch::Requests>::BatchHandler),
                    )+
                }

                impl Handler for [<$name Handler>] {
                    type Responses = ();

                    #[inline]
                    fn from_batch(self, response: &mut BatchResponse)
                        -> Result<Self::Responses, ResponseProcessingError> {
                        match self {
                            $(
                                Self::$variant(inner) => {
                                    let _ = inner.from_batch(response)?;
                                    Ok(())
                                },
                            )+
                         }
                    }
                }
            }
        }
    }
}

/// A custom "derive" macro to generate boilerplate for request and response types.
///
/// This macro acts as a dispatcher. You provide it with the target struct or enum
/// and a list of helper macros to apply. The helper macros must be specified
/// using the `#` token to pass them by path.
///
/// # Example
///
/// ```rust,ignore
/// use whatsapp_business_rs::{derive, FromResponse, IntoRequest};
///
/// derive! {
///     #[derive(Debug, Clone, PartialEq, Eq, #FromResponse, #IntoRequest)]
///     pub struct MyApiType {
///         pub id: String,
///         pub value: i32,
///     }
/// }
/// ```
macro_rules! derive {
    // Arm for structs
    {
        $(#[doc $($doc:tt)*])*
        // Parse the derive attribute to extract our custom derive paths.
        #[derive($($($der_mac:ident)? $(#$mac:path)?),*)]
        // Parse container attributes.
        $(#![$($cont_attr:tt)*])*
        $(#[$($our_cont_attr:tt)*])*
        // Parse the struct definition.
        $vis:vis struct $name:ident $(<$($life:lifetime,)? $($generic:ident),*>)? {
            $(
                $(#[$($our_field_attr:tt)*])*
                $(#![$($field_attr:tt)*])*
                $field_vis:vis $field:ident: $ty:ty,
            )+
        }
    } => {
        // First, re-emit the original struct definition, but without our special derive paths.
        $(#[doc $($doc)*])*
        $(#[derive($($der_mac)?)])*
        $(#[$($our_cont_attr)*])*
        $vis struct $name $(<$($life,)? $($generic),*>)? {
            $(
                $(#[$($our_field_attr)*])*
                $field_vis $field: $ty,
            )+
        }

        // Call the internal `@apply` rule to execute each of the specified helper macros.
        derive!(@apply [$($($mac,)*)?]
            |Cont_Attr|: $($($cont_attr)*)*,
            |Name|: $name,
            $(
                |Lives|: $($life)?,
                |Generics|: $($generic)*,
            )?
            $(
                |Field_Vis|: $field_vis,
                |Field_Attr|: $($($field_attr)*)*,
                |Field_Name|: $field,
                |Field_Type|: $ty,
            )*
        );
    };

    // Arm for enums (similar logic to structs).
    {
        $(#[doc $($doc:tt)*])*
        #[derive($($($der_mac:ident)? $(#$mac:path)?),*)]
        $(#![$cont_attr:meta])*
        $(#[$our_cont_attr:meta])*
        $vis:vis enum $name:ident $(<$($life:lifetime),* $($generic:ident),*>)? {
            $(
                $(#[$our_variant_attr:meta])*
                $(#![$variant_attr:meta])*
                $variant:ident $(($ty:ty))? $(,)?
            )+
        }
    } => {
        // Re-emit the original enum.
        $(#[doc $($doc)*])*
        $(#[derive($($der_mac)?)])*
        $(#[$our_cont_attr])*
        $vis enum $name $(<$($life),* $($generic),*>)? {
            $(
                $(#[$our_variant_attr])*
                $variant $(($ty))?,
            )+
        }

        // Apply the helper macros.
        derive!(@apply [$($($mac,)*)?]
            |Cont_Attr|: $($cont_attr)*,
            |Name|: $name,
            $(
                |Lives|: $($life)*,
                |Generics|: $($generic)*,
            )?
            $(
                |Variant_Attr|: $($variant_attr)*,
                |Variant_Name|: $variant,
                |Variant_Type|: $($ty)?,
            )*
        );
    };

    // Internal rule: "TT Muncher" to apply a list of macros.
    // It takes the first macro from the list, applies it, and recurses on the rest.
    (@apply [$first:path, $($rest:path,)*] $($value:tt)*) => {
        $first!($($value)*);
        derive!(@apply [$($rest,)*] $($value)* );
    };

    // Base case: The list of macros is empty, so we do nothing.
    (@apply [] $($value:tt)*) => {};
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::{Value, json};

    use crate::MetaError;

    use super::*;

    #[test]
    fn any_field() {
        derive! {
            #[derive(#AnyField)]
            #[allow(dead_code)]
            struct S {
                #![anyfield("field", "field_id", "another_field")]
                field: String,
                #![serde(default)]
                actual_field: Option<usize>,
            }
        }

        let jstrs = [
            r#"{
                "field": "1",
                "actual_field": 6
            }"#,
            r#"{
                "actual_field": 200,
                "field_id": "2"
            }"#,
            r#"{
                "another_field": "3"
            }"#,
        ];

        for jstr in jstrs {
            serde_json::from_str::<S>(jstr).unwrap();
        }
    }

    #[test]
    fn de_adjacent() {
        derive! {
            #[derive(#DeserializeAdjacent, #SerializeAdjacent, PartialEq)]
            #[derive(Debug)]
            enum E {
                Tag(String),
                Tag1(i32),
                #![serde(rename(deserialize = "renamed_tag"))]
                Tag2(Vec<i32>),
                // We don't need the "unknown" tag... we make the error field seem like a content
                #![serde(rename(deserialize = "errors"))]
                ErrorContent(Vec<MetaError>) // they have a details field here
            }
        }

        #[derive(Debug)]
        struct Test {
            input: &'static str,
            want: E,
            output: Value,
        }
        let tests = [
            Test {
                input: r#"{
                    "type": "tag",
                    "tag": "value"
                }"#,
                want: E::Tag("value".to_owned()),
                output: json! {{
                    "type": "tag",
                    "tag": "value"
                }},
            },
            Test {
                input: r#"{
                    "type": "tag1",
                    "tag1": 300
                }"#,
                want: E::Tag1(300),
                output: json! {{
                    "type": "tag1",
                    "tag1": 300
                }},
            },
            Test {
                input: r#"{
                    "type": "any_tag",
                    "renamed_tag": [1, 2, 3]
                }"#,
                want: E::Tag2([1, 2, 3].into()),
                output: json! {{
                    "type": "tag2",
                    "tag2": [1, 2, 3]
                }},
            },
            Test {
                input: r#"{
                    "type": "unknown",                    
                    "errors": [
                      {
                        "code": 131051,
                        "details": "Message type is not currently supported",
                        "title": "Unsupported message type"
                      }
                    ]
                }"#,
                want: E::ErrorContent(
                    [MetaError {
                        code: 131051,
                        title: Some("Unsupported message type".to_owned()),
                        ..Default::default()
                    }]
                    .into(),
                ),
                output: json! {{
                    "type": "error_content",
                    "error_content": [
                      {
                        "code": 131051,
                        "title": "Unsupported message type"
                      }
                    ]
                }},
            },
        ];

        for test in tests {
            assert_eq!(serde_json::to_value(&test.want).unwrap(), test.output);
            assert_eq!(serde_json::from_str::<E>(test.input).unwrap(), test.want);
        }
    }
}
