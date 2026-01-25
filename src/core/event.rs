//! Event system

use async_trait::async_trait;
use std::any::{Any, TypeId};
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, RwLock};
use std::collections::HashMap;
use crate::error::{Result, VibeError};

/// Event priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Priority {
    High = 0,
    #[default]
    Medium = 1,
    Low = 2,
}

/// Event trait
pub trait Event: Send + Sync + 'static {
    fn event_type(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
    
    fn priority(&self) -> Priority {
        Priority::default()
    }
    
    fn as_any(&self) -> &dyn Any;
}

/// Auto-implement Event trait
#[macro_export]
macro_rules! impl_event {
    ($($t:ty),+) => {
        $(
            impl $crate::core::event::Event for $t {
                fn as_any(&self) -> &dyn std::any::Any {
                    self
                }
            }
        )+
    };
}

impl_event!(
    String, i32, i64, u32, u64, f32, f64, bool,
    Vec<u8>, Vec<String>, HashMap<String, String>
);

/// Event wrapper with metadata
#[derive(Debug)]
pub struct EventWrapper {
    pub inner: Arc<dyn Any + Send + Sync>,
    pub type_id: TypeId,
    pub priority: Priority,
    pub timestamp: u64,
    pub source: Option<String>,
    pub response_tx: Option<oneshot::Sender<ErasedResponse>>,
}

#[derive(Debug)]
pub enum ErasedResponse {
    Success(Box<dyn Any + Send + Sync>),
    Error(String),
}

impl EventWrapper {
    pub fn new<E: Event>(event: E) -> Self {
        Self {
            inner: Arc::new(event),
            type_id: TypeId::of::<E>(),
            priority: Priority::default(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
            source: None,
            response_tx: None,
        }
    }
    
    #[must_use]
    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }
    
    #[must_use]
    pub fn with_source(mut self, source: String) -> Self {
        self.source = Some(source);
        self
    }
    
    #[must_use]
    pub fn downcast<E: Any>(&self) -> Option<&E> {
        self.inner.downcast_ref::<E>()
    }
    
    #[must_use]
    pub fn event_type_name(&self) -> &'static str {
        "Unknown"
    }
}

/// Event handler trait
#[async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: &EventWrapper) -> Result<()>;
    fn name(&self) -> &'static str;
}

/// Async event bus
pub struct EventBus {
    high_queue: mpsc::UnboundedSender<EventWrapper>,
    medium_queue: mpsc::UnboundedSender<EventWrapper>,
    low_queue: mpsc::UnboundedSender<EventWrapper>,
    handlers: Arc<RwLock<HashMap<TypeId, Vec<Box<dyn EventHandler>>>>>,
    high_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<EventWrapper>>>>,
    medium_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<EventWrapper>>>>,
    low_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<EventWrapper>>>>,
    is_running: Arc<RwLock<bool>>,
    event_count: Arc<std::sync::atomic::AtomicU64>,
}

