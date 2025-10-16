# Changelog


# [0.3.1] - 2025-10-16

This release focuses on significantly improving the ergonomics of building interactive messages. 🎉

We've supercharged the `Draft` struct with a fluent builder pattern, making it simpler and more intuitive to construct everything from simple button replies to complex, multi-section lists.

## ✨ Key Features:
- **Fluent Interactive Builders**: Easily chain methods like `.body()`, `.header()`, `.footer()`, and `.add_reply_button()` to build your message step-by-step.
- **Structured Lists**: A new `add_list_section("Section Title")` method gives you full control over organizing list options into distinct, named sections.

This update makes the API more expressive and less error-prone, helping you build powerful interactive experiences with ease.

----------

## [0.3.0] - 2025-10-14

This is a significant release focused on major internal refactoring to improve performance and type safety, while also introducing powerful new features and key API improvements.

### 🚀 Features

* **Batch Response Mapping**: A new `.map()` method on `Requests` allows for powerful, type-safe post-processing of individual responses within a batch operation. You can now transform results, send them to channels, or trigger side-effects right after a request executes.
* **Skippable Flow Steps**: The onboarding and number registration flows (`OnboardingFlow`, `NumberRegistrationFlow`) are now much more flexible. You can use new `set_*()` methods to supply data for a step you've already completed elsewhere, effectively "fast-forwarding" the flow.
* **List WABA Flows**: Added a `waba.list_flows()` method to retrieve all message flows associated with a WhatsApp Business Account.

### 💥 Breaking Changes

* **`ProductData` Price Field**: The `price: f64` field in `ProductData` has been replaced with `price: Option<Price>`, where `Price` is a new struct containing `amount` and `currency`. This aligns correctly with the Meta API.
* **Default Server Route**: The default path for the webhook server has changed from `/whatsapp_webhook` to `/`. This provides a more sensible default as the server now handles all traffic on its root.
* **Webhook Registration API**: The `Serve::configure_webhook(verify_token, pending)` method is now **deprecated**. It is replaced by the much safer and simpler **`Serve::register_webhook(pending)`**, which automatically extracts the `verify_token` from the pending request, eliminating parameter repetition.

### ✨ Improvements

* **Easier Interactive Messages**: You can now create an `InteractiveMessage` draft from simple tuples like `(Body, Action)` or `(Header, Body, Action, Footer)`, making the process far more intuitive.
* **Performance Boost**: The internal request serialization pipeline has been completely rewritten. By using a structural approach with our new `serde_metaform` crate, we've achieved a **~40% throughput improvement** in real-world scenarios and a **~2.5x speedup** in serialization benchmarks, all while reducing memory allocations.
* **Enhanced Type Safety**: The move away from dynamic request building to a structural `PendingRequest` system has eliminated previous `unsafe` code and fragile logic, making the library more robust.
* **Documentation**: Added extensive documentation and examples for new features, especially for the `batch::Requests::map` method and server configuration.

### 🛠️ Internal

* **Structural Requests**: Core request objects no longer wrap `reqwest::RequestBuilder`. Instead, they are represented by a new `PendingRequest<'a, P, Q>` struct, which holds the payload and query structurally until execution.
* **Centralized Macros**: All internal macros have been moved to the central `src/rest/macros.rs` module for better organization.
* **Dependencies**: The `USER_AGENT` now correctly uses the crate's version.

-------

## [0.2.0] - 2025-08-20
### 🚀 Added

* **`Auth::Parsed` variant**

  * Pre-parsed and optimized authentication token backed by `bytes::Bytes`.
  * Cheap to clone, reduces memory allocations for repeated header use.
  * Created by converting `Auth::Token` or `Auth::Secret` with `.parsed()`.

* **Batch API**

  * Sleek, type-safe API for batching requests in one round-trip.
  * Supports **symbolic response linking** for safe dependent requests.
  * Scales to thousands of requests efficiently.

* **Media helpers**

  * Added `.png()` and `.into_upload_parts()` for easier media workflows.

* **Expose `mime_type`** on `MediaType`.

* **WebhookHandler enhancements**

  * Default `handle_error` now includes unix time in logs.
  * Blanket `FnOnce` impl for `WebhookHandler`, enabling ad-hoc closures as handlers (closures receive the full `Event` enum).

* **Feature flags**

  ```toml
  [features]
  default   = ["batch", "server", "media_ext"]
  batch     = ["percent-encoding"]
  server    = ["axum", "hmac", "sha2", "hex", "subtle"]
  media_ext = ["infer"]
  ```

### ✨ Changed

* Client creation:

  * Now possible to build unauthenticated clients with `ClientBuilder`.
  * Client creation no longer requires `async` (non-breaking for existing code).
* Error messages improved for smoother grammar.

### 💥 Breaking

* Removed blanket `Into<String>` implementations for auths.

  * Replaced with explicit support for `Cow`, `String`, and `&str`.
  * Most existing code should continue to compile.

-------

## [0.1.1] - 2025-07-22
### Added
- More doc examples

-------

## [0.1.0] - 2025-07-19
### Added
- Initial WhatsApp Business API support
- Media types, product catalog, messages, webhooks