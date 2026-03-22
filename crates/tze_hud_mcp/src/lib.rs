//! # tze_hud_mcp
//!
//! MCP (Model Context Protocol) compatibility bridge for tze_hud.
//!
//! Implements the compatibility plane: a JSON-RPC 2.0 server that exposes
//! named tools for LLM interaction. This is intentionally NOT the hot path —
//! JSON overhead is acceptable here.
//!
//! ## Architecture
//!
//! The MCP bridge wraps a shared [`SceneGraph`] behind a mutex and translates
//! JSON-RPC tool calls into scene graph mutations. It bridges to the gRPC
//! control plane in spirit (same scene model) but speaks JSON-RPC for maximum
//! LLM compatibility.
//!
//! ## Tools Exposed
//!
//! | Tool              | Description                              |
//! |-------------------|------------------------------------------|
//! | `create_tab`      | Create a new tab in the scene            |
//! | `create_tile`     | Create a tile within a tab               |
//! | `set_content`     | Set markdown content on a tile's node    |
//! | `publish_to_zone` | Publish content to a named zone          |
//! | `list_zones`      | List available zones and their state     |

pub mod error;
pub mod server;
pub mod tools;
pub mod types;

pub use error::McpError;
pub use server::McpServer;
pub use types::{McpRequest, McpResponse, McpResult};
