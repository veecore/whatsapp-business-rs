<p align="center">
  <img src="https://raw.githubusercontent.com/veecore/whatsapp-business-rs/main/assets/roxy.png" width="160" alt="Roxy logo" />
</p>


# ⚡️ whatsapp-business-rs
[![Crates.io](https://img.shields.io/crates/v/whatsapp-business-rs)](https://crates.io/crates/whatsapp-business-rs)
[![Docs.rs](https://docs.rs/whatsapp-business-rs/badge.svg)](https://docs.rs/whatsapp-business-rs)
[![CI](https://github.com/veecore/whatsapp-business-rs/actions/workflows/ci.yaml/badge.svg)](https://github.com/veecore/whatsapp-business-rs/actions/workflows/ci.yaml)


### The ultimate Rust SDK for building badass WhatsApp Business integrations.

`whatsapp-business-rs` is your all-in-one, type-safe, async-ready toolkit for harnessing the full power of Meta's WhatsApp Business Platform — built with love in Rust 🦀.

Whether you're sending messages, managing catalogs, automating onboarding, or spinning up a webhook server that never sleeps — this crate brings it all together in a blazing-fast, developer-first package.

-----

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

✅ **Multi-Tenant Ready**
Designed with flexibility in mind, `whatsapp-business-rs` supports multi-tenant applications by allowing you to specify different authentication tokens for individual requests, eliminating the need to re-initialize the client for each business account. This is perfect for SaaS platforms managing multiple WhatsApp Business Accounts\!

-----

## 📦 Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
whatsapp-business-rs = "0.1.1" # Use the latest version
```

Need direct message replies via incoming messages? Enable the feature:

```toml
[dependencies]
whatsapp-business-rs = { version = "0.1.0", features = ["incoming_message_ext"] }
```

-----

## 🚀 Quickstart Examples

### 🔹 Send a Message (Text)

```rust
use whatsapp_business_rs::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let client = Client::new("YOUR_ACCESS_TOKEN").await?; // Initialize your client
	let business_phone_id = "YOUR_BUSINESS_NUMBER_ID"; // Replace with your WhatsApp Phone Number ID
	let recipient_number = "+16012345678"; // Replace with recipient's phone number

	// Send a simple text message
	client.message(business_phone_id)
		.send(recipient_number, "Hello from Rust! How can I help you today?")
		.await?;
	println!("Text message sent!");

	// For multi-tenant applications, you can specify an alternate token for a specific message:
	client.message(business_phone_id)
		.send(recipient_number, "This message uses a different token!")
		.with_auth("ANOTHER_BUSINESS_ACCESS_TOKEN")
		.await?;
	println!("Text message sent with an alternate token!");

	Ok(())
}
```

-----

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
	println!("Client created with custom timeout and API version!");
	Ok(())
}
```

-----

### 🔹 Send a Media Message (e.g., Video)

```rust
use whatsapp_business_rs::{Client, Media};

async fn send_video_example() -> Result<(), Box<dyn std::error::Error>> {
	let client = Client::new("YOUR_ACCESS_TOKEN").await?;

	let sender_phone_id = "YOUR_BUSINESS_PHONE_NUMBER_ID";
	let recipient_phone_number = "+16012345678";

	// Example: Send a video from a file path
	// Make sure to replace "path/to/your/video.mp4" with an actual path
	let video = Media::from_path("path/to/your/video.mp4")
		.await?
		.caption("Check out this cool video!");

	client.message(sender_phone_id).send(recipient_phone_number, video).await?;
	println!("Video message sent!");
	Ok(())
}
```

-----

### 🔹 Send a Location Message

```rust
use whatsapp_business_rs::{Client, Location};

async fn send_location_example() -> Result<(), Box<dyn std::error::Error>> {
	let client = Client::new("YOUR_ACCESS_TOKEN").await?;

	let sender_phone_id = "YOUR_BUSINESS_PHONE_NUMBER_ID";
	let recipient_phone_number = "+16012345678";

	// Send a location message with name and address
	let location = Location::new(37.44216251868683, -122.16153582049394)
		.name("Philz Coffee")
		.address("101 Forest Ave, Palo Alto, CA 94301");

	client.message(sender_phone_id).send(recipient_phone_number, location).await?;
	println!("Location message sent!");
	Ok(())
}
```

-----

### 🔹 Send an Interactive Message (Call-to-Action Button)

```rust
use whatsapp_business_rs::message::{InteractiveMessage, UrlButton};
use whatsapp_business_rs::Client;

async fn send_cta_example() -> Result<(), Box<dyn std::error::Error>> {
	let client = Client::new("YOUR_ACCESS_TOKEN").await?;

	let sender_phone_id = "YOUR_BUSINESS_PHONE_NUMBER_ID";
	let recipient_phone_number = "+16012345678";

	// Create a URL button
	let button = UrlButton::new("https://example.com/purchase", "2 for price of 1");

	// Create an interactive message with the button and a body text
	let interactive_message = InteractiveMessage::new(
		button,
		"One-time offer for this shiny product. Hurry-up!"
	);

	client.message(sender_phone_id).send(recipient_phone_number, interactive_message).await?;
	println!("Call-to-Action message sent!");
	Ok(())
}
```

-----

### 🔹 Send an Interactive Message (List of Options)

```rust
use whatsapp_business_rs::message::{InteractiveMessage, OptionList, OptionButton, Section};
use whatsapp_business_rs::Client;

async fn send_option_list_example() -> Result<(), Box<dyn std::error::Error>> {
	let client = Client::new("YOUR_ACCESS_TOKEN").await?;

	let sender_phone_id = "YOUR_BUSINESS_PHONE_NUMBER_ID";
	let recipient_phone_number = "+16012345678";

	// Create sections with options
	let section = Section::new(
		"Choose an option regarding your last order",
		[
			OptionButton::new("Cancels deliver", "Cancel", "cancel-delivery"),
			OptionButton::new("Proceed with order", "Order", "proceed-with-order"),
		],
	);

	// Create an option list with the section and a global label
	let options = OptionList::new_section(section, "Delivery Options");

	// Create an interactive message with the option list, body, and footer
	let interactive_message = InteractiveMessage::new(
		options,
		"Sorry your delivery is delayed... would you like to cancel the order?",
	).footer("Only 70% in refund");

	client.message(sender_phone_id).send(recipient_phone_number, interactive_message).await?;
	println!("Option list message sent!");
	Ok(())
}
```

-----

### 🔹 Start a Webhook Server and Echo Messages

```rust
use whatsapp_business_rs::{
    client::Client,
    server::{Server, WebhookHandler, EventContext, IncomingMessage},
    app::SubscriptionField,
    Auth,
};
use std::error::Error;

// Define a handler struct for incoming webhook events
#[derive(Debug)]
struct EchoServerHandler {
	// Store a client to send replies or just enable incoming_message_ext feature for ease
    client: Client,
}

impl EchoServerHandler {
    fn new(client: Client) -> Self {
        Self { client }
    }
}

impl WebhookHandler for EchoServerHandler {
    // This method handles incoming messages
    async fn handle_message(
        &self,
        _ctx: EventContext,
        IncomingMessage { message, .. }: IncomingMessage,
    ) {
        println!("Received message: {:#?}", message);
        // Echo the received message content back to the sender
        if let Err(e) = self.client
            .message(message.recipient) // The recipient for the echo is the original sender's ID
            .send(message.sender, message.content) // Send the same content back
            .await
        {
            eprintln!("Failed to send echo message: {}", e);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize a client with your app token (or system user token)
    // For app actions, it's recommended to use `Auth::secret`
    let client = Client::new(Auth::secret(("YOUR_APP_ID", "YOUR_APP_SECRET"))).await?;

    // Your public webhook URL (e.g., from ngrok) and verify token
    let webhook_url = "https://your-ngrok-url.ngrok-free.app/webhook"; // Replace with your actual URL
    let verify_token = "my_super_secret_verify_token";

    // Configure the webhook subscription with Meta
    let pending_configure = client
        .app("YOUR_APP_ID") // Replace with your app ID
        .configure_webhook((webhook_url, verify_token))
        .events([SubscriptionField::Messages].into()); // Subscribe to message events

    // Create an instance of your webhook handler
    let handler = EchoServerHandler::new(client);

    // Build and start the webhook server
    Server::builder()
        .endpoint("127.0.0.1:8080".parse().unwrap()) // The local address where your server will listen
        .verify_payload("YOUR_APP_SECRET") // Use your app secret for payload verification
        .build()
        .serve(handler)
        .configure_webhook(verify_token, pending_configure) // Register webhook with Meta through the server
        .await?;

    println!("Webhook server started and configured!");
    Ok(())
}
```

-----

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

	println!("Listing catalogs:");
	while let Some(catalog) = catalogs.try_next().await? {
		println!("{:?}", catalog);
	}

	Ok(())
}
```

-----

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
	println!("Product created with ID: {}", result.product.product_id());
	Ok(())
}
```

-----

## 💬 Why `whatsapp-business-rs`?

  * 🧠 **Zero guessing** – Compile-time type guarantees ensure your integrations are robust and reliable.
  * ⚙️ **Built for production** – Powered by `reqwest`, `axum`, `tokio`, and other battle-tested Rust primitives, offering high performance and scalability.
  * 🧪 **Testable** – Say goodbye to fragile mocks and boilerplate HTTP. Our design makes your code inherently testable.
  * 💥 **Extensible** – Add your own layers or handlers, or fork and customize to fit your unique needs.
  * 🚀 **Multi-Tenant Support** – Effortlessly manage multiple WhatsApp Business Accounts from a single application instance. The **`.with_auth("ANOTHER_TOKEN")`** method allows you to override the default client authentication for specific requests, enabling seamless multi-tenancy without re-initializing clients.

-----

## 🔧 Work in Progress

This crate is young but fierce. We're actively improving coverage across message templates, contacts, and more. Contributions are always welcome\!

-----

## 🦀 Let's Rust WhatsApp Right.

Tired of bloated SDKs, missing docs, or inconsistent behavior?
With `whatsapp-business-rs`, **you own the stack** — fast, clean, async-native.
Perfect for bots, CRMs, marketplaces, and next-gen commerce apps.

Ready to build something amazing? Give `whatsapp-business-rs` a try and join our growing community\!