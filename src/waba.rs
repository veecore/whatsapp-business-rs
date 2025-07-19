//! WhatsApp Business Account (WABA) Management
//!
//! This module provides functionality for working with a WhatsApp Business Account (WABA).
//!
//! Access this functionality through [`Client::waba`].
//!
//! Features include:
//! - Listing catalogs and phone numbers associated with a WABA
//! - Viewing which apps are subscribed to the WABA
//! - Running a type-safe flow to register and verify new phone numbers
//!
//! # Example – Listing Catalogs
//! ```rust,no_run
//! use whatsapp_business_rs::{client::Client, Waba, waba::CatalogMetadataField};
//! use futures::TryStreamExt as _;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let business = Waba::new("WABA_ID");
//! let client = Client::new("ACCESS_TOKEN").await?;
//!
//! let mut catalogs = client
//!     .waba(business)
//!     .list_catalogs()
//!     .metadata([CatalogMetadataField::Name, CatalogMetadataField::Vertical].into())
//!     .into_stream();
//!
//! while let Some(catalog) = catalogs.try_next().await? {
//!      println!("{:?}", catalog);
//! }
//! # Ok(()) }
//! ```
//!
//! # Example – Registering a Phone Number
//! ```rust,no_run
//! use whatsapp_business_rs::{client::Client, Waba, waba::{UnverifiedPhoneNumber, VerificationMethod}};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let phone = UnverifiedPhoneNumber {
//!      country_code: "+1".into(),
//!      phone_number: "123456789".into(),
//!      verified_name: "My Business".into(),
//! };
//!
//! let client = Client::new("ACCESS_TOKEN").await?;
//! let result = client.waba("WABA_ID")
//!      .start_number_registration(&phone)
//!      .register_phone_number().await?
//!      .request_verification_code(VerificationMethod::Sms, "en").await?
//!      .verify_phone_number("123456").await?
//!      .set_pin("000000").await?
//!      .finalize();
//!
//! println!("Registered ID: {}", result.phone_number_id);
//! # Ok(()) }
//! ```
//!
//! See also:
//! - [`Catalog`] — represents a product catalog
//! - [`PhoneNumber`] — represents a WABA-linked number
//! - [`NumberRegistrationFlow`] — guided, step-by-step flow for adding numbers

use std::{borrow::Cow, marker::PhantomData};

use serde::{Deserialize, Serialize};

use crate::{
    client::{Client, Endpoint},
    derive,
    error::Error,
    flow, fut_net_op,
    rest::client::RegisterPhoneRequest,
    view_ref, CatalogRef, Endpoint, Fields, SimpleStreamOutput, ToValue, Waba,
};

///
/// Access this via [`Client::waba()`]. This manager provides methods to:
/// - List product catalogs owned by the business
/// - List phone numbers registered to the WABA
/// - Inspect subscribed apps
/// - Initiate phone number registration flows
///
/// For most use cases, you won't construct this directly; instead, use:
/// ```rust,no_run
/// # use whatsapp_business_rs::{client::Client, Waba};
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let client = Client::new("ACCESS_TOKEN").await?;
/// let business = Waba::new("1234567890");
/// let manager = client.waba(business);
/// # Ok(()) }
/// ```
///
/// To register a new phone number:
/// ```rust,no_run
/// use whatsapp_business_rs::waba::{UnverifiedPhoneNumber, VerificationMethod};
///
/// # async fn example(manager: whatsapp_business_rs::waba::WabaManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
/// let phone = UnverifiedPhoneNumber {
///      country_code: "1".into(),
///      phone_number: "6123456789".into(),
///      verified_name: "My Business".into(),
/// };
///
/// let flow = manager.start_number_registration(&phone)
///      .register_phone_number().await?
///      .request_verification_code(VerificationMethod::Sms, "en").await?
///      .verify_phone_number("123456").await?
///      .set_pin("000000").await?
///      .finalize();
///
/// println!("Registered number: {}", flow.phone_number_id);
/// # Ok(()) }
/// ```
#[derive(Debug)]
pub struct WabaManager<'i> {
    client: Client,
    of: Cow<'i, Waba>,
}

impl<'i> WabaManager<'i> {
    /// Create a new message manager
    pub(crate) fn new(of: Cow<'i, Waba>, client: &Client) -> Self {
        Self {
            client: client.clone(),
            of,
        }
    }

