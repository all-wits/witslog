pub mod error;
pub mod registry;
pub mod schema;
pub mod server;
pub mod tools;
pub mod transport;

pub use error::{McpError, Result};
pub use registry::ToolRegistry;
pub use server::{serve_stdio, ServerConfig};
pub use transport::JsonRpcTransport;
