//! Runtime module: application and plugin management

pub mod app;
pub mod plugin_manager;

pub use app::{VibeApp, AppBuilder, AppConfig};
pub use plugin_manager::{PluginManager, PluginManagerConfig, PluginLoadOrder};
