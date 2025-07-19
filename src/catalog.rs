//! WhatsApp Business Catalog Management
//!
//! This module provides tools to manage **product catalogs** for a WhatsApp Business Account.
//! It allows you to list, create, and update products in a catalog using structured builders.
//!
//! The key entry point is [`CatalogManager`], which can be accessed via [`Client::catalog`].
//!
//! # Features
//!
//! - List all products in a catalog
//! - Create new products
//! - Update existing products via fluent builders
//! - Model products using [`Product`] and [`ProductData`] structures
//!
//! # Examples
//!
//! ## Listing products
//! ```rust,no_run
//! use futures::TryStreamExt as _;
//! use whatsapp_business_rs::Client;
//!
//! # async fn example(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
//! let catalog = client.catalog("1234567890");
//!
//! let mut stream = catalog.list_products().into_stream();
//! while let Some(product) = stream.try_next().await? {
//!     println!("Product: {}", product.name.as_deref().unwrap_or_default())
//! }
//! # Ok(())}
//! ```
//!
//! ## Creating a product
//! ```rust,no_run
//! use whatsapp_business_rs::catalog::ProductData;
//!
//! # async fn example(catalog: &whatsapp_business_rs::catalog::CatalogManager<'_>) {
//! let product = ProductData::default()
//!     .name("Rust Programming Book")
//!     .description("Learn Rust with this comprehensive guide")
//!     .price(39.99)
//!     .currency("USD")
//!     .image_url("https://example.com/book.jpg")
//!     .build("rust-book-001");
//!
//! let created = catalog.create_product(product).await.unwrap();
//! println!("Created product ID: {}", created.product.product_id());
//! # }
//! ```
//!
//! ## Updating a product
//! ```rust,no_run
//! # async fn example(catalog: whatsapp_business_rs::catalog::CatalogManager<'_>, product_ref: whatsapp_business_rs::catalog::ProductRef) {
//! catalog.update_product(&product_ref)
//!     .name("Updated Product Name")
//!     .price(4999)
//!     .availability("in stock")
//!     .await
//!     .unwrap();
//! # }
//! ```
//!
//! [`CatalogManager`]: crate::catalog::CatalogManager
//! [`Client::catalog()`]: crate::client::Client::catalog
//! [`Product`]: crate::catalog::Product
//! [`ProductData`]: crate::catalog::ProductData

use crate::{
    rest::BuilderInto, to_value, waba::Catalog, Builder, CatalogRef, Endpoint, SimpleOutput,
    SimpleStreamOutput, ToValue, Update,
};
use std::{borrow::Cow, ops::Deref};

use crate::{client::Client, derive, Fields};
use base64::engine::general_purpose::URL_SAFE;
use base64::Engine;
use serde::{Deserialize, Serialize};

/// Manager for WhatsApp Business product catalogs.
///
/// Provides methods to list, create, and update products within a specific catalog
/// associated with your WhatsApp Business Account.
///
/// This manager is typically created using [`Client::catalog()`], which takes a
/// catalog reference (e.g., obtained from `WabaManager::list_catalogs`).
///
/// # Features
/// - Stream products from a catalog using [`ListProduct::into_stream()`].
/// - Create new products using [`CatalogManager::create_product()`] and [`ProductData::build()`].
/// - Update existing products with a fluent builder via [`CatalogManager::update_product()`]
///   or [`CatalogManager::update_product_by_id()`].
///
/// # Example
/// ```rust,no_run
/// use futures::TryStreamExt as _;
/// use whatsapp_business_rs::{Client, Waba};
///
/// # async fn example_catalog_manager(client: Client) -> Result<(), Box<dyn std::error::Error>> {
/// let waba_manager = client.waba("1234567890");
///
/// // Find the first catalog associated with this WABA
/// let first_waba_catalog = waba_manager
///     .list_catalogs()
///     .into_stream()
///     .try_next() // Get the first item from the stream
///     .await?
///     .expect("Expected at least one catalog"); // Handle possible None if stream is empty
///
/// // Create a CatalogManager for this specific catalog
/// let catalog_manager = client.catalog(first_waba_catalog);
///
/// // List all products in this catalog
/// let mut products_stream = catalog_manager.list_products().into_stream();
/// while let Some(product) = products_stream.try_next().await? {
///     println!("Found product: {:?}", product.name)
/// }
/// # Ok(())}
/// ```
///
/// [`Client::catalog()`]: crate::client::Client::catalog
/// [`ProductData::build()`]: crate::catalog::ProductData::build
/// [`CatalogManager::create_product()`]: crate::catalog::CatalogManager::create_product
/// [`CatalogManager::update_product()`]: crate::catalog::CatalogManager::update_product
/// [`CatalogManager::update_product_by_id()`]: crate::catalog::CatalogManager::update_product_by_id
/// [`ListProduct::into_stream()`]: crate::catalog::ListProduct::into_stream
#[derive(Debug)]
pub struct CatalogManager<'c> {
    client: Client,
    catalog: Cow<'c, CatalogRef>,
}

