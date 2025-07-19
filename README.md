<p align="center">
  <img src="https://raw.githubusercontent.com/veecore/whatsapp-business-rs/main/assets/roxy.png" width="160" alt="Roxy logo" />
</p>


# ⚡️ whatsapp-business-rs
[![Crates.io](https://img.shields.io/crates/v/whatsapp-business-rs)](https://crates.io/crates/whatsapp-business-rs)
[![Docs.rs](https://docs.rs/whatsapp-business-rs/badge.svg)](https://docs.rs/whatsapp-business-rs)
[![CI](https://github.com/veecore/whatsapp-business-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/veecore/whatsapp-business-rs/actions/workflows/ci.yml)

### The ultimate Rust SDK for building badass WhatsApp Business integrations.

`whatsapp-business-rs` is your all-in-one, type-safe, async-ready toolkit for harnessing the full power of Meta's WhatsApp Business Platform — built with love in Rust 🦀.

Whether you're sending messages, managing catalogs, automating onboarding, or spinning up a webhook server that never sleeps — this crate brings it all together in a blazing-fast, developer-first package.

---

## ✨ Features That Matter

✅ **Rich Message Support**
Send text, images, video, documents, stickers, buttons, lists, reactions, and more — all with a single expressive API.

✅ **First-Class Client API**
A fluent, builder-based interface that makes requests feel like composing jazz — fast, elegant, and deeply ergonomic.

✅ **Zero-Hassle Webhook Server**
Spin up a signature-verified webhook in minutes — receive and respond to messages, status updates, and more with async power.

✅ **WABA & App Management**
Administer phone numbers, catalogs, subscriptions, and business onboarding flows — all programmatically.

✅ **Catalog Commerce, the Rust Way**
Create, update, and list your product catalogs like a pro. Rust safety, zero guesswork.

---

## 📦 Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
whatsapp-business-rs = "0.1.0" # Use the latest version
```

Need direct message replies via incoming messages? Enable the feature:

```toml
[dependencies]
whatsapp-business-rs = { version = "0.1.0", features = ["incoming_message_ext"] }
```

---

## 🚀 Quickstart Examples

### 🔹 Send a Message (Text)

```rust
use whatsapp_business_rs::message::Draft;
use whatsapp_business_rs::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new("YOUR_ACCESS_TOKEN").await?;
    client.message("YOUR_BUSINESS_NUMBER_ID")
          .send("+16012345678", "Hello from Rust! How can I help you today?")
          .await?;
    Ok(())
}
```

### 🔹 Create a Client (with Timeout & Version)

```rust
use std::time::Duration;
use whatsapp_business_rs::client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .api_version("v19.0")
        .connect("YOUR_ACCESS_TOKEN")
        .await?;
    Ok(())
}
```

### 🔹 Start a Webhook Server

```rust
use whatsapp_business_rs::{
    client::Client,
    server::{Server, Handler, EventContext, IncomingMessage},
    message::{Button},
    Error
};

struct MyHandler;

impl Handler for MyHandler {
    async fn handle_message(&self, _ctx: EventContext, msg: IncomingMessage) {
        println!("Received: {}", msg);
        // Compare anything
        if msg == Button::reply("no_callback", "No") {
            msg.reply("Thanks for your message!").await.unwrap();
        }
    }
}

#[tokio::main]
async fn main() {
    let server = Server::builder()
        .endpoint("127.0.0.1:8080".parse().unwrap())
        .build();

    let client = Client::new("YOUR_ACCESS_TOKEN").await.unwrap();
    let handler = MyHandler;
    server.serve(handler, client).await.unwrap();
}
```

### 🔹 Configure a Webhook

```rust
use whatsapp_business_rs::{
    app::{SubscriptionField, WebhookConfig},
    client::Client,
    App,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = App::new("YOUR_APP_ID");
    let config = WebhookConfig {
        webhook_url: "https://example.com/webhook".to_string(),
        verify_token: "very_secret".into(),
    };

    let client = Client::new("YOUR_ACCESS_TOKEN").await?;
    client
        .app(app)
        .configure_webhook(&config)
        .events([
            SubscriptionField::Messages,
            SubscriptionField::MessageTemplateStatusUpdate,
        ].into())
        .await?;
    Ok(())
}
```

### 🔹 List Product Catalogs

```rust
use whatsapp_business_rs::{client::Client, Waba, waba::CatalogMetadataField};
use futures::TryStreamExt as _;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new("YOUR_ACCESS_TOKEN").await?;
    let business = Waba::new("YOUR_WABA_ID");

    let mut catalogs = client
        .waba(business)
        .list_catalogs()
        .metadata([
            CatalogMetadataField::Name,
            CatalogMetadataField::Vertical
        ].into())
        .into_stream();

    while let Some(catalog) = catalogs.try_next().await? {
        println!("{:?}", catalog);
    }

    Ok(())
}
```

### 🔹 Create a Product

```rust
use whatsapp_business_rs::catalog::ProductData;
use whatsapp_business_rs::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new("YOUR_ACCESS_TOKEN").await?;
    let product = ProductData::default()
        .name("Rust Programming Book")
        .description("Learn Rust with this comprehensive guide")
        .price(39.99)
        .currency("USD")
        .image_url("https://example.com/book.jpg")
        .build("rust-book-001");

    let result = client.catalog("YOUR_CATALOG_ID")
        .create_product(product)
        .await?;
    println!("Product created: {}", result.product.product_id());
    Ok(())
}
```

---

## 💬 Why `whatsapp-business-rs`?

* 🧠 **Zero guessing** – Compile-time type guarantees.
* ⚙️ **Built for production** – Powered by `reqwest`, `axum`, `tokio`, and battle-tested Rust primitives.
* 🧪 **Testable** – No more fragile mocks or boilerplate HTTP.
* 💥 **Extensible** – Add your own layers or handlers — or fork and fly.

---

## 🔧 Work in Progress

This crate is young but fierce. We're actively improving coverage across message templates, contacts, and more. Contributions welcome!

---

## 🦀 Let's Rust WhatsApp Right.

Tired of bloated SDKs, missing docs, or inconsistent behavior?
With `whatsapp-business-rs`, **you own the stack** — fast, clean, async-native.
Perfect for bots, CRMs, marketplaces, and next-gen commerce apps.