impl EventBus {
    pub fn new() -> Self {
        let (high_tx, high_rx) = mpsc::unbounded_channel();
        let (medium_tx, medium_rx) = mpsc::unbounded_channel();
        let (low_tx, low_rx) = mpsc::unbounded_channel();
        
        Self {
            high_queue: high_tx,
            medium_queue: medium_tx,
            low_queue: low_tx,
            handlers: Arc::new(RwLock::new(HashMap::new())),
            high_rx: Arc::new(RwLock::new(Some(high_rx))),
            medium_rx: Arc::new(RwLock::new(Some(medium_rx))),
            low_rx: Arc::new(RwLock::new(Some(low_rx))),
            is_running: Arc::new(RwLock::new(false)),
            event_count: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }
    
    pub fn publish<E: Event>(&self, event: E) -> Result<()> {
        let wrapper = EventWrapper::new(event);
        self.publish_wrapper(wrapper)
    }
    
    pub fn publish_with_priority<E: Event>(&self, event: E, priority: Priority) -> Result<()> {
        let wrapper = EventWrapper::new(event).with_priority(priority);
        self.publish_wrapper(wrapper)
    }
    
    pub async fn publish_and_wait<E: Event, R: Event + 'static>(
        &self,
        event: E,
        timeout: Duration,
    ) -> Result<R> {
        let (tx, rx) = oneshot::channel();
        let mut wrapper = EventWrapper::new(event);
        wrapper.response_tx = Some(tx);
        
        self.publish_wrapper(wrapper)?;
        
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => match response {
                ErasedResponse::Success(data) => {
                    data.downcast::<R>()
                        .map(|b| *b)
                        .map_err(|_| VibeError::EventError("Invalid response type".to_string()))
                }
                ErasedResponse::Error(msg) => Err(VibeError::EventError(msg)),
            },
            Ok(Err(_)) => Err(VibeError::EventError("Response channel closed".to_string())),
            Err(_) => Err(VibeError::TimeoutError("Event response timeout".to_string())),
        }
    }
    
    fn publish_wrapper(&self, wrapper: EventWrapper) -> Result<()> {
        let queue = match wrapper.priority {
            Priority::High => &self.high_queue,
            Priority::Medium => &self.medium_queue,
            Priority::Low => &self.low_queue,
        };
        
        queue.send(wrapper).map_err(|e| {
            VibeError::EventError(format!("Failed to publish event: {}", e))
        })?;
        
        Ok(())
    }
    
    pub async fn subscribe<E: Event, H: EventHandler + 'static>(
        &self,
        handler: H,
    ) -> Result<()> {
        let type_id = TypeId::of::<E>();
        let mut handlers = self.handlers.write().await;
        handlers.entry(type_id).or_default().push(Box::new(handler));
        Ok(())
    }
    
    pub async fn subscribe_handler<H: EventHandler + 'static>(
        &self,
        handler: H,
        event_type: TypeId,
    ) -> Result<()> {
        let mut handlers = self.handlers.write().await;
        handlers.entry(event_type).or_default().push(Box::new(handler));
        Ok(())
    }
    
    pub async fn start(&self) -> Result<()> {
        let mut running = self.is_running.write().await;
        if *running {
            return Err(VibeError::RuntimeError("Event bus already running".to_string()));
        }
        *running = true;
        
        let mut high_rx = self.high_rx.write().await
            .take()
            .ok_or_else(|| VibeError::RuntimeError("High priority receiver already taken".to_string()))?;
            
        let mut medium_rx = self.medium_rx.write().await
            .take()
            .ok_or_else(|| VibeError::RuntimeError("Medium priority receiver already taken".to_string()))?;
            
        let mut low_rx = self.low_rx.write().await
            .take()
            .ok_or_else(|| VibeError::RuntimeError("Low priority receiver already taken".to_string()))?;
        
        let handlers = Arc::clone(&self.handlers);
        let event_count = Arc::clone(&self.event_count);
        let is_running = Arc::clone(&self.is_running);
        
        tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    Some(event) = high_rx.recv() => event,
                    Some(event) = medium_rx.recv() => event,
                    Some(event) = low_rx.recv() => event,
                    else => break,
                };
                
                event_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                
                let handlers_guard = handlers.read().await;
                if let Some(handlers_list) = handlers_guard.get(&event.type_id) {
                    for handler in handlers_list {
                        let _ = handler.handle(&event).await;
                    }
                }
            }
            
            let mut running_guard = is_running.write().await;
            *running_guard = false;
        });
        
        Ok(())
    }
    
    pub async fn stop(&self) -> Result<()> {
        let mut running = self.is_running.write().await;
        *running = false;
        Ok(())
    }
    
    pub fn processed_events(&self) -> u64 {
        self.event_count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ClosureHandler<F> {
    name: &'static str,
    f: F,
}

#[async_trait]
impl<F, Fut> EventHandler for ClosureHandler<F>
where
    F: Fn(&EventWrapper) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<()>> + Send,
{
    async fn handle(&self, event: &EventWrapper) -> Result<()> {
        (self.f)(event).await
    }
    
    fn name(&self) -> &'static str {
        self.name
    }
}

pub fn handler<F, Fut>(name: &'static str, f: F) -> ClosureHandler<F>
where
    F: Fn(&EventWrapper) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<()>> + Send,
{
    ClosureHandler { name, f }
}

#[async_trait]
pub trait Request: Event {
    type Response: Event;
}

pub trait BroadcastEvent: Event {}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio;
    
    #[derive(Debug)]
    struct TestEvent {
        value: i32,
    }
    
    impl Event for TestEvent {
        fn as_any(&self) -> &dyn Any {
            self
        }
    }
    
    struct TestHandler;
    
    #[async_trait]
    impl EventHandler for TestHandler {
        async fn handle(&self, event: &EventWrapper) -> Result<()> {
            if let Some(e) = event.downcast::<TestEvent>() {
                assert_eq!(e.value, 42);
            }
            Ok(())
        }
        
        fn name(&self) -> &'static str {
            "TestHandler"
        }
    }
    
    #[tokio::test]
    async fn test_event_bus_publish_and_subscribe() {
        let bus = EventBus::new();
        
        bus.subscribe::<TestEvent, _>(TestHandler).await.unwrap();
        bus.start().await.unwrap();
        
        let event = TestEvent { value: 42 };
        bus.publish(event).unwrap();
        
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        
        assert_eq!(bus.processed_events(), 1);
    }
}