impl<'c> CatalogManager<'c> {
    pub(crate) fn new(catalog: Cow<'c, CatalogRef>, client: &Client) -> Self {
        Self {
            client: client.clone(),
            catalog,
        }
    }

    /// Prepares a request to create a new product in the catalog.
    ///
    /// This method returns a [`CreateProduct`] builder. The product is created
    /// when the returned `CreateProduct` instance is `.await`ed.
    ///
    /// # Arguments
    /// * `product` - The [`RawProduct`] data to be created. Use `ProductData::build()`
    ///   to easily construct this.
    ///
    /// # Returns
    /// A `CreateProduct` builder, which can be `.await`ed to send the creation request.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use whatsapp_business_rs::catalog::{ProductData, CatalogManager, RawProduct};
    /// # async fn example_create_product(catalog: CatalogManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
    /// let product_data = ProductData::default()
    ///      .name("Fancy Widget Pro")
    ///      .price(9999)
    ///      .currency("USD")
    ///      .description("A high-quality widget for all your needs.")
    ///      .build("fancy-widget-pro-sku"); // Your unique retailer ID
    ///
    /// let created_product_metadata = catalog.create_product(product_data).await?;
    /// println!("Created product with ID: {}", created_product_metadata.product_id());
    /// # Ok(())}
    /// ```
    /// [`RawProduct`]: crate::catalog::RawProduct
    /// [`CreateProduct`]: crate::catalog::CreateProduct
    pub fn create_product(&self, product: RawProduct) -> CreateProduct {
        let url = self.endpoint("products");
        let request = self.client.post(url).json(&product);

        CreateProduct { request }
    }

    /// Prepares a request to list products in a catalog.
    ///
    /// This method returns a [`ListProduct`] builder. To retrieve products,
    /// call `.into_stream()` on the builder and then iterate over the stream.
    ///
    /// # Returns
    /// A `ListProduct` builder, which can be configured with `metadata()`
    /// and then converted into a stream using `into_stream()`.
    ///
    /// # Example
    /// ```rust,no_run
    /// use futures::TryStreamExt as _;
    /// use whatsapp_business_rs::{
    ///     catalog::{CatalogManager, ProductDataField},
    ///     Fields,
    /// };
    ///
    /// # async fn example_list_products(catalog: &CatalogManager<'_>)  -> Result<(), Box<dyn std::error::Error>> {
    /// // List all products, requesting only their name and price
    /// let mut products_stream = catalog
    ///     .list_products()
    ///     .metadata(
    ///         Fields::new()
    ///             .with(ProductDataField::Name)
    ///             .with(ProductDataField::Price),
    ///     )
    ///     .into_stream();
    ///
    /// while let Some(product) = products_stream.try_next().await? {
    ///     println!("Product: {:?} - Price: {:?}", product.name, product.price)
    /// }
    /// # Ok(())}
    /// ```
    /// [`ListProduct`]: crate::catalog::ListProduct
    pub fn list_products(&self) -> ListProduct {
        let url = self.endpoint("products");
        let request = self.client.get(url);

        ListProduct { request }
    }

