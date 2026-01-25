//! Plugin manager

use crate::core::{Context, EventBus, LifecyclePlugin, Plugin, PluginMetadata, PluginState};
use crate::error::{Result, VibeError};
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use std::time::Duration;
use tokio::time::timeout;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluginLoadOrder {
    Sequential,
    Parallel,
    #[default]
    DependencyFirst,
}

#[derive(Debug, Clone)]
pub struct PluginManagerConfig {
    pub timeout: Duration,
    pub load_order: PluginLoadOrder,
    pub enable_dependency_check: bool,
    pub continue_on_failure: bool,
    pub max_retries: u32,
}

impl Default for PluginManagerConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            load_order: PluginLoadOrder::default(),
            enable_dependency_check: true,
            continue_on_failure: false,
            max_retries: 3,
        }
    }
}

impl PluginManagerConfig {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    
    pub fn with_load_order(mut self, order: PluginLoadOrder) -> Self {
        self.load_order = order;
        self
    }
    
    pub fn enable_dependency_check(mut self, enable: bool) -> Self {
        self.enable_dependency_check = enable;
        self
    }
    
    pub fn continue_on_failure(mut self, continue_on_failure: bool) -> Self {
        self.continue_on_failure = continue_on_failure;
        self
    }
    
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }
}

pub struct PluginManager {
    plugins: Arc<RwLock<HashMap<String, RwLock<LifecyclePlugin>>>>,
    startup_order: Arc<RwLock<Vec<String>>>,
    config: PluginManagerConfig,
    event_bus: Arc<EventBus>,
    is_started: Arc<RwLock<bool>>,
}

