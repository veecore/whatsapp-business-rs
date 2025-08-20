# Changelog

## [0.1.0] - 2025-07-19
### Added
- Initial WhatsApp Business API support
- Media types, product catalog, messages, webhooks

## [0.1.1] - 2025-07-22
### Added
- More doc examples


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
