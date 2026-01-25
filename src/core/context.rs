//! Context system for resource management

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use crate::core::event::EventBus;
use crate::core::plugin::PluginMetadata;
use crate::error::{Result, VibeError};

type ResourceFactory = Box<dyn Fn() -> Result<Box<dyn Any + Send + Sync>> + Send + Sync>;

/// Context for plugin-framework interaction
#[derive(Clone)]
pub struct Context {
    resources: Arc<RwLock<ResourceRegistry>>,
    event_bus: Arc<EventBus>,
    plugin_meta: Option<Arc<PluginMetadata>>,
}

impl Context {
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            resources: Arc::new(RwLock::new(ResourceRegistry::new())),
            event_bus,
            plugin_meta: None,
        }
    }
    
    pub fn with_plugin(self, plugin_meta: PluginMetadata) -> Self {
        Self {
            plugin_meta: Some(Arc::new(plugin_meta)),
            ..self
        }
    }
    
    pub fn insert<T: Send + Sync + 'static>(&self, resource: T) -> Result<()> {
        let mut registry = self.resources.write();
        registry.insert(resource)?;
        Ok(())
    }
    
    pub fn insert_lazy<T, F>(&self, factory: F) -> Result<()>
    where
        T: Send + Sync + 'static,
        F: Fn() -> Result<T> + Send + Sync + 'static,
    {
        let mut registry = self.resources.write();
        registry.insert_lazy(factory)?;
        Ok(())
    }
    
    pub fn get<T: 'static + Send + Sync, R, F>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&T) -> R,
    {
        let mut registry = self.resources.write();
        registry.get::<T>().map(|arc| f(&*arc))
    }
    
    pub fn get_mut<T: 'static + Send + Sync, R, F>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&T) -> R,
    {
        let mut registry = self.resources.write();
        registry.get_mut::<T>().map(|arc| f(&*arc))
    }
    
    pub fn contains<T: 'static>(&self) -> bool {
        let registry = self.resources.read();
        registry.contains::<T>()
    }
    
    pub fn remove<T: 'static + Send + Sync>(&self) -> Option<Arc<T>> {
        let mut registry = self.resources.write();
        registry.remove::<T>()
    }
    
    pub fn publish<E: crate::core::event::Event>(&self, event: E) -> Result<()> {
        self.event_bus.publish(event)?;
        Ok(())
    }
    
    pub fn publish_with_priority<E: crate::core::event::Event>(
        &self,
        event: E,
        priority: crate::core::event::Priority,
    ) -> Result<()> {
        self.event_bus.publish_with_priority(event, priority)
    }
    
    pub async fn subscribe<H, E>(&self, handler: H) -> Result<()>
    where
        H: crate::core::event::EventHandler + 'static,
        E: crate::core::event::Event + 'static,
    {
        use std::any::TypeId;
        self.event_bus.subscribe_handler(handler, TypeId::of::<E>()).await
    }
    
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }
}

/// Type-safe DI container
pub struct ResourceRegistry {
    resources: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    lazy_factories: HashMap<TypeId, ResourceFactory>,
    lazy_initialized: HashMap<TypeId, bool>,
}

impl ResourceRegistry {
    fn new() -> Self {
        Self {
            resources: HashMap::new(),
            lazy_factories: HashMap::new(),
            lazy_initialized: HashMap::new(),
        }
    }
    
    fn insert<T: Send + Sync + 'static>(&mut self, resource: T) -> Result<()> {
        let type_id = TypeId::of::<T>();
        self.resources.insert(type_id, Arc::new(resource));
        self.lazy_initialized.insert(type_id, true);
        Ok(())
    }
    
    fn insert_lazy<T, F>(&mut self, factory: F) -> Result<()>
    where
        T: Send + Sync + 'static,
        F: Fn() -> Result<T> + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        let factory: ResourceFactory = Box::new(move || {
            let resource = factory()?;
            Ok(Box::new(resource))
        });
        
        self.lazy_factories.insert(type_id, factory);
        self.lazy_initialized.insert(type_id, false);
        Ok(())
    }
    
    fn get<T: 'static + Send + Sync>(&mut self) -> Option<Arc<T>> {
        let type_id = TypeId::of::<T>();
        
        if let Some(&initialized) = self.lazy_initialized.get(&type_id) {
            if !initialized {
                if let Err(_) = self.initialize_lazy::<T>() {
                    return None;
                }
            }
        }
        
        self.resources
            .get(&type_id)
            .and_then(|arc| arc.clone().downcast::<T>().ok())
    }
    
    fn get_mut<T: 'static + Send + Sync>(&mut self) -> Option<Arc<T>> {
        self.get::<T>()
    }
    
    fn initialize_lazy<T: 'static + Send + Sync>(&mut self) -> Result<()> {
        let type_id = TypeId::of::<T>();
        
        if let Some(factory) = self.lazy_factories.remove(&type_id) {
            let boxed_resource = factory().map_err(|e| VibeError::Other(format!("Factory error: {}", e)))?;
            
            if let Ok(resource) = boxed_resource.downcast::<T>() {
                self.resources.insert(type_id, Arc::new(*resource));
                self.lazy_initialized.insert(type_id, true);
                Ok(())
            } else {
                Err(VibeError::ResourceNotFound(
                    format!("Failed to downcast lazy initialized resource: {}", std::any::type_name::<T>())
                ))
            }
        } else {
            Err(VibeError::ResourceNotFound(
                format!("Lazy factory not found for type: {}", std::any::type_name::<T>())
            ))
        }
    }
    
    fn contains<T: 'static>(&self) -> bool {
        let type_id = TypeId::of::<T>();
        self.resources.contains_key(&type_id)
    }
    
    fn remove<T: 'static + Send + Sync>(&mut self) -> Option<Arc<T>> {
        let type_id = TypeId::of::<T>();
        self.lazy_initialized.remove(&type_id)?;
        let _ = self.lazy_factories.remove(&type_id);
        self.resources
            .remove(&type_id)
            .and_then(|arc| arc.downcast::<T>().ok())
    }
}

