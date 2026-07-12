pub mod error;
pub mod transport;
pub mod registry;
pub mod tools;

pub use error::{McpError, Result};
pub use transport::JsonRpcTransport;
pub use registry::ToolRegistry;