    /// Prepares a request to list all product catalogs associated with the business account.
    ///
    /// This method returns a [`ListCatalog`] builder. To retrieve catalogs,
    /// call `.into_stream()` on the builder and then iterate over the stream.
    /// You can optionally select which metadata fields to include using the `metadata()` method.
    ///
    /// # Returns
    /// A [`ListCatalog`] builder, which can be configured and then converted
    /// into an asynchronous stream of [`Catalog`] items.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::waba::CatalogMetadataField;
    /// use futures::TryStreamExt as _;
    ///
    /// # async fn example(manager: whatsapp_business_rs::waba::WabaManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
    /// manager
    ///     .list_catalogs()
    ///     .metadata([CatalogMetadataField::Name, CatalogMetadataField::Vertical].into())
    ///     .into_stream()
    ///     .try_for_each(|catalog| async move {
    ///         println!("{:?}", catalog);
    ///         Ok(())
    ///     })
    ///     .await?;
    /// # Ok(())}
    /// ```
    ///
    /// [`Catalog`]: crate::Catalog
    /// [`ListCatalog`]: crate::waba::ListCatalog
    pub fn list_catalogs(&self) -> ListCatalog {
        let url = self.endpoint("product_catalogs");
        let request = self.client.get(url);
        ListCatalog { request }
    }

    /// Prepares a request to list all phone numbers attached to this WhatsApp Business Account (WABA).
    ///
    /// This method returns a [`ListNumber`] builder. To retrieve phone numbers,
    /// call `.into_stream()` on the builder and then iterate over the stream.
    /// You can select which metadata fields to include per phone number using the `metadata()` method.
    ///
    /// # Returns
    /// A [`ListNumber`] builder, which can be configured and then converted
    /// into an asynchronous stream of [`PhoneNumber`] items.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::waba::PhoneNumberMetadataField;
    /// use futures::TryStreamExt as _;
    ///
    /// # async fn example(manager: whatsapp_business_rs::waba::WabaManager<'_>) -> Result<(), Box<dyn std::error::Error>> {    
    /// manager.list_phone_numbers()
    ///      .into_stream()
    ///      .try_for_each(|number| async move {
    ///          println!("{:?}", number);
    ///          Ok(())
    ///      })
    ///      .await?;
    /// # Ok(())}
    /// ```
    ///
    /// [`PhoneNumber`]: crate::PhoneNumber
    /// [`ListNumber`]: crate::waba::ListNumber
    pub fn list_phone_numbers(&self) -> ListNumber {
        let url = self.endpoint("phone_numbers");
        let request = self.client.get(url);

        ListNumber { request }
    }

    /// Prepares a request to list all apps this WABA is currently subscribed to.
    ///
    /// This method returns a [`ListApp`] builder. To retrieve subscribed apps,
    /// call `.into_stream()` on the builder and then iterate over the stream.
    /// Unlike other listing methods, this does not support sparse field selection —
    /// all standard fields are returned.
    ///
    /// # Returns
    /// A [`ListApp`] builder, which can be converted into an asynchronous stream
    /// of [`AppInfo`] items.
    ///
    /// # Example
    /// ```rust
    /// use futures::TryStreamExt as _;
    ///
    /// # async fn example(manager: whatsapp_business_rs::waba::WabaManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
    /// manager.list_subcribed_apps()
    ///      .into_stream()
    ///      .try_for_each(|app| async move {
    ///          println!("{:?}", app);
    ///          Ok(())
    ///      })
    ///      .await?;
    /// # Ok(())}
    /// ```
    ///
    /// [`AppInfo`]: crate::AppInfo
    /// [`ListApp`]: crate::waba::ListApp
    pub fn list_subcribed_apps(&self) -> ListApp {
        let url = self.endpoint("subscribed_apps");
        let request = self.client.get(url);

        ListApp { request }
    }

