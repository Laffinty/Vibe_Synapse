//! Vibe Synapse - A plugin-first Rust framework
//! 
//! Lightweight, high-performance framework with plugin architecture.

#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod core;
pub mod runtime;
pub mod error;

pub mod prelude {
    //! Commonly used types and traits
    
    pub use crate::core::{Context, Event, EventBus, EventWrapper, Priority, Plugin};
    pub use crate::core::{PluginMetadata, PluginState};
    pub use crate::core::{handler, Request, BroadcastEvent};
    pub use crate::runtime::{VibeApp, AppBuilder, AppConfig};
    pub use crate::runtime::{PluginManager, PluginManagerConfig, PluginLoadOrder};
    pub use crate::error::{Result, VibeError, ContextExt};
    pub use crate::impl_event;
    pub use async_trait::async_trait;
}

pub use error::{Result, VibeError};
pub use core::{Context, Event, Plugin};
pub use runtime::VibeApp;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");

pub fn build_info() -> String {
    format!("Vibe Synapse {}", VERSION)
}

pub fn runtime_check() -> Result<()> {
    if tokio::runtime::Handle::try_current().is_err() {
        return Err(VibeError::RuntimeError(
            "No Tokio runtime found. Use #[tokio::main]".to_string()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_version() {
        assert!(!VERSION.is_empty());
    }
    
    #[test]
    fn test_authors() {
        assert_eq!(AUTHORS, env!("CARGO_PKG_AUTHORS"));
    }
    
    #[test]
    fn test_build_info() {
        let info = build_info();
        assert!(info.contains("Vibe Synapse"));
    }
    
    #[tokio::test]
    async fn test_prelude_imports() {
        use prelude::*;
        
        let bus = EventBus::new();
        let _ctx = Context::new(std::sync::Arc::new(bus));
        
        assert!(true);
    }
}
