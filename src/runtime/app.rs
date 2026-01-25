//! Application runtime

use crate::core::{Context, EventBus};
use crate::runtime::plugin_manager::{PluginManager, PluginManagerConfig};
use crate::error::{Result, VibeError};
use std::sync::Arc;
use tokio::time::{sleep, Duration, timeout};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub name: String,
    pub version: String,
    pub description: String,
    pub plugin_config: PluginManagerConfig,
    pub graceful_shutdown_timeout: Duration,
    pub enable_signal_handling: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            name: "VibeSynapseApp".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            description: String::new(),
            plugin_config: PluginManagerConfig::default(),
            graceful_shutdown_timeout: Duration::from_secs(30),
            enable_signal_handling: true,
        }
    }
}

impl AppConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }
    
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }
    
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }
    
    pub fn with_plugin_config(mut self, config: PluginManagerConfig) -> Self {
        self.plugin_config = config;
        self
    }
    
    pub fn with_graceful_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.graceful_shutdown_timeout = timeout;
        self
    }
    
    pub fn with_signal_handling(mut self, enable: bool) -> Self {
        self.enable_signal_handling = enable;
        self
    }
}

pub struct AppBuilder {
    config: Option<AppConfig>,
    plugins_to_add: Vec<Box<dyn crate::core::Plugin>>,
    event_bus: Option<Arc<EventBus>>,
}

impl AppBuilder {
    pub fn new() -> Self {
        Self {
            config: None,
            plugins_to_add: Vec::new(),
            event_bus: None,
        }
    }
    
    pub fn with_config(mut self, config: AppConfig) -> Self {
        self.config = Some(config);
        self
    }
    
    pub fn add_plugin<P: crate::core::Plugin + 'static>(mut self, plugin: P) -> Self {
        self.plugins_to_add.push(Box::new(plugin));
        self
    }
    
    pub fn add_plugins<I>(mut self, plugins: I) -> Self
    where
        I: IntoIterator<Item = Box<dyn crate::core::Plugin>>,
    {
        self.plugins_to_add.extend(plugins);
        self
    }
    
    pub fn with_event_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(bus);
        self
    }
    
    pub async fn build(mut self) -> Result<VibeApp> {
        let config = self.config.take().unwrap_or_default();
        let event_bus = self.event_bus.unwrap_or_else(|| Arc::new(EventBus::new()));
        
        let plugin_manager = PluginManager::with_config(
            Arc::clone(&event_bus),
            config.plugin_config.clone(),
        );
        
        for plugin in self.plugins_to_add {
            plugin_manager.add_plugin(plugin).await?;
        }
        
        Ok(VibeApp {
            config,
            event_bus,
            plugin_manager,
            is_running: Arc::new(tokio::sync::RwLock::new(false)),
        })
    }
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct VibeApp {
    config: AppConfig,
    event_bus: Arc<EventBus>,
    plugin_manager: PluginManager,
    is_running: Arc<tokio::sync::RwLock<bool>>,
}

impl VibeApp {
    pub fn build() -> AppBuilder {
        AppBuilder::new()
    }
    
    pub async fn run(&self) -> Result<()> {
        let mut running = self.is_running.write().await;
        if *running {
            return Err(VibeError::RuntimeError("Application already running".to_string()));
        }
        
        self.event_bus.start().await?;
        self.plugin_manager.start_all().await?;
        *running = true;
        
        if self.config.enable_signal_handling {
            self.wait_for_shutdown().await?;
        }
        
        Ok(())
    }
    
    async fn wait_for_shutdown(&self) -> Result<()> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            
            let mut sigint = signal(SignalKind::interrupt())?;
            let mut sigterm = signal(SignalKind::terminate())?;
            
            tokio::select! {
                _ = sigint.recv() => {}
                _ = sigterm.recv() => {}
            }
        }
        
        #[cfg(windows)]
        {
            use tokio::signal::ctrl_c;
            
            ctrl_c().await.map_err(|e| VibeError::RuntimeError(format!("Signal error: {}", e)))?;
        }
        
        self.shutdown().await?;
        Ok(())
    }
    
    pub async fn shutdown(&self) -> Result<()> {
        {
            let running = self.is_running.read().await;
            if !*running {
                return Ok(());
            }
        }
        
        self.plugin_manager.stop_all().await?;
        self.event_bus.stop().await?;
        
        let mut running = self.is_running.write().await;
        *running = false;
        
        Ok(())
    }
    
    pub async fn force_shutdown(&self) -> Result<()> {
        match timeout(
            self.config.graceful_shutdown_timeout,
            self.shutdown(),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                Err(VibeError::TimeoutError("Shutdown timeout".to_string()))
            }
        }
    }
    
    pub fn context(&self) -> Context {
        Context::new(Arc::clone(&self.event_bus))
    }
    
    pub fn config(&self) -> &AppConfig {
        &self.config
    }
    
    pub fn plugin_manager(&self) -> &PluginManager {
        &self.plugin_manager
    }
    
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }
    
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }
    
    pub async fn wait_until_started(&self, timeout: Duration) -> Result<()> {
        let start = std::time::Instant::now();
        
        while start.elapsed() < timeout {
            if *self.is_running.read().await {
                return Ok(());
            }
            sleep(Duration::from_millis(10)).await;
        }
        
        Err(VibeError::TimeoutError("Application failed to start within timeout".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Event, PluginMetadata};
    use async_trait::async_trait;
    
    struct TestPlugin;
    
    #[async_trait]
    impl crate::core::Plugin for TestPlugin {
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata::new("test", "1.0.0")
        }
    }
    
    #[derive(Debug)]
    struct TestEvent;
    
    impl Event for TestEvent {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    
    #[tokio::test]
    async fn test_app_builder() {
        let app = VibeApp::build()
            .with_config(AppConfig::new("test-app"))
            .add_plugin(TestPlugin)
            .build()
            .await
            .unwrap();
        
        assert_eq!(app.config().name, "test-app");
        assert_eq!(app.plugin_manager().plugin_count(), 1);
    }
    
    #[tokio::test]
    async fn test_app_context() {
        let app = VibeApp::build().build().await.unwrap();
        let ctx = app.context();
        
        ctx.publish(TestEvent).unwrap();
        assert!(app.event_bus().processed_events() > 0);
    }
    
    #[test]
    fn test_app_config_builder() {
        let config = AppConfig::new("my-app")
            .with_version("1.0.0")
            .with_description("A test app")
            .with_signal_handling(false);
        
        assert_eq!(config.name, "my-app");
        assert_eq!(config.version, "1.0.0");
        assert_eq!(config.description, "A test app");
        assert!(!config.enable_signal_handling);
    }
}