    /// Initiates the registration process for a new phone number.
    ///
    /// This method starts a guided, type-safe flow for registering and verifying
    /// a new phone number with the WhatsApp Business Account. The process involves
    /// multiple steps: registering the number, requesting a verification code,
    /// verifying the code, and finally completing the registration.
    ///
    /// # Parameters
    /// - `phone`: An [`UnverifiedPhoneNumber`] struct containing the country code,
    ///   phone number, and desired verified name for the new number.
    ///
    /// # Returns
    /// A [`NumberRegistrationFlow`] builder, which guides you through the
    /// subsequent steps of phone number registration.
    ///
    /// # Example
    /// ```rust,no_run
    /// use whatsapp_business_rs::waba::{UnverifiedPhoneNumber, VerificationMethod};
    ///
    /// # async fn example(manager: whatsapp_business_rs::waba::WabaManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
    /// let phone = UnverifiedPhoneNumber {
    ///     country_code: "+1".into(),
    ///     phone_number: "123456789".into(),
    ///     verified_name: "My Business Name".into(),
    /// };
    ///
    /// let registration_result = manager.start_number_registration(&phone)
    ///     .register_phone_number().await?
    ///     .request_verification_code(VerificationMethod::Sms, "en").await?
    ///     .verify_phone_number("123456").await?
    ///     .set_pin("000000").await?
    ///     .finalize();
    ///
    /// println!("Successfully registered phone number with ID: {}", registration_result.phone_number_id);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`UnverifiedPhoneNumber`]: crate::waba::UnverifiedPhoneNumber
    /// [`NumberRegistrationFlow`]: crate::waba::NumberRegistrationFlow
    pub fn start_number_registration<'a, 'b>(
        &'a self,
        phone_number: &'b UnverifiedPhoneNumber,
    ) -> NumberRegistrationFlow<'a, 'b> {
        NumberRegistrationFlow::new(self, phone_number)
    }

    Endpoint! {of.account_id}
}

SimpleStreamOutput! {
    ListCatalog => Catalog
}

impl ListCatalog {
    /// Specifies which metadata fields to include for each catalog in the response.
    ///
    /// If not called, a default set of fields will be returned.
    ///
    /// # Parameters
    /// - `fields`: A [`Fields`] struct containing the desired [`CatalogMetadataField`]s.
    ///
    /// # Returns
    /// The updated builder instance.
    ///
    /// [`Fields`]: crate::Fields
    /// [`CatalogMetadataField`]: crate::waba::CatalogMetadataField
    pub fn metadata(mut self, metadata: Fields<CatalogMetadataField>) -> Self {
        self.request = metadata.into_request(self.request, ["id"]);
        self
    }
}

SimpleStreamOutput! {
    ListNumber => PhoneNumber
}

impl ListNumber {
    /// Specifies which metadata fields to include for each phone number in the response.
    ///
    /// If not called, a default set of fields will be returned.
    ///
    /// # Parameters
    /// - `fields`: A [`Fields`] struct containing the desired [`PhoneNumberMetadataField`]s.
    ///
    /// # Returns
    /// The updated builder instance.
    ///
    /// [`Fields`]: crate::Fields
    /// [`PhoneNumberMetadataField`]: crate::waba::PhoneNumberMetadataField
    pub fn metadata(mut self, metadata: Fields<PhoneNumberMetadataField>) -> Self {
        self.request = metadata.into_request(self.request, ["id"]);
        self
    }
}

SimpleStreamOutput! {
    ListApp => AppInfo
}

#[derive(Deserialize, Clone, Debug)]
#[non_exhaustive]
pub struct AppInfo {
    /// App ID
    pub id: String,
    /// Human-readable name of the app (optional).
    #[serde(default)]
    pub name: Option<String>,
    /// Optional link to the app in Meta's platform (usually internal).
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Deserialize, PartialEq, Clone, Debug)]
#[repr(C)]
pub struct Catalog {
    pub id: String,
    #[serde(flatten)]
    pub metadata: CatalogMetadata,
}

derive! {
    #[derive(#crate::Fields, Deserialize, PartialEq, Clone, Debug, Default)]
    #[non_exhaustive]
    pub struct CatalogMetadata {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub product_count: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub feed_count: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub vertical: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub is_local_catalog: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub is_catalog_segment: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub default_image_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub fallback_image_url: Option<String>,
    }
}

impl Catalog {
    pub fn as_ref(&self) -> CatalogRef {
        CatalogRef {
            id: self.id.clone(),
        }
    }
}

/// A phone number that hasn't yet been registered with WhatsApp
///
/// Use this when initiating a new registration via [`WabaManager::start_number_registration()`].
/// The number must have already been created in Meta Business Manager.
///
/// - `country_code`: The country calling code (e.g., `"44"` for UK)
/// - `phone_number`: The actual number, without or with the country code
/// - `verified_name`: The display name for the number, as submitted during verification
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::waba::UnverifiedPhoneNumber;
///
/// let number = UnverifiedPhoneNumber {
///     country_code: "44".into(),
///     phone_number: "0123456789".into(),
///     verified_name: "My Biz".into(),
/// };
/// ```
#[derive(Serialize, Debug)]
pub struct UnverifiedPhoneNumber {
    #[serde(rename = "cc")]
    pub country_code: String,
    /// The phone number, with or without the country calling code.
    pub phone_number: String,
    pub verified_name: String,
}

