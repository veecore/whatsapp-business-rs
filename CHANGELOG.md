# Changelog

## [0.4.2] - 2025-10-31

### 🧰 Maintenance

* **CI Improvements:**

  * Added a `cargo check --no-default-features` step to ensure builds succeed when default features are disabled.
  * Enforced format, lint, and test checks to run consistently across feature configurations.

### ⚙️ Changed

* **Meta API Updated:**
  Upgraded Meta Graph API version from **v22.0 → v24.0** for improved compatibility and continued support.

----------

## [0.4.1] - 2025-10-21

### ❗ Fixed
- **Critical Batch Safety:** Fixed a critical bug in batch processing where a handler failing to consume its response could lead to **response misalignment** for all subsequent requests. The batch parser now strictly checks that all responses are consumed and will return an `ExcessiveResponses` error if any remain, preventing silent failures and data corruption.

### ✨ Added
- Added `MessageManager::get_media_info` for retrieving media information and public URLs from meta servers.
- Added several ergonomic getters to `message::Content`:
  - `text_body()`: Gets text from a `Text` message only.
  - `text()`: Gets the primary text from a `Text` body, `Media` caption, or `Order` note.
  - `media()`: Returns the `Media` payload, if present.
  - `button_click()`: Returns the `Button` payload from an interactive click response.
- Added `callback_id()` and `label()` getters to `message::Button`.
- Implemented `Deref<Target = Content>` for `Message`, allowing direct access to all new `Content` getters (e.g., `my_message.text()`).

### ⚠️ Changed
- `Requests::map_handle` is now marked `unsafe` and its documentation has been expanded to strongly warn against its use, as it can easily break batch parsing guarantees.

### 📦 Refactored
- (Internal) Reorganized all `batch` feature-gated code into separate sub-modules for better project structure and maintainability.

----------

## [0.4.0] - 2025-10-20

### Added

-   **Low-Level "Bring Your Own Server" (BYOS) API**: Introduced a new `webhook_service` module designed for users who want to integrate WhatsApp webhook logic into their own server frameworks.
    -   `whatsapp_business_rs::webhook_service::WebhookServiceBuilder`: A dedicated builder for configuring the low-level service.
    -   `whatsapp_business_rs::webhook_service::WebhookService`: The core service struct, which is `Clone`, `Send`, `Sync`, and `'static'`. It exposes a single `.handle(http::Request)` method for processing all webhook-related requests.
    -   This allows for seamless integration with external web frameworks like `axum`, `hyper`, and `warp`.
    
----------

## [0.3.1] - 2025-10-16

### Added

- A comprehensive set of builder methods on `Draft` to fluidly create interactive messages. This includes methods for setting the `body`, `header`, `footer`, and adding reply/URL `buttons`.
- New `Draft::add_list_section` method to allow creating multiple, named sections within an interactive list message, enabling more structured lists.

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