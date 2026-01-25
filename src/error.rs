//! Error handling system

use thiserror::Error;

#[derive(Error, Debug)]
pub enum VibeError {
    #[error("Plugin load failed: {0}")]
    PluginLoadError(String),
    
    #[error("Plugin initialization failed: {0}")]
    PluginInitError(String),
    
    #[error("Plugin not found: {0}")]
    PluginNotFound(String),
    
    #[error("Event processing failed: {0}")]
    EventError(String),
    
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),
    
    #[error("Runtime error: {0}")]
    RuntimeError(String),
    
    #[error("Subscription error: {0}")]
    SubscriptionError(String),
    
    #[error("Operation timeout: {0}")]
    TimeoutError(String),
    
    #[error("{0}")]
    Other(String),
    
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
    
    #[error("IO error: {0}")]
    IoError(String),
}

pub type Result<T> = std::result::Result<T, VibeError>;

pub trait ContextExt<T> {
    fn with_plugin(self, plugin_name: &str) -> Result<T>;
    fn with_event(self, event_type: &str) -> Result<T>;
    fn with_resource(self, resource_type: &str) -> Result<T>;
}

impl<T> ContextExt<T> for Result<T> {
    fn with_plugin(self, plugin_name: &str) -> Result<T> {
        self.map_err(|e| match e {
            VibeError::PluginLoadError(msg) => VibeError::PluginLoadError(format!("{} [plugin: {}]", msg, plugin_name)),
            VibeError::PluginInitError(msg) => VibeError::PluginInitError(format!("{} [plugin: {}]", msg, plugin_name)),
            other => VibeError::PluginLoadError(format!("{} [plugin: {}]", other, plugin_name)),
        })
    }
    
    fn with_event(self, event_type: &str) -> Result<T> {
        self.map_err(|e| match e {
            VibeError::EventError(msg) => VibeError::EventError(format!("{} [event: {}]", msg, event_type)),
            other => VibeError::EventError(format!("{} [event: {}]", other, event_type)),
        })
    }
    
    fn with_resource(self, resource_type: &str) -> Result<T> {
        self.map_err(|e| match e {
            VibeError::ResourceNotFound(msg) => VibeError::ResourceNotFound(format!("{} [resource: {}]", msg, resource_type)),
            other => VibeError::ResourceNotFound(format!("{} [resource: {}]", other, resource_type)),
        })
    }
}

impl From<std::io::Error> for VibeError {
    fn from(error: std::io::Error) -> Self {
        VibeError::IoError(error.to_string())
    }
}

#[macro_export]
macro_rules! bail {
    ($msg:literal) => {
        return Err($crate::error::VibeError::Other($msg.to_string()))
    };
    ($err:expr) => {
        return Err($crate::error::VibeError::Other($err.to_string()))
    };
    ($fmt:expr, $($arg:tt)*) => {
        return Err($crate::error::VibeError::Other(format!($fmt, $($arg)*)))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_display() {
        let err = VibeError::PluginLoadError("test error".to_string());
        assert_eq!(err.to_string(), "Plugin load failed: test error");
    }
    
    #[test]
    fn test_context_ext() {
        let err: Result<()> = Err(VibeError::PluginLoadError("base error".to_string()));
        let err = err.with_plugin("test_plugin");
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("test_plugin"));
    }
    
    #[test]
    fn test_bail_macro() {
        fn test_fn() -> Result<()> {
            bail!("test bail message");
        }
        let result = test_fn();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "test bail message");
    }
}