/// Represents the method used for phone number verification.
///
/// This enum is used to specify how a verification code should be sent
/// to the phone number during the registration process.
#[derive(Debug, Clone, Copy)]
pub enum VerificationMethod {
    /// Send the verification code via an SMS message.
    Sms,
    /// Send the verification code via a voice call.
    Voice,
}

impl VerificationMethod {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Sms => "SMS",
            Self::Voice => "VOICE",
        }
    }
}

/// A registered WhatsApp Business phone number
///
/// This struct contains the unique ID of the number as well as
/// various metadata fields such as its display name, verification status,
/// certificate status, and webhook configuration.
///
/// Retrieved via [`WabaManager::list_phone_numbers()`].
///
/// # Example
/// ```rust,no_run
/// use whatsapp_business_rs::waba::PhoneNumberMetadataField;
/// use futures::TryStreamExt as _;
///
/// # async fn example(manager: whatsapp_business_rs::waba::WabaManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
/// manager.list_phone_numbers()
///     .into_stream()
///     .try_for_each(|number| async move {
///         println!("ID: {}, Display: {:?}", number.id, number.metadata.display_phone_number);
///         Ok(())
///     })
///     .await?;
/// # Ok(())}
/// ```
#[derive(Deserialize, Debug)]
pub struct PhoneNumber {
    pub id: String,
    #[serde(flatten)]
    pub metadata: PhoneNumberMetadata,
}

derive! {
    #[derive(#crate::Fields, Deserialize, Debug, Default)]
    #[non_exhaustive]
    pub struct PhoneNumberMetadata {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub display_phone_number: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub verified_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub code_verification_status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub quality_rating: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub last_onboarded_time: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub certificate: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub new_certificate: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub name_status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub new_name_status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub platform_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub webhook_configuration: Option<WebhookConfig>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub throughput: Option<Throughput>,
    }
}

/// Webhook configuration
#[derive(Deserialize, Debug)]
pub struct WebhookConfig {
    #[serde(rename = "application")]
    pub webhook_url: String,
}

/// Throughput level
#[derive(Debug, Deserialize, Serialize)]
pub struct Throughput {
    pub level: String,
}

impl<'a, 'b> NumberRegistrationFlow<'a, 'b> {
    pub(crate) fn new(manager: &'a WabaManager, number: &'b UnverifiedPhoneNumber) -> Self {
        Self {
            manager,
            number,
            phone_number_id: (),
            pending_verify: (),
            verify: (),
            _complete: (),
        }
    }
}

impl<'a, 'b, S2, S3> NumberRegistrationFlow<'a, 'b, String, S2, S3> {
    #[inline(always)]
    pub(crate) fn endpoint<'s>(&'s self, path: &'a str) -> Endpoint<'s, 2> {
        self.manager.client.a_node(&self.phone_number_id).join(path)
    }
}

