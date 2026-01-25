//! Plugin system

use async_trait::async_trait;
use std::any::Any;
use crate::core::context::Context;
use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    Loaded,
    Initializing,
    Running,
    Stopped,
    Failed,
}

#[derive(Debug, Clone)]
pub struct PluginMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub dependencies: Vec<String>,
}

impl PluginMetadata {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            description: String::new(),
            author: String::new(),
            dependencies: Vec::new(),
        }
    }
    
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }
    
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = author.into();
        self
    }
    
    pub fn with_dependency(mut self, dep: impl Into<String>) -> Self {
        self.dependencies.push(dep.into());
        self
    }
    
    pub fn with_dependencies(mut self, deps: Vec<impl Into<String>>) -> Self {
        self.dependencies.extend(deps.into_iter().map(Into::into));
        self
    }
}

#[async_trait]
impl Plugin for Box<dyn Plugin> {
    fn metadata(&self) -> PluginMetadata {
        (**self).metadata()
    }
    
    async fn load(&mut self, ctx: &mut Context) -> Result<()> {
        (**self).load(ctx).await
    }
    
    async fn start(&mut self, ctx: &Context) -> Result<()> {
        (**self).start(ctx).await
    }
    
    async fn stop(&mut self, ctx: &Context) -> Result<()> {
        (**self).stop(ctx).await
    }
    
    async fn unload(&mut self, ctx: &Context) -> Result<()> {
        (**self).unload(ctx).await
    }
    
    fn state(&self) -> PluginState {
        (**self).state()
    }
    
    fn name(&self) -> String {
        (**self).name()
    }
    
    fn version(&self) -> String {
        (**self).version()
    }
}

/// Plugin trait
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("unknown", "0.1.0")
    }
    
    async fn load(&mut self, _ctx: &mut Context) -> Result<()> {
        Ok(())
    }
    
    async fn start(&mut self, _ctx: &Context) -> Result<()> {
        Ok(())
    }
    
    async fn stop(&mut self, _ctx: &Context) -> Result<()> {
        Ok(())
    }
    
    async fn unload(&mut self, _ctx: &Context) -> Result<()> {
        Ok(())
    }
    
    fn state(&self) -> PluginState {
        PluginState::Loaded
    }
    
    fn name(&self) -> String {
        self.metadata().name
    }
    
    fn version(&self) -> String {
        self.metadata().version
    }
}

/// Function plugin wrapper
pub struct FunctionPlugin<F> {
    metadata: PluginMetadata,
    func: F,
}

impl<F> FunctionPlugin<F> {
    pub fn new(metadata: PluginMetadata, func: F) -> Self {
        Self { metadata, func }
    }
}

#[async_trait]
impl<F, Fut> Plugin for FunctionPlugin<F>
where
    F: Fn(&mut Context) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<()>> + Send,
{
    fn metadata(&self) -> PluginMetadata {
        self.metadata.clone()
    }
    
    async fn load(&mut self, ctx: &mut Context) -> Result<()> {
        (self.func)(ctx).await
    }
    
    async fn start(&mut self, _ctx: &Context) -> Result<()> {
        Ok(())
    }
}

#[macro_export]
macro_rules! plugin_fn {
    ($name:expr, $version:expr, $func:expr) => {{
        use $crate::core::plugin::{FunctionPlugin, PluginMetadata};
        let metadata = PluginMetadata::new($name, $version);
        FunctionPlugin::new(metadata, $func)
    }};
    ($metadata:expr, $func:expr) => {{
        use $crate::core::plugin::FunctionPlugin;
        FunctionPlugin::new($metadata, $func)
    }};
}

pub trait PluginFactory: Send + Sync {
    fn create(&self) -> Box<dyn Plugin>;
    fn metadata(&self) -> PluginMetadata;
}

pub struct PluginInstance {
    pub plugin: Box<dyn Plugin>,
    pub metadata: PluginMetadata,
    pub state: PluginState,
}

impl PluginInstance {
    pub fn new(plugin: Box<dyn Plugin>) -> Self {
        let metadata = Plugin::metadata(&plugin);
        Self {
            plugin,
            metadata,
            state: PluginState::Loaded,
        }
    }
    
    pub fn name(&self) -> &str {
        &self.metadata.name
    }
    
    pub fn is_running(&self) -> bool {
        matches!(self.state, PluginState::Running)
    }
    
    pub fn to_meta(&self) -> (String, PluginState) {
        (self.metadata.name.clone(), self.state)
    }
}

pub trait PluginBox: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn metadata(&self) -> PluginMetadata;
}

impl<T: Plugin> PluginBox for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    
    fn metadata(&self) -> PluginMetadata {
        Plugin::metadata(self)
    }
}

/// Lifecycle plugin wrapper
pub struct LifecyclePlugin {
    pub instance: PluginInstance,
    pub dependencies: Vec<String>,
    pub ready: bool,
}

impl LifecyclePlugin {
    pub fn new(plugin: Box<dyn Plugin>, dependencies: Vec<String>) -> Self {
        Self {
            instance: PluginInstance::new(plugin),
            dependencies,
            ready: false,
        }
    }
    
    pub fn check_dependencies(&self, loaded_plugins: &[String]) -> bool {
        self.dependencies.iter().all(|dep| loaded_plugins.contains(dep))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    struct TestPlugin;
    
    #[async_trait]
    impl Plugin for TestPlugin {
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata::new("test", "1.0.0")
                .with_description("A test plugin")
                .with_author("Test")
        }
    }
    
    #[test]
    fn test_plugin_metadata_builder() {
        let meta = PluginMetadata::new("test", "1.0.0")
            .with_description("Test plugin")
            .with_author("Test Author")
            .with_dependency("dep1")
            .with_dependencies(vec!["dep2", "dep3"]);
        
        assert_eq!(meta.name, "test");
        assert_eq!(meta.version, "1.0.0");
        assert_eq!(meta.description, "Test plugin");
        assert_eq!(meta.author, "Test Author");
        assert_eq!(meta.dependencies.len(), 3);
        assert!(meta.dependencies.contains(&"dep1".to_string()));
    }
    
    #[test]
    fn test_plugin_instance() {
        let plugin = Box::new(TestPlugin);
        let instance = PluginInstance::new(plugin);
        
        assert_eq!(instance.name(), "test");
        assert_eq!(instance.state, PluginState::Loaded);
        assert!(!instance.is_running());
    }
    
    #[test]
    fn test_lifecycle_plugin() {
        let plugin = Box::new(TestPlugin);
        let lifecycle = LifecyclePlugin::new(plugin, vec!["dep1".to_string()]);
        
        assert_eq!(lifecycle.instance.name(), "test");
        assert_eq!(lifecycle.dependencies.len(), 1);
        assert!(!lifecycle.ready);
        assert!(lifecycle.check_dependencies(&["dep1".to_string()]));
        assert!(!lifecycle.check_dependencies(&["dep2".to_string()]));
    }
}