impl PluginManager {
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            startup_order: Arc::new(RwLock::new(Vec::new())),
            config: PluginManagerConfig::default(),
            event_bus,
            is_started: Arc::new(RwLock::new(false)),
        }
    }
    
    pub fn with_config(event_bus: Arc<EventBus>, config: PluginManagerConfig) -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            startup_order: Arc::new(RwLock::new(Vec::new())),
            config,
            event_bus,
            is_started: Arc::new(RwLock::new(false)),
        }
    }
    
    pub async fn add_plugin<P: Plugin + 'static>(&self, plugin: P) -> Result<()> {
        let metadata = plugin.metadata();
        let plugin_name = metadata.name.clone();
        
        if self.plugins.read().contains_key(&plugin_name) {
            return Err(VibeError::PluginLoadError(format!(
                "Plugin {} already exists",
                plugin_name
            )));
        }
        
        let lifecycle = LifecyclePlugin::new(
            Box::new(plugin),
            metadata.dependencies.clone(),
        );
        
        self.load_plugin(&plugin_name, lifecycle).await?;
        Ok(())
    }
    
    async fn load_plugin(&self, name: &str, mut lifecycle: LifecyclePlugin) -> Result<()> {
        let plugin_name = name.to_string();
        let ctx = Context::new(self.event_bus.clone())
            .with_plugin(lifecycle.instance.metadata.clone());
        
        let mut plugin = lifecycle.instance.plugin;
        let result = timeout(
            self.config.timeout,
            plugin.load(&mut ctx.clone().into())
        )
        .await;
        
        match result {
            Ok(Ok(())) => {
                lifecycle.instance.plugin = plugin;
                self.plugins
                    .write()
                    .insert(plugin_name.clone(), RwLock::new(lifecycle));
                Ok(())
            }
            Ok(Err(e)) => {
                Err(VibeError::PluginLoadError(format!(
                    "Plugin {} failed to load: {}",
                    plugin_name, e
                )))
            }
            Err(_) => {
                Err(VibeError::PluginLoadError(format!(
                    "Plugin {} load timeout after {:?}",
                    plugin_name, self.config.timeout
                )))
            }
        }
    }
    
    pub async fn start_all(&self) -> Result<()> {
        let mut started = self.is_started.write();
        if *started {
            return Err(VibeError::RuntimeError("Plugins already started".to_string()));
        }
        
        self.resolve_dependencies().await?;
        
        let order = self.config.load_order;
        match order {
            PluginLoadOrder::Sequential => self.start_sequential().await?,
            PluginLoadOrder::Parallel => self.start_parallel().await?,
            PluginLoadOrder::DependencyFirst => self.start_dependency_first().await?,
        }
        
        *started = true;
        Ok(())
    }
    
    async fn resolve_dependencies(&self) -> Result<Vec<String>> {
        let plugins = self.plugins.read();
        let mut resolved = Vec::new();
        let mut unresolved = plugins.keys().cloned().collect::<Vec<_>>();
        
        let mut attempts = 0;
        let max_attempts = unresolved.len() * 2;
        
        while !unresolved.is_empty() && attempts < max_attempts {
            attempts += 1;
            let mut remaining = Vec::new();
            
            for plugin_name in unresolved {
                let plugin = plugins.get(&plugin_name).unwrap().read();
                
                let all_deps_resolved = plugin.dependencies.iter()
                    .all(|dep| resolved.contains(dep));
                
                if all_deps_resolved {
                    resolved.push(plugin_name);
                } else {
                    remaining.push(plugin_name);
                }
            }
            
            unresolved = remaining;
        }
        
        if !unresolved.is_empty() {
            return Err(VibeError::PluginInitError(
                format!("Circular dependency detected: {:?}", unresolved)
            ));
        }
        
        let mut startup_order = self.startup_order.write();
        *startup_order = resolved.clone();
        
        Ok(resolved)
    }
    
    async fn start_dependency_first(&self) -> Result<()> {
        let startup_order = self.startup_order.read().clone();
        let mut errors = Vec::new();
        
        for plugin_name in startup_order {
            if let Err(e) = self.start_plugin(&plugin_name).await {
                errors.push((plugin_name.clone(), e));
                
                if !self.config.continue_on_failure {
                    return Err(VibeError::PluginInitError(
                        format!("Failed to start plugin {}: {}", plugin_name, errors.last().unwrap().1)
                    ));
                }
            }
        }
        
        Ok(())
    }
    
    async fn start_sequential(&self) -> Result<()> {
        let plugin_names: Vec<String> = self.plugins.read().keys().cloned().collect();
        let mut errors = Vec::new();
        
        for name in plugin_names {
            if let Err(e) = self.start_plugin(&name).await {
                errors.push((name.clone(), e));
                
                if !self.config.continue_on_failure {
                    return Err(VibeError::PluginInitError(
                        format!("Failed to start plugin {}: {}", name, errors.last().unwrap().1)
                    ));
                }
            }
        }
        
        Ok(())
    }
    
    async fn start_parallel(&self) -> Result<()> {
        let plugin_names: Vec<String> = self.plugins.read().keys().cloned().collect();
        let mut errors = Vec::new();
        
        for name in plugin_names {
            if let Err(e) = self.start_plugin(&name).await {
                errors.push(e);
                
                if !self.config.continue_on_failure {
                    return Err(VibeError::PluginInitError(
                        format!("Failed to start plugin {}: {}", name, errors.last().unwrap())
                    ));
                }
            }
        }
        
        Ok(())
    }
    
    async fn start_plugin(&self, name: &str) -> Result<()> {
        let plugin_map = self.plugins.read();
        let plugin_lock = plugin_map
            .get(name)
            .ok_or_else(|| VibeError::PluginNotFound(name.to_string()))?;
        
        let mut plugin = plugin_lock.write();
        let plugin_name = plugin.instance.metadata.name.clone();
        let ctx = Context::new(self.event_bus.clone())
            .with_plugin(plugin.instance.metadata.clone());
        
        let plugin_ref = &mut *plugin;
        let result = timeout(
            self.config.timeout,
            plugin_ref.instance.plugin.start(&ctx)
        )
        .await;
        
        match result {
            Ok(Ok(())) => {
                Ok(())
            }
            Ok(Err(e)) => {
                plugin.instance.state = PluginState::Failed;
                Err(VibeError::PluginInitError(format!(
                    "Plugin {} failed to start: {}",
                    plugin_name, e
                )))
            }
            Err(_) => {
                plugin.instance.state = PluginState::Failed;
                Err(VibeError::PluginInitError(format!(
                    "Plugin {} start timeout after {:?}",
                    plugin_name, self.config.timeout
                )))
            }
        }
    }
    
    pub async fn stop_all(&self) -> Result<()> {
        let mut started = self.is_started.write();
        if !*started {
            return Ok(());
        }
        
        let startup_order = self.startup_order.read();
        let mut errors = Vec::new();
        
        for plugin_name in startup_order.iter().rev() {
            if let Err(e) = self.stop_plugin(plugin_name).await {
                errors.push((plugin_name.clone(), e));
            }
        }
        
        *started = false;
        Ok(())
    }
    
    async fn stop_plugin(&self, name: &str) -> Result<()> {
        let plugin_map = self.plugins.read();
        if let Some(plugin_lock) = plugin_map.get(name) {
            let mut plugin = plugin_lock.write();
            
            if plugin.instance.state == PluginState::Running {
                let ctx = Context::new(self.event_bus.clone())
                    .with_plugin(plugin.instance.metadata.clone());
                
                match timeout(
                    self.config.timeout,
                    plugin.instance.plugin.stop(&ctx)
                )
                .await
                {
                    Ok(Ok(())) => {
                        plugin.instance.state = PluginState::Stopped;
                    }
                    Ok(Err(e)) => {
                        plugin.instance.state = PluginState::Failed;
                        return Err(e);
                    }
                    Err(_) => {
                        plugin.instance.state = PluginState::Failed;
                        return Err(VibeError::TimeoutError(
                            format!("Plugin {} stop timeout", name)
                        ));
                    }
                }
            }
        }
        
        Ok(())
    }
    
    pub fn list_plugins(&self) -> Vec<(String, PluginState)> {
        self.plugins
            .read()
            .iter()
            .map(|(name, plugin)| (name.clone(), plugin.read().instance.state))
            .collect()
    }
    
    pub fn get_plugin_metadata(&self, name: &str) -> Option<PluginMetadata> {
        self.plugins
            .read()
            .get(name)
            .map(|plugin| plugin.read().instance.metadata.clone())
    }
    
    pub fn has_plugin(&self, name: &str) -> bool {
        self.plugins.read().contains_key(name)
    }
    
    pub fn plugin_count(&self) -> usize {
        self.plugins.read().len()
    }
    
    pub fn is_started(&self) -> bool {
        *self.is_started.read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event::EventBus;
    use async_trait::async_trait;
    
    struct TestPlugin;
    
    #[async_trait]
    impl Plugin for TestPlugin {
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata::new("test", "1.0.0")
        }
    }
    
    #[tokio::test]
    async fn test_plugin_manager_creation() {
        let bus = Arc::new(EventBus::new());
        let manager = PluginManager::new(bus);
        
        assert_eq!(manager.plugin_count(), 0);
        assert!(!manager.is_started());
    }
    
    #[tokio::test]
    async fn test_add_plugin() {
        let bus = Arc::new(EventBus::new());
        let bus_clone = Arc::clone(&bus);
        let manager = PluginManager::new(bus_clone);
        
        let plugin = TestPlugin;
        manager.add_plugin(plugin).await.unwrap();
        
        assert_eq!(manager.plugin_count(), 1);
        assert!(manager.has_plugin("test"));
    }
    
    #[tokio::test]
    async fn test_duplicate_plugin() {
        let bus = Arc::new(EventBus::new());
        let manager = PluginManager::new(bus);
        
        let plugin1 = TestPlugin;
        let plugin2 = TestPlugin;
        
        manager.add_plugin(plugin1).await.unwrap();
        let result = manager.add_plugin(plugin2).await;
        
        assert!(result.is_err());
        assert_eq!(manager.plugin_count(), 1);
    }
}
