//! Ergonomic macros for fire-and-forget event logging.
//!
//! These macros provide a clean, zero-overhead interface to the runtime.
//! They only allocate if the runtime is initialized and match Rust's
//! standard logging conventions. Each macro respects the runtime's
//! log level setting, so you can globally control which events are recorded.
//!
//! # Example
//!
//! ```rust
//! # use eventline::runtime;
//! # use eventline::{event_info, event_warn, event_error, event_debug, event_scope_async};
//! # tokio_test::block_on(async {
//! # runtime::init().await;
//! # let cache_size = 256;
//! # let err = "connection failed";
//! # let id = 42;
//! event_info!("Server started on port {}", 8080).await;
//! event_warn!("Cache size: {} MB", cache_size).await;
//! event_error!("Failed to connect: {}", err).await;
//! event_debug!("Processing request {}", id).await;
//!
//! event_scope_async!("RequestHandler", {
//!     event_info!("Handling request").await;
//!     // ... work happens here ...
//! }).await;
//! # runtime::reset().await;
//! # });
//! ```

pub mod event;
pub mod scope;
pub mod tests;