pub trait ContextExt {
    fn plugin_name(&self) -> &str;
    fn is_in_plugin(&self) -> bool;
}

impl ContextExt for Context {
    fn plugin_name(&self) -> &str {
        self.plugin_meta
            .as_ref()
            .map(|meta| meta.name.as_str())
            .unwrap_or("core")
    }
    
    fn is_in_plugin(&self) -> bool {
        self.plugin_meta.is_some()
    }
}

/// Scoped context for temporary data
pub struct ScopedContext {
    inner: Context,
    scope_data: Arc<RwLock<HashMap<String, Arc<dyn Any + Send + Sync>>>>,
}

impl ScopedContext {
    pub fn new(context: Context) -> Self {
        Self {
            inner: context,
            scope_data: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    pub fn inner(&self) -> &Context {
        &self.inner
    }
    
    pub fn set_scope<T: Send + Sync + 'static>(&self, key: impl Into<String>, value: T) {
        let mut data = self.scope_data.write();
        data.insert(key.into(), Arc::new(value));
    }
    
    pub fn get_scope<T: 'static, R, F>(&self, key: &str, f: F) -> Option<R>
    where
        F: FnOnce(&T) -> R,
    {
        let data = self.scope_data.read();
        data.get(key).and_then(|arc| {
            arc.downcast_ref::<T>().map(f)
        })
    }
}

pub struct ContextBuilder {
    event_bus: Option<Arc<EventBus>>,
}

impl ContextBuilder {
    pub fn new() -> Self {
        Self {
            event_bus: None,
        }
    }
    
    pub fn with_event_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(bus);
        self
    }
    
    pub fn build(self) -> Result<Context> {
        let event_bus = self.event_bus
            .ok_or_else(|| VibeError::RuntimeError("EventBus is required".to_string()))?;
        
        Ok(Context::new(event_bus))
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event::EventBus;
    
    #[test]
    fn test_resource_insert_and_get() {
        let bus = Arc::new(EventBus::new());
        let ctx = Context::new(bus);
        
        ctx.insert::<i32>(42).unwrap();
        let value = ctx.get::<i32, _, _>(|v| *v);
        assert_eq!(value, Some(42));
    }
    
    #[test]
    fn test_resource_lazy_initialization() {
        let bus = Arc::new(EventBus::new());
        let ctx = Context::new(bus);
        
        let init_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count_clone = init_count.clone();
        
        ctx.insert_lazy(move || -> Result<String> {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("initialized".to_string())
        }).unwrap();
        
        let value = ctx.get::<String, _, _>(|v| v.clone());
        assert_eq!(value, Some("initialized".to_string()));
        assert_eq!(init_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        
        let value = ctx.get::<String, _, _>(|v| v.clone());
        assert_eq!(value, Some("initialized".to_string()));
        assert_eq!(init_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
    
    #[test]
    fn test_context_with_plugin() {
        let bus = Arc::new(EventBus::new());
        let ctx = Context::new(bus.clone());
        
        let meta = PluginMetadata::new("test-plugin", "1.0.0");
        let plugin_ctx = ctx.with_plugin(meta);
        
        assert!(plugin_ctx.is_in_plugin());
        assert_eq!(plugin_ctx.plugin_name(), "test-plugin");
    }
    
    #[test]
    fn test_context_building() {
        let bus = Arc::new(EventBus::new());
        let ctx = ContextBuilder::new()
            .with_event_bus(bus)
            .build()
            .unwrap();
        
        assert!(ctx.contains::<i32>() == false);
    }
}
