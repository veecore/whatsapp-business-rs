#[macro_export]
#[doc(hidden)]
macro_rules! Endpoint {
    ($($node:tt)*) => {
        #[inline(always)]
        pub(crate) fn endpoint<'a>(&'a self, path: &'a str) -> $crate::client::Endpoint<'a, 2>
        {
            self.client.endpoint().join(&self.$($node)*).join(path)
        }
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! enum_traits {
    (|$target:ident| $($content:ident => $variant:ident),*) => {
        $(
            impl From<$content> for $target {
                fn from(value: $content) -> Self {
                    Self::$variant(value)
                }
            }

            impl PartialEq<$content> for $target {
                fn eq(&self, other: &$content) -> bool {
                    if let $target::$variant(inner) = self {
                        inner.eq(other)
                    } else {
                        false
                    }
                }
            }
        )*
    }
}

// better as fn
#[inline]
#[allow(clippy::needless_lifetimes)]
pub(crate) unsafe fn view_ref<'r, M, R>(m: &'r M) -> &'r R {
    // shift safety on caller
    &*(m as *const M as *const R)
}

#[macro_export]
#[doc(hidden)]
macro_rules! IntoFuture {
    (
        impl $(<$($lt:lifetime $(,)?)? $($gen:ident),*>)? $name:ident $(<$($lt2:lifetime $(,)?)? $($gen2:ident),*>)?
        $([where $($wheres:tt)*])?
        {
            $(#[$meta:meta])*
            pub async fn $func:ident ( $($args:tt)* ) -> $ret:ty $body:block
        }
    ) => {
        impl $(<$($lt,)? $($gen),*>)? $name $(<$($lt2,)? $($gen2),*>)?
        $(where $($wheres)*)?
        {
            $(#[$meta])*
            pub async fn $func($($args)*) -> $ret $body
        }

        #[cfg(feature = "nightly")]
        impl $(<$($lt,)? $($gen),*>)? ::std::future::IntoFuture for $name $(<$($lt2,)? $($gen2),*>)?
        $(where $($wheres)*)?
        {
            type Output = $ret;
            type IntoFuture = impl ::std::future::Future<Output = $ret>;

            fn into_future(self) -> Self::IntoFuture {
                self.$func()
            }
        }

        #[cfg(not(feature = "nightly"))]
        impl $(<$($lt,)? $($gen),*>)? ::std::future::IntoFuture for $name $(<$($lt2,)? $($gen2),*>)?
        $(where $($wheres)*)?
        {
            type Output = $ret;
            type IntoFuture = ::std::pin::Pin<Box<dyn ::std::future::Future<Output = $ret> + Send $($(+ $lt)?)?>>;

            fn into_future(self) -> Self::IntoFuture {
                Box::pin(self.$func())
            }
        }
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! SimpleStreamOutput {
    ($(#[$cont_attr:meta])* $name:ident => $stream:ty) => {
        /// This struct represents a pending API request.
        ///
        /// It provides methods to configure the request and then convert it into
        /// an asynchronous stream of results.
        ///
        /// The network request is performed when the generated stream is iterated over.
        $(#[$cont_attr])*
        pub struct $name {
            request: reqwest::RequestBuilder,
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
                self.request = self.request.bearer_auth(auth.to_value());
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
                    $crate::rest::stream_net_op(self.request)
                }
            }
        }
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! SimpleOutput {
    ($(#[$cont_attr:meta])* $name:ident $(<$life:lifetime>)? => $execute:ty) => {
        paste::paste! {
            $(#[$cont_attr])*
            /// This struct represents a pending API request.
            ///
            /// It provides methods to configure the request before executing it.
            ///
            /// It does not perform any network operations until it is `.await`ed
            /// (due to its `IntoFuture` implementation) or its `execute().await` method is called.
            #[must_use = "" $name " does nothing unless you `.await` or `.execute().await` it"]
            pub struct $name $(<$life>)? {
                request: reqwest::RequestBuilder,
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
                    self.request = self.request.bearer_auth(auth.to_value());
                    self
                }
            }

            $crate::IntoFuture! {
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
                    pub async fn execute(self) -> Result<$execute, $crate::error::Error> {
                        $crate::rest::fut_net_op(self.request).await
                    }
                }
            }
        }
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! to_value {
    ($($name:ty)*) => {
        $(
            impl $crate::ToValue<'_, $name> for $name {
                #[inline]
                fn to_value(self) -> std::borrow::Cow<'static, $name> {
                    std::borrow::Cow::Owned(self)
                }
            }

            impl<'a> $crate::ToValue<'a, $name> for &'a $name {
                #[inline]
                fn to_value(self) -> std::borrow::Cow<'a, $name> {
                    std::borrow::Cow::Borrowed(self)
                }
            }
        )*
    }
}

#[macro_export]
#[doc(hidden)]
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
            #[inline]
            pub fn new($field: impl Into<String>) -> Self {
                Self {$field: $field.into()}
            }

            #[inline]
            pub fn $field(&self) -> &str {
                &self.$field
            }
        }

        impl<T: Into<String>> From<T> for $name {
            fn from(value: T) -> Self {
                Self::new(value)
            }
        }

        impl serde::Serialize for $name {
            #[inline]
            fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
                ser.serialize_str(&self.$field)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            #[inline]
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let id: String = <String as serde::Deserialize>::deserialize(d)?;
                Ok(Self {$field: id})
            }
        }

        $crate::to_value! {
            $name
        }
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! flow {
    {
        $(#[$cont_attr:meta])*
        $vis:vis struct $name:ident <$($life:lifetime,)* $($generic:ident),* $(,)?> {
            $(
                $(#[$field_attr:meta])*
                $field_vis:vis $field:ident: $ty:ty,
            )+
        }

        $($step:tt)*
    } => {
        $(#[$cont_attr])*
        $vis struct $name <$($life,)* $($generic = (),)*> {
            $(
                $(#[$field_attr])*
                $field_vis $field: $ty,
            )+
        }


        $crate::flow! {
            |Name|: $name,
            |Lives|: $($life)*,
            |Params|: > ,
            |Fields|: > > [$($field,)*],
            |Steps|: $($step)*
        }
    };
    {
        |Name|: $name:ident,
        |Lives|: $($life:lifetime)*,
        |Params|: $($prev:ty)* > $($just_now:ty)?,
        |Fields|: $($prev_field:ident)* > $($just_now_field:ident)? > [$to_be_added:ident, $($other_field:ident,)*],
        |Steps|: $(#[$func_attr:meta])* $(#![$optional:ident])? $vis:vis $async:ident fn $next:ident$(<$f_life:lifetime>)?($($arg:tt)*) -> $step_ty:ty $body:block $($other:tt)*
    } => {
        impl<$($life),*> $name <$($life,)* $($prev,)* $($just_now)?> {
            $(#[$func_attr])*
            // FIXME: self module
            $vis $async fn $next$(<$f_life>)?($($arg)*) -> Result<$name <$($life,)* $($prev,)* $($just_now,)? $step_ty>, Error> {
                let (to_be_added, state) = $body;
                Ok($name {
                    $(
                        $prev_field: state.$prev_field,
                    )*
                    $(
                        $just_now_field: state.$just_now_field,
                    )?
                    $to_be_added: to_be_added,
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

            // More like FlowStepRuined rn... the following step and steps should anticipate this by accepting
            // any T (which may scatter the steps) or a specific one that indicates skip
            // (We introduced FlowStepSkipped for this)... but this just snowballs except maybe we use traits.
            // Since only share_credit_line needs this for now and it's the last... we concede for now.
            // We won't use Option to avoid an unneccessary runtime check
            $crate::flow! {
                @optional
                $($optional)?
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
        $crate::flow! {
            |Name|: $name,
            |Lives|: $($life)*,
            |Params|:  $($prev)* $($just_now)? > $step_ty,
            |Fields|: $($prev_field)* $($just_now_field)? > $to_be_added > [$($other_field,)*],
            |Steps|: $($other)*
        }
    };
    {
        |Name|: $name:ident,
        |Lives|: $($life:lifetime)*,
        |Params|: $($prev:ty)* > $($just_now:ty)?,
        |Fields|: $($prev_field:ident)* > $($just_now_field:ident)? > [$($residue:ident,)*],
        |Steps|:
    } => {};
    {@optional optional $imp:item} => {
        $imp
    };
    {@optional $imp:item} => {}
    // {
    //     $($whatever:tt)*
    // } => {}
}

// Only for enum
#[macro_export]
#[doc(hidden)]
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
        paste::paste!(
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
                            Self::$variant => stringify!([<$variant:snake>]),
                        )*
                    }
                }
            }
        );
    }
}

// Can't be used with any other macro... currently very limited
// because we want to avoid recursive filtering and other things
//
// We currently only need it on only one field in each... so let's
// be lazy.
#[macro_export]
#[doc(hidden)]
macro_rules! AnyField {
    {
        |Cont_Attr|: $($cont_attr:meta)*,
        |Name|: $name:ident,
        $(
            |Lives|: $($life:lifetime)*,
            |Generics|: $($generic:ident)*,
        )?
        |Field_Vis|: $any_field_vis:vis,
        |Field_Attr|: anyfield($($any_field:literal),*) $($any_field_attr:meta)*,
        |Field_Name|: $any_field_ident:ident,
        |Field_Type|: $any_field_ty:ty,
        $(
            |Field_Vis|: $field_vis:vis,
            |Field_Attr|: $($field_attr:meta)*,
            |Field_Name|: $field:ident,
            |Field_Type|: $ty:ty,
        )*
    } => {
        paste::paste! {
            impl<'a> serde::Deserialize<'a> for $name {
                #[inline]
                fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                    where
                        D: serde::Deserializer<'a> {
                    let helper = [<AnyField $name>]::deserialize(deserializer)?;
                    Ok(unsafe { std::mem::transmute::<[<AnyField $name>], $name>(helper) })
                }
            }

            #[derive(serde::Deserialize)]
            #[allow(dead_code)]
            pub(crate) struct [<AnyField $name>]$(<$($life),* $($generic),*>)? {
                #[serde(flatten)]
                $(#[$any_field_attr])*
                $any_field_ident: [<AnyField $name Ty>],
                $(
                    $(#[$field_attr])*
                    $field: $ty,
                )*
            }

            // We could use enum but we want to make the helper the same size
            // as the main so we could transmute... Hmm... not worth it
            #[allow(dead_code)]
            struct [<AnyField $name Ty>]($any_field_ty);

            impl<'a> serde::Deserialize<'a> for [<AnyField $name Ty>] {
                fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                    where
                        D: serde::Deserializer<'a> {
                    deserializer.deserialize_map([<AnyField $name Ty Visitor>])
                }
            }

            struct [<AnyField $name Ty Visitor>];

            impl<'de> serde::de::Visitor<'de> for [<AnyField $name Ty Visitor>]  {
                type Value = [<AnyField $name Ty>];

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    write!(formatter, "struct {} with any of the fields: {:#?}.",
                           stringify!([<AnyField $name Ty>]),
                           [$($any_field),*])
                }

                fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                    where
                        A: serde::de::MapAccess<'de>, {
                    // FIXME: just match k as raw &str?
                    #[derive(serde::Deserialize)]
                    #[allow(non_camel_case_types)]
                    enum Field {
                        $(
                            [<$any_field>],
                        )*
                    }

                    let (_, v): (Field, $any_field_ty) = map.next_entry()?.ok_or_else(|| {
                        <A::Error as serde::de::Error>::custom(format!("missing one of the fields: {:#?}", [$($any_field),*]))
                    })?;
                    Ok([<AnyField $name Ty>](v))
                }
            }
        }
    };
}

#[macro_export]
#[doc(hidden)]
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
                        V: BuilderInto<$ty>
                    {
                        self.item.$field = $field.builder_into();
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

#[macro_export]
#[doc(hidden)]
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

#[macro_export]
#[doc(hidden)]
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
            $crate::derive! {
                #[derive(#$crate::FieldsTrait, Serialize, PartialEq, Eq, Hash, Clone, Copy, Debug)]
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

#[macro_export]
#[doc(hidden)]
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
        $crate::ContentTraits! {
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
                type Request = [<$name Request>];

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
                fn with_auth(self, auth: &$crate::client::Auth) -> Self {
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

            $crate::derive! {
                #[derive(Debug, #$crate::SerializeAdjacent)]
                // I think the implicit puppetteering (transumting) in impl_adjacent_serde made the
                // linter think we're never reading or constructing the variants
                #[allow(dead_code)]
                pub(crate) enum [<$name Request>] {
                    $(
                        $media_type(<$media_ty as $crate::rest::IntoMessageRequest>::Request),
                    )+

                    $(
                        $(#![$variant_attr])*
                        $variant $((<$data as $crate::rest::IntoMessageRequest>::Request))?,
                    )*
                }
            }

            impl $crate::rest::FromResponse for $name {
                type Response<'a> = [<$name Response>]<'a>;

                // TODO: Handle auto-download
                #[inline]
                fn from_response<'a>(response: Self::Response<'a>) -> Result<Self, $crate::error::ServiceErrorKind> {
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
            $crate::derive! {
                #[derive(Debug, #$crate::DeserializeAdjacent)]
                #![serde(bound(deserialize = "'de: 'a, 'a: 'de"))]
                #[allow(dead_code)]
                pub(crate) enum [<$name Response>]<'a> {
                    $(
                        $media_type(<$media_ty as $crate::rest::FromResponse>::Response<'a>),
                    )+

                    $(
                        $(#![$variant_attr])*
                        $variant $((<$data as $crate::rest::FromResponse>::Response<'a>))?,
                    )*
                }
            }

            impl From<$media_ty> for $name {
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

            $crate::ContentTraits! {
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

                impl<T: Into<String> + Send> IntoDraft for T {
                    #[inline]
                    fn into_draft(self) -> Draft {
                        Draft::text(self)
                    }
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

/// Helper for adjacent enum serialization
#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct AdjacentHelper<'a, T> {
    // We don't need it
    #[serde(default)]
    pub(crate) r#type: Cow<'a, str>,
    #[serde(flatten)]
    pub(crate) inner: T,
}

/// Macro for adjacent enum serialization/deserialization
///
/// Use only when enum name matches the intended serialization
/// tag
//
// FIXME: We should try to minimize how using this could go wrong
#[macro_export]
#[doc(hidden)]
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
            #[derive(serde::Deserialize)]
            $(#[$cont_attr])*
            #[serde(rename_all = "snake_case")]
            #[doc(hidden)]
            #[allow(dead_code)]
            pub(crate) enum [<$name DeAdjacentHelper>] $(<$($life),* $($generic),*>)? {
                $(
                    $(#[$variant_attr])*
                    $variant $(($ty))?,
                )+
            }

            impl<'de, $($($life),* $($generic),*)?> Deserialize<'de> for $name $(<$($life),* $($generic),*>)? {
                #[inline]
                fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                    let helper = $crate::rest::AdjacentHelper::<[<$name DeAdjacentHelper>]>::deserialize(d)?;
                    Ok(unsafe { std::mem::transmute::<[<$name DeAdjacentHelper>], $name>(helper.inner) })
                }
            }
        );
    }
}

#[macro_export]
#[doc(hidden)]
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
            #[derive(serde::Serialize)]
            $(#[$cont_attr])*
            #[serde(rename_all = "snake_case")]
            #[doc(hidden)]
            #[allow(dead_code)]
            pub(crate) enum [<$name SerAdjacentHelper>] $(<$($life),* $($generic),*>)? {
                $(
                    $(#[$variant_attr])*
                    $variant$(($ty))?,
                )+
            }

            impl$(<$($life),* $($generic),*>)? Serialize for $name $(<$($life),* $($generic),*>)? {
                #[inline]
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {

                    let r#type = match self {
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
                        r#type: std::borrow::Cow::Borrowed(r#type),
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

/// Macro helper for deriving request/response conversions
#[macro_export]
#[doc(hidden)]
macro_rules! FromResponse {
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
            impl<$($($life,)* $($generic,)*)?> $crate::rest::FromResponse for $name $(<$($life),* $($generic),*>)?
            where
                $($ty: $crate::rest::FromResponse,)*
            {
                type Response<'a> = [<$name Response>]<'a, $($($generic),*)?>;

                #[inline]
                fn from_response<'a>(
                    response: Self::Response<'a>,
                ) -> Result<Self, $crate::error::ServiceErrorKind> {
                    Ok(Self {
                        $(
                            $field: <$ty as $crate::rest::FromResponse>::from_response(response.$field)?,
                        )+
                    })
                }
            }

            #[derive(::serde::Deserialize)]
            $(#[$cont_attr])*
            #[serde(bound(deserialize = "'de: 'a, 'a: 'de"))]
            #[doc(hidden)]
            pub(crate) struct [<$name Response>]<'a, $($($life),* $($generic),*)?>
            where
                $($ty: $crate::rest::FromResponse,)*
            {
                $(
                    $(#[$field_attr])*
                    $field: <$ty as $crate::rest::FromResponse>::Response<'a>,
                )+
            }

            // Manually implementing Debug to avoid unnecessary T: Debug bound created by the Debug macro
            impl<'a, $($($life),* $($generic),*)?> std::fmt::Debug for [<$name Response>]<'a, $($($life),* $($generic),*)?>
            where
                $($ty: $crate::rest::FromResponse,)*
            {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    let mut debug_struct = f.debug_struct(stringify!([<$name Response>]));
                    $(
                        debug_struct.field(stringify!($field), &self.$field);
                    )+
                    debug_struct.finish()
                }
            }
        );
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
        paste::paste!(
            impl<$($($life,)* $($generic,)*)?> $crate::rest::FromResponse for $name $(<$($life),* $($generic),*>)?
            where
                $($ty: $crate::rest::FromResponse,)*
            {
                type Response<'a> = [<$name Response>]<'a, $($($generic),*)?>;

                #[inline]
                fn from_response<'a>(
                    response: Self::Response<'a>,
                ) -> Result<Self, $crate::error::ServiceErrorKind> {
                    Ok(match response {
                        $(
                            Self::Response::$variant([<$ty:snake>]) =>
                            Self::$variant(<$ty as $crate::rest::FromResponse>::from_response([<$ty:snake>])?),
                        )*
                    })
                }
            }

            #[derive(Debug, ::serde::Deserialize)]
            $(#[$cont_attr])*
            #[serde(bound(deserialize = "'de: 'a, 'a: 'de"))]
            #[doc(hidden)]
            pub(crate) enum [<$name Response>]<'a, $($($life),* $($generic),*)?>
            where
                $($ty: $crate::rest::FromResponse,)*
            {
                $(
                    $(#[$variant_attr])*
                    $variant(<$ty as $crate::rest::FromResponse>::Response<'a>),
                )+
            }
        );
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! IntoRequest {
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
            impl<$($($life,)* $($generic,)*)?> $crate::rest::IntoMessageRequest for $name $(<$($life),* $($generic),*>)?
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
            {
                type Request = [<$name Request>]<$($($generic),*)?>;

                type Output = [<$name Output>]<$($($generic),*)?>;

                #[inline]
                fn into_request<'i, 't>(
                    self,
                    manager: &$crate::client::MessageManager<'i>,
                    to: &'t $crate::IdentityRef,
                ) -> Self::Output {
                    Self::Output{
                        $(
                            $field: self.$field.into_request(manager, to),
                        )+
                    }
                }
            }

            #[doc(hidden)]
            pub(crate) struct [<$name Output>]<$($($life),* $($generic),*)?>
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
            {
                $(
                    $field: <$ty as $crate::rest::IntoMessageRequest>::Output,
                )*
            }

            impl $crate::rest::IntoMessageRequestOutput for [<$name Output>] {
                type Request = [<$name Request>]<$($($life),* $($generic),*)?>;

                #[inline]
                fn with_auth(self, auth: &$crate::client::Auth) -> Self {
                    Self {
                        $(
                            $field: self.$field.with_auth(auth),
                        )*
                    }
                }

                #[inline]
                async fn execute(self) -> Result<Self::Request, $crate::error::Error> {
                    Ok(Self::Request {
                        $(
                            $field: self.$field.execute().await?,
                        )*
                    })
                }
            }

            #[derive(Debug, ::serde::Serialize)]
            $(#[$cont_attr])*
            #[doc(hidden)]
            pub(crate) struct [<$name Request>]<$($($life),* $($generic),*)?>
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
                // Bound life
            {
                $(
                    $(#[$field_attr])*
                    $field: <$ty as $crate::rest::IntoMessageRequest>::Request,
                )+
            }
        );
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
        paste::paste!(
            impl<$($($life,)* $($generic,)*)?> $crate::rest::IntoMessageRequest for $name $(<$($life),* $($generic),*>)?
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
            {
                type Request = [<$name Request>]<$($($generic),*)?>;

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

            #[doc(hidden)]
            pub(crate) enum [<$name Output>]<$($($life),* $($generic),*)?>
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
            {
                $(
                    $variant(<$ty as $crate::rest::IntoMessageRequest>::Output),
                )*
            }

            impl $crate::rest::IntoMessageRequestOutput for [<$name Output>] {
                type Request = [<$name Request>]<$($($life),* $($generic),*)?>;

                #[inline]
                fn with_auth(self, auth: &$crate::client::Auth) -> Self {
                    match self {
                        $(
                            Self::$variant(inner) =>
                            Self::$variant(inner.with_auth(auth)),
                        )*
                    }
                }

                #[inline]
                async fn execute(self) -> Result<Self::Request, $crate::error::Error> {
                    Ok(match self {
                        $(
                            Self::$variant(inner) =>
                            Self::Request::$variant(inner.execute().await?),
                        )*
                    })
                }
            }

            #[derive(Debug, ::serde::Serialize)]
            $(#[$cont_attr])*
            #[doc(hidden)]
            pub(crate) enum [<$name Request>]<$($($life),* $($generic),*)?>
            where
                $($ty: $crate::rest::IntoMessageRequest,)*
            {
                $(
                    $(#[$variant_attr])*
                    $variant(<$ty as $crate::rest::IntoMessageRequest>::Request),
                )+
            }

        );
    }
}

// FIXME: The current setup works because every macro needs the serde attr
// and none needs a custom one.

/// This is the “entry” macro for the other "derive" macros
///
/// NOTE: Rep attributes to pass on as "#!\[attr\]"
#[macro_export]
#[doc(hidden)]
macro_rules! derive {
    // Struct
    {
        // FIXME
        $(#[doc $($doc:tt)*])*
        #[derive($($($der_mac:ident)? $(#$mac:path)?),*)]
        $(#![$($cont_attr:tt)*])*
        $(#[$($our_cont_attr:tt)*])*
        $vis:vis struct $name:ident $(<$($life:lifetime),* $($generic:ident),*>)? {
            $(
                $(#[$($our_field_attr:tt)*])*
                $(#![$($field_attr:tt)*])*
                $field_vis:vis $field:ident: $ty:ty,
            )+
        }
    } => {
        // Declare (minus our special `#IntoRequest` etc.)
        $(#[doc $($doc)*])*
        $(#[derive($($der_mac)?)])*
        $(#[$($our_cont_attr)*])*
        $vis struct $name $(<$($life),* $($generic),*>)? {
            $(
                $(#[$($our_field_attr)*])*
                $field_vis $field: $ty,
            )+
        }

       $crate::derive!(@apply [$($($mac,)*)?]
                |Cont_Attr|: $($($cont_attr)*)*,
                |Name|: $name,
                $(
                    |Lives|: $($life)*,
                    |Generics|: $($generic)*,
                )?
                $(
                    |Field_Vis|: $field_vis,
                    |Field_Attr|: $($($field_attr)*)*,
                    |Field_Name|: $field,
                    |Field_Type|: $ty,
                )*);
    };
    // Enum
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
        // Declare (minus our special `#IntoRequest` etc.)
        $(#[doc $($doc)*])*
        $(#[derive($($der_mac)?)])*
        $(#[$our_cont_attr])*
        $vis enum $name $(<$($life),* $($generic),*>)? {
            $(
                $(#[$our_variant_attr])*
                $variant $(($ty))?,
            )+
        }

        $crate::derive!(@apply [$($($mac,)*)?]
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
                )*);
    };
    (@apply [$first:path, $($rest:path,)*] $($value:tt)*) => {
        $first!($($value)*);
        $crate::derive!(@apply [$($rest,)*] $($value)* );
    };
    (@apply [] $($value:tt)*) => {};
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::{json, Value};

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
            }
        }

        let jstrs = [
            r#"{
                "field": "1"
            }"#,
            r#"{
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