    /// Initiates an update operation for a product using its Meta-assigned ID.
    ///
    /// This method returns an [`Update<ProductData>`] builder that allows you to
    /// fluently set specific fields for the product (e.g., name, price, availability).
    /// The update is sent when the returned builder is `.await`ed.
    ///
    /// # Parameters
    /// - `product`: A reference to the [`MetaProductRef`] for the product you want to update.
    ///
    /// # Returns
    /// An `Update<ProductData>` builder. Await this builder to execute the update.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use whatsapp_business_rs::catalog::{CatalogManager, MetaProductRef};
    /// # async fn example_update_by_meta_id(catalog: &CatalogManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
    /// let product_meta_ref = MetaProductRef::from("123456789012345"); // Meta's product ID
    ///
    /// catalog.update_product_by_id(&product_meta_ref)
    ///      .name("Super Shiny Widget (Updated)")
    ///      .price(8999) // New price
    ///      .currency("USD")
    ///      .availability("in stock")
    ///      .await?; // Await to send the update
    ///
    /// # Ok(())}
    /// ```
    ///
    /// # Notes
    /// - The product must exist in the catalog.
    /// - This method uses the Graph API endpoint for product updates directly.
    ///
    /// [`Update<ProductData>`]: crate::catalog::Update
    /// [`MetaProductRef`]: crate::catalog::MetaProductRef
    pub fn update_product_by_id<'p, P>(&self, product: P) -> Update<'p, ProductData>
    where
        P: ToValue<'p, MetaProductRef>,
    {
        let product = product.to_value();
        let url = self.client.a_node(&product.product_id);
        let request = self.client.post(url);
        Update::new(request)
    }

    /// Initiates an update operation for a product using its retailer ID.
    ///
    /// This method returns an [`Update<ProductData>`] builder that allows you to
    /// fluently set specific fields for the product (e.g., name, price, availability).
    /// The update is sent when the returned builder is `.await`ed.
    ///
    /// # Parameters
    /// - `product`: A reference to the [`ProductRef`] for the product you want to update.
    ///
    /// # Returns
    /// An `Update<ProductData>` builder. Await this builder to execute the update.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use whatsapp_business_rs::catalog::{CatalogManager, ProductRef};
    /// # async fn example_update_by_retailer_id(catalog: &CatalogManager<'_>) -> Result<(), Box<dyn std::error::Error>> {
    /// let product_retailer_ref = ProductRef::from("my-unique-shirt-sku"); // Your internal retailer ID
    ///
    /// catalog.update_product(&product_retailer_ref)
    ///      .name("Premium T-Shirt (New Season)")
    ///      .price(12999) // New price
    ///      .currency("EUR")
    ///      .availability("out of stock")
    ///      .await?; // Await to send the update
    ///
    /// # Ok(())}
    /// ```
    ///
    /// # Notes
    /// - The product must exist in the catalog.
    /// - This method uses the `retailer_id` to target the product.
    ///
    /// [`Update<ProductData>`]: crate::catalog::Update
    /// [`ProductRef`]: crate::catalog::ProductRef
    pub fn update_product<'p, P>(&self, product: P) -> Update<'p, ProductData>
    where
        P: ToValue<'p, ProductRef>,
    {
        let catalog_id = &self.catalog.id;
        let product = product.to_value();
        let retailer_id_b64 = URL_SAFE.encode(product.product_retailer_id());
        let product_uri = format!("catalog:{catalog_id}:{retailer_id_b64}");

        let url = self.client.a_node(&product_uri);
        let request = self.client.post(url);
        Update::new(request)
    }

    Endpoint! {catalog.id}
}

SimpleStreamOutput! {
    ListProduct => Product
}

impl ListProduct {
    /// Filters the product list to include only specified fields in the response.
    ///
    /// By default, the API might return a standard set of fields. Use this method
    /// to request only the fields you need, which can reduce payload size and
    /// improve performance.
    ///
    /// # Parameters
    /// - `metadata`: A [`Fields`] instance containing the desired [`ProductDataField`]s.
    ///
    /// # Returns
    /// The updated `ListProduct` builder.
    ///
    /// # Example
    /// ```rust
    /// use whatsapp_business_rs::{catalog::ProductDataField, Fields};
    ///
    /// # async fn example_metadata(catalog: &whatsapp_business_rs::catalog::CatalogManager<'_>) {
    /// // Request only the product name and price
    /// let products_builder = catalog.list_products()
    ///      .metadata(Fields::new()
    ///          .with(ProductDataField::Name)
    ///          .with(ProductDataField::Price));
    /// // Now call .into_stream() on `products_builder`
    /// # }
    /// ```
    /// [`Fields`]: crate::Fields
    /// [`ProductDataField`]: crate::catalog::ProductDataField
    pub fn metadata(mut self, metadata: Fields<ProductDataField>) -> Self {
        self.request = metadata.into_request(self.request, ["id", "retailer_id"]);
        self
    }
}