flow! {
    /// A step-based flow for registering a WhatsApp Business phone number.
    ///
    /// This flow is enforced in steps, with type-level guarantees to prevent skipping.
    ///
    /// Each step is enforced through type-level state parameters. Only valid step sequences can compile.
    ///
    /// ⚠️ This flow is not considered stable — steps or requirements may change.
    #[must_use = "NumberRegistrationFlow's steps need to be completed"]
    pub struct NumberRegistrationFlow<'a, 'b, S1, S2, S3, S4> {
        phone_number_id: S1,
        pending_verify: S2,
        verify: S3,
        _complete: S4,

        manager: &'a WabaManager<'a>,
        number: &'b UnverifiedPhoneNumber,
    }

    /// Step 1: Creates a phone number registration request.
    ///
    /// # Returns
    /// The next flow step, containing the phone number ID.
    pub async fn register_phone_number(
        self,
    ) -> String {
        let client = &self.manager.client;

        let url = self.manager.endpoint("phone_numbers");

        let request = client
            .post(url)
            .json(&self.number);

        let res: RegisterResponse = fut_net_op(request).await?;

        (res.id, self)
    }

    /// Step 2: Sends a request to Meta to begin verification.
    ///
    /// This sends a verification code via the chosen method (SMS or voice).
    ///
    /// # Arguments
    /// - `method`: How the code should be delivered.
    /// - `language`: Language of the code message.
    ///
    /// # Returns
    /// Next flow step, marking the number as awaiting verification.
    pub async fn request_verification_code(
        self,
        method: VerificationMethod,
        language: &str,
    ) -> PhantomData<()> {
        let client = &self.manager.client;

        let request = client
            .post(self.endpoint("request_code"))
            .query(&[("code_method", method.as_str()), ("language", language)]);

        (fut_net_op(request).await?, self)
    }

    /// Step 3: Confirms the number using the code sent.
    ///
    /// # Arguments
    /// - `code`: The numeric code received by SMS or voice, without the hyphen.
    ///
    /// # Returns
    /// Next step to complete registration.
    pub async fn verify_phone_number(
        self,
        code: &str, // Verification code, without the hyphen.
    ) -> PhantomData<()>
    {
        let client = &self.manager.client;

        let request = client
            .post(self.endpoint("verify_code"))
            .query(&[("code", code)]);

        (fut_net_op(request).await?, self)
    }

    /// Step 4: Finalizes phone number registration with optional 2FA PIN.
    ///
    /// If 2FA is already enabled on the number, use the same PIN.
    ///
    /// # Arguments
    /// - `pin`: A 6-digit PIN to assign to the phone number.
    ///
    /// # Returns
    /// Final step with registration completed.
    #![optional] pub async fn set_pin(
        self,
        pin: &str,
    ) -> PhantomData<()> {
        let client = &self.manager.client;
        // If the verified business phone number already has two-step verification enabled,
        // set this value to the number's 6-digit two-step verification PIN. If you do not recall
        // the PIN, you can update it.
        // If the verified business phone number does not have two-step verification enabled,
        // set this value to a 6-digit number. This will be the business phone number's two-step
        // verification PIN.
        let request = RegisterPhoneRequest::from_pin(pin);

        let url = self.manager.endpoint("register");
        let request = client
            .post(url)
            .json(&request);

        (fut_net_op(request).await?, self)
    }
}

impl NumberRegistrationFlow<'_, '_, String, PhantomData<()>, PhantomData<()>, PhantomData<()>> {
    /// Final step: Returns the registered phone number ID.
    ///
    /// Use this to persist or associate the number with a business.
    pub fn finalize(self) -> NumberRegistrationResult {
        NumberRegistrationResult {
            phone_number_id: self.phone_number_id,
        }
    }
}

/// Result of a completed phone number registration.
#[derive(Debug)]
pub struct NumberRegistrationResult {
    /// The registered phone number ID from Meta.
    pub phone_number_id: String,
}

#[derive(Deserialize, Debug)]
pub(crate) struct RegisterResponse {
    id: String,
}

impl ToValue<'_, CatalogRef> for Catalog {
    #[inline]
    fn to_value(self) -> Cow<'static, CatalogRef> {
        Cow::Owned(CatalogRef { id: self.id })
    }
}

impl<'a> ToValue<'a, CatalogRef> for &'a Catalog {
    #[inline]
    fn to_value(self) -> Cow<'a, CatalogRef> {
        // SAFETY:
        // - `Catalog` is #[repr(C)] so fields are laid out in declared order
        // - `CatalogRef` is #[repr(C)] and only includes a prefix of `Catalog` fields.
        // - So it's safe to transmute a `&Identity` into a `&CatalogRef`.
        let view = unsafe { view_ref(self) };
        Cow::Borrowed(view)
    }
}

impl<'a> ToValue<'a, Waba> for WabaManager<'a> {
    #[inline]
    fn to_value(self) -> Cow<'a, Waba> {
        self.of
    }
}

impl<'a> ToValue<'a, Waba> for &'a WabaManager<'a> {
    #[inline]
    fn to_value(self) -> Cow<'a, Waba> {
        Cow::Borrowed(self.of.as_ref())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    // CD test
    #[test]
    fn ub() {
        let catalog = Catalog {
            id: "catalog_id".to_owned(),
            metadata: CatalogMetadata {
                name: Some("test_catalog".to_owned()),
                product_count: Some(200),
                ..Default::default()
            },
        };

        let view: &CatalogRef = unsafe { view_ref(&catalog) };

        assert_eq!(view.id, "catalog_id");

        let view_clone = view.clone();

        assert_eq!(view_clone, *view);
    }
}
