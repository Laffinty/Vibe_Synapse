//! Core module: event system, plugin system, and context

pub mod event;
pub mod plugin;
pub mod context;

pub use event::{Event, EventBus, EventWrapper, Priority, EventHandler, handler, Request, BroadcastEvent};
pub use plugin::{Plugin, PluginMetadata, PluginState, PluginInstance, LifecyclePlugin};
pub use context::{Context, ContextBuilder, ContextExt, ScopedContext};
pub use crate::impl_event;