SimpleOutput! {
    CreateProduct => ProductCreate
}

/// Represents a product listed in a WhatsApp Business catalog.
///
/// A `Product` is returned from listing operations and includes both WhatsApp-managed
/// metadata (like `id`) and your own fields (like `retailer_id`, name, price, etc.).
///
/// Use [`Product::as_ref()`] to get a lightweight reference for updates or identification.
///
/// # Example
/// ```rust
/// # async fn example(catalog: whatsapp_business_rs::catalog::CatalogManager<'_>) {
/// let products = catalog.list_products().into_stream();
/// # }
/// ```
///
/// [`Product::as_ref()`]: crate::catalog::Product::as_ref
#[derive(Deserialize, PartialEq, Debug)]
pub struct Product {
    /// WhatsApp-generated product ID
    pub id: String,
    /// The catalog the product belongs to (optional)
    #[serde(rename = "product_catalog")]
    pub catalog: Option<Catalog>,
    /// Uploaded product data
    #[serde(flatten)]
    pub raw: RawProduct,
}

impl Deref for Product {
    type Target = RawProduct;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

/// A new product to be created in the catalog
///
/// Built using [`ProductData::build()`] after setting all desired fields.
#[derive(PartialEq, Serialize, Deserialize, Debug)]
pub struct RawProduct {
    /// Your internal product ID
    pub retailer_id: String,
    #[serde(flatten)]
    pub data: ProductData,
}

impl Deref for RawProduct {
    type Target = ProductData;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl RawProduct {
    pub fn builder() -> ProductData {
        ProductData::default()
    }
}

derive! {
    /// Core fields of a WhatsApp product.
    ///
    /// Use [`ProductData::default()]` or [`RawProduct::builder()`] to build up product attributes.
    /// These fields define what a user sees when browsing your catalog.
    #[derive(#Update, #Fields, #Builder, PartialEq, Serialize, Deserialize, Debug, Default)]
    #[non_exhaustive]
    pub struct ProductData {
        /// Display name of the product
        pub name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub rich_text_description: Option<String>,
        /// Detailed product description
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub description: Option<String>,
        /// Product color (used for filtering)
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub color: Option<String>,
        /// Product size (free-form or standardized)
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub size: Option<String>,
        /// Product category (arbitrary string)
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub category: Option<String>,
        /// Price (as float)
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub price: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub sale_price: Option<f64>,
        /// Currency code (ISO 4217)
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub currency: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub quantity_to_sell_on_facebook: Option<usize>,
        /// Product brand
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub brand: Option<String>,
        /// Availability status (e.g. `"in stock"`)
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub availability: Option<String>,
        /// List of image URLs
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub images: Option<Vec<ProductImage>>,
        /// Primary image URL
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub image_url: Option<String>,
        /// URL to view or buy product
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub url: Option<String>,
        /// Hide this product from listings
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub is_hidden: Option<bool>,
        /// Shipping/delivery category
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub delivery_category: Option<String>,
        /// For variants
        pub item_group_id: Option<String>,
    }
}

#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
#[non_exhaustive]
pub struct ProductImage {
    pub url: String,
}

impl ProductData {
    pub fn build(self, retailer_id: &str) -> RawProduct {
        RawProduct {
            retailer_id: retailer_id.to_owned(),
            data: self,
        }
    }
}

/// Lightweight reference to an existing product
///
/// Use when updating or deleting a product without fetching the full [`Product`].
///
/// # Example
/// ```rust
/// use whatsapp_business_rs::catalog::ProductRef;
///
/// let reference = ProductRef::from_product_retailer_id("book-123");
/// ```
#[derive(Serialize, Deserialize, Eq, Clone, Debug)]
pub struct ProductRef {
    /// Retailer-defined product ID
    product_retailer_id: String,
    /// Optional catalog ref
    #[serde(
        rename = "catalog_id",
        skip_serializing_if = "Option::is_none",
        default
    )]
    catalog: Option<CatalogRef>,
}

impl Product {
    pub fn as_ref(&self) -> ProductRef {
        ProductRef {
            product_retailer_id: self.raw.retailer_id.clone(),
            catalog: self.catalog.as_ref().map(Catalog::as_ref),
        }
    }
}

