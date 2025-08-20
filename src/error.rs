//! Error Handling
//!
//! This module defines the crate's core error types, providing a structured way to handle
//! various issues that can occur during interactions with the WhatsApp Business API,
//! network operations, and data processing.

use std::error::Error as StdError;

use reqwest::StatusCode;

use crate::MetaError;

/// The **top-level error enum** for the `whatsapp-business-rs` crate.
///
/// This enum aggregates various categories of errors that can occur within the
/// library, providing a unified error handling mechanism. It uses `#[non_exhaustive]`
/// to allow for future additions of error variants without breaking client code.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// Represents an error occurring during network operations (e.g., HTTP requests,
    /// connection issues, DNS resolution failures, or TLS errors).
    ///
    /// This variant wraps a `BoxError` (a boxed `dyn std::error::Error + Send + Sync`)
    /// to handle underlying network client errors, typically from `reqwest`.
    #[error("A network error occurred: {0}")]
    Network(#[from] BoxError),

    /// Represents an error specifically related to the WhatsApp Business API service
    /// or data processing.
    ///
    /// This variant wraps a `ServiceError` which further categorizes issues like
    /// API-specific errors or parsing failures. It is distinct from `MetaError`,
    /// which describes errors *reported by Meta itself in an API response body*.
    #[error("An API service or data processing error occurred: {0}")]
    Service(#[from] ServiceError),

    /// Represents an **I/O error** that occurred during IO operations,
    /// such as downloading media to disk.
    #[error("An I/O error occurred: {0}")]
    Io(#[from] std::io::Error),

    /// Represents an **internal logic error** within the `whatsapp-business-rs` crate,
    /// or an error caused by invalid input that should have been caught earlier (e.g.,
    /// providing non-UTF8 characters where ASCII/UTF-8 is strictly required,
    /// or an unexpected state during request building).
    ///
    /// These errors typically indicate a bug in the library itself or misuse of the API
    /// that isn't handled gracefully by other error variants.
    #[error("An internal library error occurred: {0}")]
    Internal(BoxError),
}

impl Error {
    pub(crate) fn network(err: BoxError) -> Self {
        Self::Network(err)
    }

    pub(crate) fn internal(err: BoxError) -> Self {
        Self::Internal(err)
    }
}

/// Represents **service-level errors** encountered during API interactions or data processing.
/// This struct provides context such as the HTTP status code, the affected endpoint,
/// and a more specific error kind.
#[derive(thiserror::Error, Debug)]
#[error("Service error at endpoint '{endpoint:?}': {kind} (HTTP status {status})")]
#[non_exhaustive]
pub struct ServiceError {
    pub(crate) status: StatusCode,
    pub(crate) kind: ServiceErrorKind,
    #[cfg(debug_assertions)]
    pub(crate) endpoint: Option<String>,
    // TODO: Include method
}

impl ServiceError {
    /// Returns the HTTP status code associated with this service error.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the API endpoint (if known) where this service error occurred.
    pub fn endpoint(&self) -> Option<&str> {
        #[cfg(debug_assertions)]
        {
            self.endpoint.as_deref()
        }

        #[cfg(not(debug_assertions))]
        {
            None
        }
    }

    /// Returns the specific kind of service error.
    pub fn kind(&self) -> &ServiceErrorKind {
        &self.kind
    }

    pub(crate) fn api(error: MetaError) -> ServiceErrorKind {
        ServiceErrorKind::Api(ApiError {
            error: Box::new(error),
        })
    }

    pub(crate) fn parse(source: BoxError, body: String) -> ServiceErrorKind {
        ServiceErrorKind::Parse(ParseError {
            source: Some(source),
            body,
        })
    }

    pub(crate) fn payload(source: BoxError) -> ServiceErrorKind {
        ServiceErrorKind::InvalidPayload(Some(source))
    }
}

/// This enum is a sub-category of `ServiceError`, providing more granular detail about
/// issues that arise from interacting with the WhatsApp Business API or processing
/// its responses.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum ServiceErrorKind {
    /// Represents an **API-specific error**, typically indicating a problem reported
    /// by Meta's API due to an invalid request, rate limiting, or server-side issue.
    ///
    /// This variant wraps an `ApiError` which contains the HTTP status code and
    /// [`MetaError`] details directly from the API response.
    #[error("The API returned an error: {0}")]
    Api(#[from] ApiError),

    /// Represents an error occurring during the **parsing or deserialization of API responses**
    /// or webhook payloads into expected Rust data structures.
    ///
    /// This variant wraps a `ParseError` which includes the original body that failed
    /// to parse and a source error.
    #[error("Failed to parse the API response or webhook payload: {0}")]
    Parse(#[from] ParseError),

    /// Indicates that a **webhook payload or API response contains invalid or unexpected data**
    /// that does not conform to the expected structure, even if it could be partially parsed.
    ///
    /// This suggests a mismatch between the expected data format and what was
    /// actually received, possibly due to API changes or malformed input.
    /// It includes an optional `BoxError` source for debugging.
    #[error("The webhook or API response had an invalid or unexpected payload structure.")]
    InvalidPayload(#[source] Option<BoxError>),
}

impl ServiceErrorKind {
    pub(crate) fn service(
        self,
        #[cfg(debug_assertions)] endpoint: impl Into<String>,
        status: StatusCode,
    ) -> ServiceError {
        ServiceError {
            status,
            kind: self,
            #[cfg(debug_assertions)]
            endpoint: Some(endpoint.into()),
        }
    }
}

/// Represents an **API-specific error with an HTTP status code and error details**.
///
/// This struct is used within `ServiceError::Api` to provide granular details about
/// errors reported directly by the WhatsApp Business API, as parsed into a [`MetaError`].
///
/// # Fields
/// - `error`: A boxed [`MetaError`] containing the detailed error information from Meta.
#[derive(thiserror::Error, Debug)]
#[error("Meta API error: {error}")]
#[non_exhaustive]
pub struct ApiError {
    pub error: Box<MetaError>,
}

/// Represents an error that occurred during **data parsing or deserialization**.
///
/// This struct is used when the crate fails to convert raw string or byte data
/// (e.g., JSON responses from the API, webhook payloads) into the expected
/// Rust data structures.
///
/// # Fields
/// - `source`: An optional `BoxError` representing the underlying cause of the
///   parsing failure (e.g., a `serde_json::Error`).
/// - `body`: The original raw `String` content that could not be parsed,
///   useful for debugging.
#[derive(thiserror::Error, Debug)]
#[error("Failed to parse the response body. Raw body content was: '{}'.", body)]
#[non_exhaustive]
pub struct ParseError {
    #[source]
    pub(crate) source: Option<BoxError>,
    pub body: String,
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        if value.is_builder() || value.is_redirect() {
            // Builder errors, redirection errors (which reqwest might auto-follow but can signal issues),
            // and request composition errors often point to internal misconfiguration or invalid input.
            Self::internal(value.into())
        } else {
            // filter redirect for service?
            Self::network(value.into())
        }
    }
}

/// A convenient type alias for a boxed, trait-object error that can be sent across threads.
///
/// This is typically used to erase the concrete type of an error when it needs to be
/// stored or passed up the call stack generically.
pub type BoxError = Box<dyn StdError + Send + Sync>;