impl ProductRef {
    pub fn from_product_retailer_id(id: impl Into<String>) -> Self {
        Self {
            product_retailer_id: id.into(),
            catalog: None,
        }
    }

    pub fn with_catalog(mut self, catalog: impl Into<CatalogRef>) -> Self {
        self.catalog = Some(catalog.into());
        self
    }

    pub fn product_retailer_id(&self) -> &str {
        &self.product_retailer_id
    }

    pub fn catalog(&self) -> Option<&CatalogRef> {
        self.catalog.as_ref()
    }
}

/// Reference to a product using WhatsApp's internal ID.
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct MetaProductRef {
    product_id: String,
}

impl Product {
    pub fn as_ref_meta(&self) -> MetaProductRef {
        MetaProductRef {
            product_id: self.id.clone(),
        }
    }
}

impl MetaProductRef {
    pub fn from_product_id(id: impl Into<String>) -> Self {
        Self {
            product_id: id.into(),
        }
    }

    pub fn product_id(&self) -> &str {
        &self.product_id
    }
}

/// Result of a successful product creation
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ProductCreate {
    /// Reference to the newly created product
    pub product: MetaProductRef,
}

impl Deref for ProductCreate {
    type Target = MetaProductRef;

    fn deref(&self) -> &Self::Target {
        &self.product
    }
}

impl PartialEq for ProductRef {
    fn eq(&self, other: &Self) -> bool {
        self.product_retailer_id == other.product_retailer_id
    }
}

impl PartialEq<Product> for ProductRef {
    fn eq(&self, other: &Product) -> bool {
        self.product_retailer_id == other.retailer_id
    }
}

impl PartialEq<MetaProductRef> for ProductCreate {
    fn eq(&self, other: &MetaProductRef) -> bool {
        self.product.eq(other)
    }
}

impl PartialEq<Product> for MetaProductRef {
    fn eq(&self, other: &Product) -> bool {
        self.product_id == other.id
    }
}

to_value! {
    ProductRef MetaProductRef
}

impl<T: Into<String>> From<T> for MetaProductRef {
    #[inline]
    fn from(value: T) -> Self {
        Self::from_product_id(value)
    }
}

impl<T: Into<String>> From<T> for ProductRef {
    #[inline]
    fn from(value: T) -> Self {
        Self::from_product_retailer_id(value)
    }
}

impl ToValue<'_, ProductRef> for Product {
    #[inline]
    fn to_value(self) -> Cow<'static, ProductRef> {
        Cow::Owned(ProductRef {
            product_retailer_id: self.retailer_id.clone(),
            catalog: self.catalog.map(|c| c.to_value().into_owned()),
        })
    }
}

// TOVIEW
impl ToValue<'_, ProductRef> for &Product {
    #[inline]
    fn to_value(self) -> Cow<'static, ProductRef> {
        Cow::Owned(self.as_ref())
    }
}

impl ToValue<'_, MetaProductRef> for Product {
    #[inline]
    fn to_value(self) -> Cow<'static, MetaProductRef> {
        Cow::Owned(MetaProductRef {
            product_id: self.id,
        })
    }
}

// TOVIEW
impl ToValue<'_, MetaProductRef> for &Product {
    #[inline]
    fn to_value(self) -> Cow<'static, MetaProductRef> {
        Cow::Owned(self.as_ref_meta())
    }
}

impl<'m> ToValue<'m, MetaProductRef> for &'m ProductCreate {
    #[inline]
    fn to_value(self) -> Cow<'m, MetaProductRef> {
        Cow::Borrowed(&self.product)
    }
}

impl ToValue<'_, MetaProductRef> for ProductCreate {
    #[inline]
    fn to_value(self) -> Cow<'static, MetaProductRef> {
        Cow::Owned(self.product)
    }
}

impl<'a> ToValue<'a, CatalogRef> for CatalogManager<'a> {
    #[inline]
    fn to_value(self) -> Cow<'a, CatalogRef> {
        self.catalog
    }
}

impl<'a> ToValue<'a, CatalogRef> for &'a CatalogManager<'a> {
    #[inline]
    fn to_value(self) -> Cow<'a, CatalogRef> {
        Cow::Borrowed(self.catalog.as_ref())
    }
}
