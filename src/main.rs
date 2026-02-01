// ==============================================================================
// AI DEVELOPER GUIDE - Vibe_Synapse Framework
// ==============================================================================
//
// [FRAMEWORK OVERVIEW]
// This is a Rust-based modular application framework with core features:
// - Automatic module registration via inventory crate
// - Message bus-driven inter-module communication
// - Automatic lifecycle management with graceful shutdown
//
// [KEY CONCEPTS]
// 1. Module: A struct implementing the Module trait, an independent functional unit
// 2. MessageBus: Message bus for modules to send/receive messages
// 3. module_init!: Macro to register a module with the framework
// 4. Message: Data carrier for inter-module communication
//
// [MUST READ FOR AI DEVELOPERS]
// Adding a new module requires TWO STEPS (both are mandatory):
//
// ┌─────────────────────────────────────────────────────────────────────────┐
// │ STEP 1: Declare Module (Let Rust compiler know this file exists)         │
// ├─────────────────────────────────────────────────────────────────────────┤
// │ Add in the "MODULE DECLARATION AREA" below:                              │
// │   pub mod your_module_name;                                              │
// │                                                                          │
// │ Prerequisite: Your module file must be at:                               │
// │   src/model/your_module_name/mod.rs                                      │
// └─────────────────────────────────────────────────────────────────────────┘
//                                    ↓
// ┌─────────────────────────────────────────────────────────────────────────┐
// │ STEP 2: Register Module (Let the framework know this module exists)      │
// ├─────────────────────────────────────────────────────────────────────────┤
// │ At the bottom of your module file (src/model/your_module_name/mod.rs):   │
// │   crate::module_init!(YourStructName, "module_name");                    │
// │                                                                          │
// │ Note: YourStructName must implement Module trait + Default trait         │
// └─────────────────────────────────────────────────────────────────────────┘
//
// [WHY TWO STEPS? TECHNICAL LIMITATION EXPLAINED]
// ┌─────────────────────────────────────────────────────────────────────────┐
// │ Q: Why manual mod declaration when using inventory auto-discovery?       │
// │                                                                          │
// │ A: How Rust module system works:                                         │
// │    - Rust compiler only compiles explicitly referenced code files        │
// │    - Without "mod xxx", src/model/xxx/mod.rs won't be compiled at all    │
// │    - File not compiled → inventory::submit! won't execute                │
// │      → auto-discovery fails                                              │
// │                                                                          │
// │    Therefore, mod declaration is a Rust requirement. Inventory can only  │
// │    collect code from "already compiled" files. This is by Rust design,   │
// │    not a framework flaw.                                                 │
// └─────────────────────────────────────────────────────────────────────────┘
//
// [EXAMPLE: Adding a Logger Module]
// 1. Create file: src/model/logger/mod.rs
// 2. Add in MODULE DECLARATION AREA below: pub mod logger;
// 3. At bottom of logger/mod.rs: crate::module_init!(LoggerModule, "logger");
// ==============================================================================

// Windows GUI subsystem setting (no console window after compilation)
// ⚠️ DANGER: Removing this will cause console window to appear in Windows GUI mode
#![windows_subsystem = "windows"]

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock, watch};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{info, warn, error, debug};

// ==============================================================================
// CONFIGURATION CONSTANTS - Tweak these based on your use case
// ==============================================================================
/// Channel capacity to prevent memory exhaustion under high load
/// 
/// # Design Rationale
/// - 1000 is chosen as a balance between memory usage (~1KB-1MB depending 
///   on message size) and throughput
/// - For high-throughput scenarios, increase to 10000+
/// - For memory-constrained environments, reduce to 100
/// 
/// # When to Adjust
/// - Increase if seeing "Channel full" errors in logs
/// - Decrease if monitoring shows high memory usage from message backlog
const CHANNEL_CAPACITY: usize = 1000;

/// Maximum time allowed for module initialization
/// 
/// # Design Rationale
/// - Modules should do lightweight setup in initialize()
/// - Heavy operations (network connections, file I/O) should be done in spawned tasks
/// - This timeout prevents a single slow module from blocking entire system startup
/// 
/// # Handling Timeouts
/// When a module times out, it is skipped but other modules continue loading.
/// Check logs for "initialization timed out" messages.
const MODULE_INIT_TIMEOUT_SECS: u64 = 30;

/// Maximum time to wait for module shutdown during graceful shutdown
/// 
/// # Design Rationale
/// - Shutdown should be quick but allow enough time for cleanup
/// - If a module cannot shutdown within this time, it is forcefully terminated
const MODULE_SHUTDOWN_TIMEOUT_SECS: u64 = 10;

/// Test mode default duration in seconds
/// 
/// # Usage
/// Override with `--test <seconds>` command line argument
const TEST_MODE_DEFAULT_DURATION_SECS: u64 = 60;

// ==============================================================================
// MODULE DECLARATION AREA - Where AI developers add modules
// ==============================================================================
// [OPERATION GUIDE]
// For each new module, add one line here: pub mod module_name;
// The module name must match the subdirectory name under src/model/
//
// [CURRENT MODULE LIST]
mod model {
    pub mod simple_gui;  // GUI demo module, located at src/model/simple_gui/mod.rs
}

// [TEMPLATE FOR FUTURE MODULES]
// mod model {
//     pub mod simple_gui;   // Existing GUI module
//     pub mod your_module;  // ← Add your module here
// }
//
// ⚠️ IMPORTANT: After modifying this section, remember to add the
//    module_init! macro call at the bottom of your module file!

// ==============================================================================
// INVENTORY-BASED AUTO-REGISTRATION SYSTEM
// ==============================================================================
// AUTOMATIC MODULE DISCOVERY MECHANISM
//
// How it works:
// 1. Each module calls module_init!(ModuleType, "module_name") at file bottom
// 2. This creates a static ModuleBuildInfo with name and constructor function
// 3. inventory::submit! registers the static with the inventory collector
// 4. At compile time, inventory::iter::<ModuleBuildInfo> yields all registered modules
// 5. ModuleRegistry::register_all_modules() constructs and initializes each module
//
// Benefits:
// - No manual module lists to maintain
// - Compile-time safety: can't forget to register a module
// - Type-safe module construction
// - Automatic dependency injection (MessageBus passed to initialize())

/// ModuleBuildInfo stores compile-time information for constructing a module
/// 
/// # Invariants
/// - name must be unique across all modules (enforced at runtime)
/// - construct_fn must never panic (caller assumes infallible construction)
#[derive(Clone, Copy)]
pub struct ModuleBuildInfo {
    pub name: &'static str,
    pub construct_fn: fn() -> Box<dyn Module>,
}

impl ModuleBuildInfo {
    /// Creates new ModuleBuildInfo
    /// 
    /// # Safety Invariant
    /// The construct_fn must be safe to call from any thread and must not panic.
    pub const fn new(name: &'static str, construct_fn: fn() -> Box<dyn Module>) -> Self {
        Self { name, construct_fn }
    }
}

// Collects all ModuleBuildInfo instances submitted via inventory::submit!
// This is the heart of the auto-discovery system
inventory::collect!(ModuleBuildInfo);

/// Macro for modules to self-register with the inventory system
/// 
/// USAGE (add this to the bottom of your module file):
///   module_init!(YourModuleType, "your_module_name");
///
/// This creates:
/// 1. A module constructor function
/// 2. A static ModuleBuildInfo instance
/// 3. Submits the static to inventory for auto-discovery
///
/// EXAMPLE in src/model/my_module/mod.rs:
///   pub struct MyModule { ... }
///   
///   #[async_trait]
///   impl Module for MyModule { ... }
///   
///   impl Default for MyModule {
///       fn default() -> Self { Self::new() }
///   }
///   
///   // Add this line at the bottom of the file
///   module_init!(MyModule, "my_module");
///
/// # Implementation Contract
/// - YourModuleType MUST implement Default (for construction)
/// - YourModuleType MUST implement Module trait
/// - Module name must be unique (runtime panic if duplicate)
#[macro_export]
macro_rules! module_init {
    ($module_ty:ty, $name:expr) => {
        // Module constructor - called by registry to create instances
        // ⚠️ Must never panic - registry assumes infallible construction
        fn construct_module() -> Box<dyn $crate::Module> {
            Box::new(<$module_ty>::default())
        }
        
        // Static build info - stored in inventory at compile time
        #[used]  // Prevents the compiler from optimizing this away
        #[doc(hidden)]
        static MODULE_BUILD_INFO: $crate::ModuleBuildInfo = $crate::ModuleBuildInfo::new(
            $name,
            construct_module
        );
        
        // Submit to inventory for auto-discovery
        inventory::submit! {
            MODULE_BUILD_INFO
        }
    };
}

// ==============================================================================
// CORE ARCHITECTURE: MESSAGE BUS SYSTEM
// ==============================================================================
// MESSAGE-DRIVEN COMMUNICATION SYSTEM
//
// Design Principles:
// - All inter-module communication happens via typed messages
// - Zero direct dependencies between modules
// - Type-safe message routing based on TypeId
// - Arc-based sharing for efficient multi-subscriber delivery
// - Single FIFO channel per message type (simplified from priority system)
//
// Message Flow:
// 1. Publisher creates a typed message implementing Message trait
// 2. bus.publish(message) wraps it in Arc and routes to subscribers
// 3. Dispatcher receives message and forwards to all subscribed modules
// 4. Each module's process_message() is called concurrently
// 5. Results are collected and errors logged
//
// Key Types:
// - Message trait: All messages must implement this
// - MessageEnvelope: Wraps messages with metadata for routing
// - MessageBus: Central hub for publish/subscribe operations
// - TypeId: Compile-time unique identifier for each message type

/// Trait for all inter-module messages
/// 
/// # Implementation Requirements
/// - Must be Send + Sync + 'static (for thread safety)
/// - Must implement clone_box() for Arc-based sharing
/// - Should be Clone for easy implementation
/// - Message type is identified by compile-time TypeId
/// 
/// # Type Safety Guarantee
/// TypeId is guaranteed unique per type by Rust compiler. Two different types
/// will never have the same TypeId, even if they have identical structure.
///
/// # Performance Note
/// clone_box() is only called once when wrapping in Arc. Subscribers receive
/// Arc clones which are cheap (atomic ref-count increment only).
///
/// EXAMPLE MESSAGE TYPE:
/// ```rust
/// #[derive(Clone)]
/// pub struct MyMessage {
///     pub data: String,
/// }
/// 
/// impl Message for MyMessage {
///     fn as_any(&self) -> &dyn Any { self }
///     fn message_type(&self) -> TypeId { TypeId::of::<MyMessage>() }
///     fn clone_box(&self) -> Box<dyn Message> { Box::new(self.clone()) }
/// }
/// ```
pub trait Message: Send + Sync + 'static {
    /// Returns &dyn Any for downcasting to concrete type
    fn as_any(&self) -> &dyn Any;
    
    /// Returns unique TypeId for this message type
    fn message_type(&self) -> TypeId;
    
    /// Creates a Box<dyn Message> clone of self
    /// 
    /// # Performance
    /// This is called once during publish(). Prefer cheap clones here.
    fn clone_box(&self) -> Box<dyn Message>;
}

/// Wraps a message with routing metadata
/// 
/// Fields:
/// - message_type: TypeId for routing to correct subscribers
/// - payload: Arc<Box<dyn Message>> for efficient sharing
/// 
/// # Arc Usage Pattern
/// The Arc enables multiple subscribers to receive the same message
/// without cloning the entire payload (clone_box only called once).
/// 
/// # Clone Behavior
/// clone_arc() only clones the Arc (increments ref count), not the inner message.
#[derive(Clone)]
pub struct MessageEnvelope {
    pub message_type: TypeId,
    pub payload: Arc<Box<dyn Message>>,
    /// Sequence number for debugging and ordering guarantees
    pub sequence: u64,
    /// Timestamp when message was published (for latency monitoring)
    pub timestamp: std::time::Instant,
}

impl MessageEnvelope {
    /// Creates a new envelope from a typed message
    /// 
    /// # Type Safety
    /// TypeId is automatically derived from the generic parameter M.
    pub fn new<M: Message>(msg: M, sequence: u64) -> Self {
        Self {
            message_type: TypeId::of::<M>(),
            payload: Arc::new(Box::new(msg)),
            sequence,
            timestamp: std::time::Instant::now(),
        }
    }
    
    /// Efficient cloning - only clones the Arc, not the inner message
    /// 
    /// # Performance
    /// O(1) operation - only increments atomic reference count
    pub fn clone_arc(&self) -> Self {
        Self {
            message_type: self.message_type,
            payload: Arc::clone(&self.payload),
            sequence: self.sequence,
            timestamp: self.timestamp,
        }
    }
}

/// Internal channel structure for a single message type
/// 
/// # Invariants
/// - receiver is None after dispatcher has started (taken by run_message_dispatcher)
/// - sender remains valid until message type is unregistered
struct MessageChannel {
    sender: mpsc::Sender<MessageEnvelope>,
    /// None when dispatcher has taken ownership
    receiver: Arc<RwLock<Option<mpsc::Receiver<MessageEnvelope>>>>,
}

/// Errors that can occur during message publishing
#[derive(Debug, Clone)]
pub enum PublishError {
    /// Message type not registered with the bus
    TypeNotRegistered(TypeId),
    /// Channel is at capacity (backpressure)
    ChannelFull(TypeId),
    /// Channel has been closed (dispatcher stopped)
    ChannelClosed(TypeId),
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PublishError::TypeNotRegistered(id) => {
                write!(f, "Message type {:?} not registered. Call register_message_type first.", id)
            }
            PublishError::ChannelFull(id) => {
                write!(f, "Channel at capacity for message type {:?}", id)
            }
            PublishError::ChannelClosed(id) => {
                write!(f, "Channel closed for message type {:?}", id)
            }
        }
    }
}

impl std::error::Error for PublishError {}

/// Central message bus for publish/subscribe operations
/// 
/// Thread-safe via RwLock and Arc. Handles:
/// - Message type registration (creates channels)
/// - Message publication (routes to subscribers)
/// - Subscription management (add/remove subscribers)
/// - Auto-starting dispatchers for each message type
///
/// # Usage Pattern
/// ```rust
/// let bus = MessageBus::new();
/// let msg_type = bus.register_message_type::<MyMessage>().await;
/// bus.subscribe(msg_type, "my_module".to_string()).await;
/// bus.publish(MyMessage { data: "hello".to_string() }).await?;
/// ```
#[derive(Clone)]
pub struct MessageBus {
    inner: Arc<MessageBusInner>,
}

struct MessageBusInner {
    channels: RwLock<HashMap<TypeId, MessageChannel>>,
    subscribers: RwLock<HashMap<TypeId, Vec<String>>>,
    /// Maps module name to subscribed message types (for efficient cleanup)
    module_subscriptions: RwLock<HashMap<String, Vec<TypeId>>>,
    registry: std::sync::Mutex<Option<Arc<ModuleRegistry>>>,
    /// Global message sequence counter for ordering/debugging
    sequence_counter: AtomicUsize,
}

impl MessageBus {
    /// Creates a new message bus instance
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(MessageBusInner {
                channels: RwLock::new(HashMap::new()),
                subscribers: RwLock::new(HashMap::new()),
                module_subscriptions: RwLock::new(HashMap::new()),
                registry: std::sync::Mutex::new(None),
                sequence_counter: AtomicUsize::new(0),
            }),
        })
    }
    
    /// Links the bus to a registry (called by ModuleRegistry::new)
    /// 
    /// # Panic Safety
    /// This uses lock() which could panic if poisoned, but registry setup
    /// happens at startup before any potential panic sources.
    pub(crate) fn set_registry(&self, registry: Arc<ModuleRegistry>) {
        match self.inner.registry.lock() {
            Ok(mut guard) => *guard = Some(registry),
            Err(poisoned) => {
                // Lock was poisoned by a panicking thread, but we can still recover
                warn!("Registry lock was poisoned, recovering");
                *poisoned.into_inner() = Some(registry);
            }
        }
    }

    /// Registers a new message type with the bus
    /// 
    /// USAGE:
    ///   let my_message_type = bus.register_message_type::<MyMessage>().await;
    ///   bus.subscribe(my_message_type, "my_module".to_string()).await;
    ///
    /// Side effect: Automatically starts a dispatcher for this message type
    ///
    /// # Idempotency
    /// Calling this multiple times for the same type is safe and cheap (no-op).
    pub async fn register_message_type<M: Message>(&self) -> TypeId {
        let type_id = TypeId::of::<M>();
        let mut channels_guard = self.inner.channels.write().await;
        
        if !channels_guard.contains_key(&type_id) {
            debug!("Registering message type: {:?}", type_id);
            
            // Create single FIFO channel (simplified from priority system)
            let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
            
            channels_guard.insert(type_id, MessageChannel {
                sender,
                receiver: Arc::new(RwLock::new(Some(receiver))),
            });
            
            // Release lock before spawning async tasks to avoid deadlock
            drop(channels_guard);
            
            // Auto-start dispatcher for this message type
            if let Some(registry) = self.get_registry() {
                if let Some(receiver) = self.take_receiver(&type_id).await {
                    info!("Auto-starting dispatcher for message type: {:?}", type_id);
                    tokio::spawn(run_message_dispatcher(
                        registry,
                        Arc::new(self.clone()),
                        type_id,
                        receiver,
                    ));
                }
            }
        }
        
        type_id
    }

    /// Publishes a message to all subscribed modules
    /// 
    /// RETURNS:
    /// - Ok(()) if message was successfully queued
    /// - Err(PublishError) if message type not registered or channel full/closed
    ///
    /// MESSAGE TYPE SAFETY:
    /// - TypeId automatically derived from generic parameter M
    /// - Must call register_message_type::<M>() before publishing first message of type M
    ///
    /// # Backpressure
    /// If the channel is at capacity, this returns ChannelFull error immediately.
    /// Callers should handle this by retrying with backoff or dropping messages.
    pub async fn publish<M: Message>(&self, message: M) -> Result<(), PublishError> {
        let type_id = TypeId::of::<M>();
        let channels_guard = self.inner.channels.read().await;
        
        if let Some(channel) = channels_guard.get(&type_id) {
            let subscriber_count = self.get_subscribers(&type_id).await.len();
            let sequence = self.inner.sequence_counter.fetch_add(1, Ordering::SeqCst) as u64;
            let envelope = MessageEnvelope::new(message, sequence);
            
            // Send to single FIFO channel
            match channel.sender.try_send(envelope) {
                Ok(()) => {
                    if subscriber_count == 0 {
                        warn!("Published message to type {:?} with 0 subscribers", type_id);
                    } else {
                        debug!("Published message to type {:?}, {} subscribers", type_id, subscriber_count);
                    }
                    Ok(())
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    Err(PublishError::ChannelFull(type_id))
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    Err(PublishError::ChannelClosed(type_id))
                }
            }
        } else {
            Err(PublishError::TypeNotRegistered(type_id))
        }
    }

    /// Subscribes a module to receive messages of a specific type
    /// 
    /// USAGE (in module's initialize()):
    ///   let msg_type = bus.register_message_type::<MyMessage>().await;
    ///   bus.subscribe(msg_type, self.name().to_string()).await;
    ///
    /// # Duplicate Subscriptions
    /// A module can subscribe multiple times to the same type, but will only
    /// receive each message once (subscribers are stored in a Vec but filtered).
    pub async fn subscribe(&self, message_type: TypeId, module_name: String) {
        // Add to subscribers map
        let mut subscribers_guard = self.inner.subscribers.write().await;
        subscribers_guard.entry(message_type)
            .or_insert_with(Vec::new)
            .push(module_name.clone());
        drop(subscribers_guard);
        
        // Add to module subscriptions map (for efficient cleanup)
        let mut module_subs = self.inner.module_subscriptions.write().await;
        module_subs.entry(module_name.clone())
            .or_insert_with(Vec::new)
            .push(message_type);
        drop(module_subs);
        
        info!("Module '{}' subscribed to message type: {:?}", module_name, message_type);
    }
    
    /// Unsubscribes a module from a specific message type
    /// 
    /// CALLED AUTOMATICALLY by ModuleRegistry::unregister_module
    /// 
    /// # Returns
    /// true if subscription was found and removed, false if not found
    pub async fn unsubscribe(&self, message_type: &TypeId, module_name: &str) -> bool {
        let mut subscribers_guard = self.inner.subscribers.write().await;
        
        let removed = if let Some(subscribers) = subscribers_guard.get_mut(message_type) {
            let before = subscribers.len();
            subscribers.retain(|s| s != module_name);
            let after = subscribers.len();
            before != after
        } else {
            false
        };
        
        if removed {
            debug!("Module '{}' unsubscribed from message type: {:?}", module_name, message_type);
        }
        
        // Also update module subscriptions map
        drop(subscribers_guard);
        let mut module_subs = self.inner.module_subscriptions.write().await;
        if let Some(subs) = module_subs.get_mut(module_name) {
            subs.retain(|t| t != message_type);
        }
        
        removed
    }
    
    /// Unsubscribes a module from all message types (efficient batch operation)
    /// 
    /// This is O(k) where k is the number of types the module subscribed to,
    /// instead of O(n) where n is total number of message types.
    pub async fn unsubscribe_all(&self, module_name: &str) {
        let types_to_clean: Vec<TypeId> = {
            let mut module_subs = self.inner.module_subscriptions.write().await;
            module_subs.remove(module_name).unwrap_or_default()
        };
        
        if types_to_clean.is_empty() {
            return;
        }
        
        let mut subscribers_guard = self.inner.subscribers.write().await;
        for msg_type in types_to_clean {
            if let Some(subscribers) = subscribers_guard.get_mut(&msg_type) {
                let before = subscribers.len();
                subscribers.retain(|s| s != module_name);
                if subscribers.len() != before {
                    debug!("Removed subscription to {:?}", msg_type);
                }
            }
        }
        
        // Remove empty subscriber lists
        subscribers_guard.retain(|_, subscribers| !subscribers.is_empty());
        
        info!("Cleaned up all subscriptions for module: {}", module_name);
    }

    /// Returns list of modules subscribed to a message type
    pub async fn get_subscribers(&self, message_type: &TypeId) -> Vec<String> {
        let subscribers_guard = self.inner.subscribers.read().await;
        subscribers_guard.get(message_type)
            .cloned()
            .unwrap_or_default()
    }

    /// Internal: Takes receiver channel for dispatcher (consumes it)
    async fn take_receiver(&self, message_type: &TypeId) -> Option<mpsc::Receiver<MessageEnvelope>> {
        let channels_guard = self.inner.channels.read().await;
        if let Some(channel) = channels_guard.get(message_type) {
            let mut rx_guard = channel.receiver.write().await;
            rx_guard.take()
        } else {
            None
        }
    }
    
    /// Gets the registry if set
    fn get_registry(&self) -> Option<Arc<ModuleRegistry>> {
        match self.inner.registry.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => {
                warn!("Registry lock poisoned");
                poisoned.into_inner().clone()
            }
        }
    }
    
    /// Signals the application to exit (called by GUI modules when window closes)
    pub async fn signal_exit(&self) {
        if let Some(registry) = self.get_registry() {
            registry.signal_exit().await;
        }
    }
}

// ==============================================================================
// CORE ARCHITECTURE: MODULE SYSTEM
// ==============================================================================
// MODULE LIFECYCLE AND TRAITS
//
// Each module goes through four phases:
// 1. Construction: Module is created (via Default::default())
// 2. Initialization: ModuleRegistry calls initialize() with Arc<MessageBus>
// 3. Active: Module processes messages via process_message()
// 4. Shutdown: ModuleRegistry calls shutdown() for cleanup
//
// THREAD SAFETY:
// - All methods are async and must be non-blocking
// - Modules must be Send + Sync for concurrent message processing
// - Use Arc<RwLock<T>> for shared state within modules
// - Never store direct references to other modules (use messages!)

/// Errors that can occur during module operations
#[derive(Debug)]
pub enum ModuleError {
    InitializationTimeout { module: String, duration: Duration },
    ShutdownTimeout { module: String, duration: Duration },
    DuplicateName { name: String },
}

impl std::fmt::Display for ModuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModuleError::InitializationTimeout { module, duration } => {
                write!(f, "Module '{}' initialization timed out after {:?}", module, duration)
            }
            ModuleError::ShutdownTimeout { module, duration } => {
                write!(f, "Module '{}' shutdown timed out after {:?}", module, duration)
            }
            ModuleError::DuplicateName { name } => {
                write!(f, "Duplicate module name: '{}'", name)
            }
        }
    }
}

impl std::error::Error for ModuleError {}

/// Core trait for all modules
/// 
/// # Implementation Contract
/// 
/// ## Thread Safety
/// - All methods must be Send + Sync (enforced by trait bounds)
/// - process_message() may be called concurrently from multiple threads
/// - Use interior mutability (RwLock/Mutex) if state needs to change
///
/// ## Non-blocking Requirement
/// - initialize() must complete quickly (< 1 second ideally)
/// - process_message() must never block (use spawn for heavy work)
/// - shutdown() should complete within MODULE_SHUTDOWN_TIMEOUT_SECS
///
/// ## Panic Safety
/// - Methods should not panic under normal conditions
/// - Use Result for error propagation
/// - Framework catches panics but prefers Result-based error handling
///
/// LIFECYCLE METHODS (called by ModuleRegistry):
///
/// 1. name() -> &'static str
///    - Returns unique module identifier
///    - Used for logging, subscription management, and debugging
///    - Must be unique across all modules (enforced at runtime)
///
/// 2. initialize(&mut self, bus: Arc<MessageBus>) 
///    - Called once after module construction
///    - Receives Arc<MessageBus> for message operations
///    - Register message types: bus.register_message_type::<M>().await
///    - Subscribe to messages: bus.subscribe(type_id, self.name().to_string()).await
///    - Perform lightweight setup (no heavy I/O or blocking)
///    - Return Err to prevent module from loading
///    - Has MODULE_INIT_TIMEOUT_SECS to complete or will be skipped
///
/// 3. process_message(&self, envelope: MessageEnvelope)
///    - Called for every message the module is subscribed to
///    - Check message type: envelope.message_type == TypeId::of::<MyMessage>()
///    - Extract message: envelope.payload.as_any().downcast_ref::<MyMessage>()
///    - Must be non-blocking - spawn tasks for heavy work
///    - Return Ok(()) even if message type is irrelevant
///    - May be called concurrently from multiple dispatcher tasks
///
/// 4. shutdown(&mut self)
///    - Called during graceful shutdown
///    - Clean up resources: close connections, flush buffers, etc.
///    - Called before module is removed from registry
///    - Has MODULE_SHUTDOWN_TIMEOUT_SECS to complete
///    - Must return Ok(()) even if cleanup has issues (log errors instead)
#[async_trait]
pub trait Module: Send + Sync {
    /// Returns unique module name (must be static for inventory)
    /// 
    /// # Uniqueness Requirement
    /// This name must be unique across all modules. If two modules return the
    /// same name, the second one will overwrite the first (with a warning).
    fn name(&self) -> &'static str;
    
    /// Initializes module with message bus access
    /// 
    /// # Implementation Guidelines
    /// 
    /// ## Typical Implementation:
    /// ```rust
    /// async fn initialize(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn Error>> {
    ///     // Store bus reference for later use
    ///     *self.bus.write().await = Some(bus.clone());
    ///     
    ///     // Register message types this module publishes/receives
    ///     let msg_type = bus.register_message_type::<MyMessage>().await;
    ///     
    ///     // Subscribe to message types
    ///     bus.subscribe(msg_type, self.name().to_string()).await;
    ///     
    ///     // Lightweight setup only - don't block!
    ///     info!("[MyModule] Initialized");
    ///     Ok(())
    /// }
    /// ```
    ///
    /// ## Error Handling
    /// - Return Err to prevent this module from loading
    /// - Other modules will still be loaded (error isolation)
    /// - Use anyhow::Error or Box<dyn Error> for flexible error types
    ///
    /// ## Timeout
    /// This method has MODULE_INIT_TIMEOUT_SECS to complete. If it takes longer,
    /// the module will be skipped and an error will be logged.
    async fn initialize(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// Processes incoming messages - called by dispatcher
    /// 
    /// # Implementation Pattern
    /// ```rust
    /// async fn process_message(&self, envelope: MessageEnvelope) -> Result<(), Box<dyn Error>> {
    ///     if envelope.message_type == TypeId::of::<MyMessage>() {
    ///         if let Some(msg) = envelope.payload.as_any().downcast_ref::<MyMessage>() {
    ///             // Handle MyMessage
    ///             self.handle_my_message(msg).await?;
    ///         }
    ///     }
    ///     Ok(())  // Always return Ok, even for irrelevant messages
    /// }
    /// ```
    ///
    /// # Performance Requirements
    /// - Must complete quickly (microseconds to milliseconds)
    /// - For heavy processing, spawn a task and return immediately:
    ///   ```rust
    ///   tokio::spawn(async move { heavy_processing().await });
    ///   ```
    ///
    /// # Concurrency
    /// This method may be called concurrently from multiple dispatcher tasks.
    /// Ensure state access is properly synchronized.
    async fn process_message(&self, envelope: MessageEnvelope) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// Cleanup when module is being unloaded
    /// 
    /// # Responsible For:
    /// - Closing network connections
    /// - Flushing buffers to disk
    /// - Releasing external resources
    /// - Saving state if needed
    /// - Signaling any spawned tasks to stop
    ///
    /// # Error Handling
    /// Must return Ok(()) even if cleanup fails (log errors but don't panic).
    /// The module will be removed from registry regardless of return value.
    ///
    /// # Timeout
    /// Has MODULE_SHUTDOWN_TIMEOUT_SECS to complete. Framework will proceed
    /// with shutdown even if this times out.
    async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Registry managing all loaded modules
/// 
/// RESPONSIBILITIES:
/// - Auto-discovery of modules via inventory system
/// - Module lifecycle management (initialize -> run -> shutdown)
/// - Cleanup subscriptions when modules are unloaded
/// - Signal application exit when GUI closes (Windows GUI mode)
/// - Enforce module name uniqueness
pub struct ModuleRegistry {
    pub bus: Arc<MessageBus>,
    modules: Arc<RwLock<HashMap<String, Box<dyn Module>>>>,
    exit_tx: Arc<RwLock<Option<watch::Sender<bool>>>>,
    /// Set of registered module names (for duplicate detection)
    registered_names: Arc<RwLock<std::collections::HashSet<String>>>,
}

impl ModuleRegistry {
    /// Creates a new module registry linked to a message bus
    pub fn new(bus: Arc<MessageBus>) -> Arc<Self> {
        let registry = Arc::new(Self {
            bus: bus.clone(),
            modules: Arc::new(RwLock::new(HashMap::new())),
            exit_tx: Arc::new(RwLock::new(None)),
            registered_names: Arc::new(RwLock::new(std::collections::HashSet::new())),
        });
        
        // Link bus to registry for auto-dispatcher startup
        bus.set_registry(registry.clone());
        
        registry
    }
    
    /// Sets the exit signal sender for GUI graceful shutdown
    /// 
    /// CALLED BY: main() to receive exit notification from GUI
    pub async fn set_exit_sender(&self, sender: watch::Sender<bool>) {
        let mut guard = self.exit_tx.write().await;
        *guard = Some(sender);
    }
    
    /// Signals the application to exit (called by GUI when window closes)
    pub async fn signal_exit(&self) {
        if let Some(tx) = self.exit_tx.read().await.as_ref() {
            let _ = tx.send(true);
            info!("Exit signal sent");
        }
    }

    /// Auto-discovers and registers all modules using inventory system
    /// 
    /// ALGORITHM:
    /// 1. Iterate over all ModuleBuildInfo submitted via inventory::submit!
    /// 2. For each module info: construct -> initialize (with timeout) -> store in map
    /// 3. Log each registration for debugging
    /// 
    /// ERROR HANDLING:
    /// - If a module's initialize() fails or times out, the module is NOT loaded
    /// - Other modules continue loading (error isolation)
    /// - Duplicate module names are detected and warned (last one wins)
    /// 
    /// # Returns
    /// Ok(()) if registration process completed (individual modules may have failed)
    pub async fn register_all_modules(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("\n========== Auto Module Registration ==========");
        
        // Get all module build info from inventory
        let build_infos: Vec<_> = inventory::iter::<ModuleBuildInfo>.into_iter().collect();
        
        if build_infos.is_empty() {
            warn!("No modules discovered. Ensure modules call module_init! macro.");
            return Ok(());
        }
        
        let mut success_count = 0;
        let mut failure_count = 0;
        
        // Construct and initialize each module
        for info in build_infos {
            let module_name = info.name;
            
            // Check for duplicate names
            {
                let mut names = self.registered_names.write().await;
                if names.contains(module_name) {
                    warn!("Duplicate module name detected: '{}'. Last one wins.", module_name);
                }
                names.insert(module_name.to_string());
            }
            
            info!("Registering module: {}", module_name);
            
            // Construct module instance via stored constructor function
            // SAFETY: construct_fn is assumed to never panic (documented invariant)
            let mut module = (info.construct_fn)();
            
            // Initialize module with bus access and timeout
            let init_result = tokio::time::timeout(
                Duration::from_secs(MODULE_INIT_TIMEOUT_SECS),
                module.initialize(self.bus.clone())
            ).await;
            
            match init_result {
                Ok(Ok(())) => {
                    // Store in module map
                    let mut modules_guard = self.modules.write().await;
                    modules_guard.insert(module_name.to_string(), module);
                    success_count += 1;
                    info!("✓ Module '{}' registered successfully", module_name);
                }
                Ok(Err(e)) => {
                    error!("✗ Module '{}' initialization failed: {}", module_name, e);
                    failure_count += 1;
                }
                Err(_) => {
                    error!("✗ Module '{}' initialization timed out after {}s", 
                           module_name, MODULE_INIT_TIMEOUT_SECS);
                    failure_count += 1;
                }
            }
        }
        
        info!("========== Module Registration Complete: {} succeeded, {} failed ==========\n", 
              success_count, failure_count);
        Ok(())
    }

    /// Gracefully unloads a module and cleans up subscriptions
    /// 
    /// STEPS:
    /// 1. Call module.shutdown() for cleanup (with timeout)
    /// 2. Remove module from registry map
    /// 3. Remove all subscriptions for this module (O(k) where k = subscribed types)
    /// 4. Return Ok(()) even if cleanup fails
    pub async fn unregister_module(&self, name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Unregistering module: {}", name);
        
        // Step 1: Shutdown the module with timeout
        let module_opt = {
            let mut modules_guard = self.modules.write().await;
            modules_guard.remove(name)
        };
        
        if let Some(mut module) = module_opt {
            let shutdown_result = tokio::time::timeout(
                Duration::from_secs(MODULE_SHUTDOWN_TIMEOUT_SECS),
                module.shutdown()
            ).await;
            
            match shutdown_result {
                Ok(Ok(())) => {
                    debug!("Module '{}' shutdown successfully", name);
                }
                Ok(Err(e)) => {
                    warn!("Module '{}' shutdown returned error: {}", name, e);
                }
                Err(_) => {
                    warn!("Module '{}' shutdown timed out after {}s", 
                          name, MODULE_SHUTDOWN_TIMEOUT_SECS);
                }
            }
        }
        
        // Step 2: Clean up all subscriptions for this module (efficient O(k) operation)
        self.bus.unsubscribe_all(name).await;
        
        // Remove from registered names
        let mut names = self.registered_names.write().await;
        names.remove(name);
        
        info!("Unregistered module: {}", name);
        Ok(())
    }

    /// Returns list of all registered module names
    pub async fn list_modules(&self) -> Vec<String> {
        let modules_guard = self.modules.read().await;
        modules_guard.keys().cloned().collect()
    }
    
    /// Gets a reference to a module by name
    /// 
    /// # Returns
    /// Some(module) if found, None if not registered
    /// Checks if a module is registered
    pub async fn has_module(&self, name: &str) -> bool {
        let modules_guard = self.modules.read().await;
        modules_guard.contains_key(name)
    }
}

// ==============================================================================
// BUILT-IN MESSAGE TYPES
// ==============================================================================

/// Standard system message for control messages
/// 
/// USE CASES:
/// - System initialization / shutdown notifications
/// - Module control commands
/// - Framework-level events
/// 
/// Fields:
/// - source: Module name sending the message
/// - target: "all" or specific module name (currently unused in dispatcher)
/// - content: String payload
#[derive(Clone, Debug)]
pub struct SystemMessage {
    pub source: String,
    /// Target module name, or "all" for broadcast
    /// NOTE: Currently not used by dispatcher (all subscribers receive all messages)
    pub target: String,
    pub content: String,
}

impl Message for SystemMessage {
    fn as_any(&self) -> &dyn Any {
        self
    }
    
    fn message_type(&self) -> TypeId {
        TypeId::of::<SystemMessage>()
    }
    
    fn clone_box(&self) -> Box<dyn Message> {
        Box::new(self.clone())
    }
}

// ==============================================================================
// MESSAGE DISPATCHER
// ==============================================================================
// ASYNC MESSAGE DISPATCHING SYSTEM
//
// The dispatcher runs in a separate tokio task for each message type.
// It continuously receives messages and forwards them to all subscribed modules.
//
// FLOW:
// 1. Receives MessageEnvelope from channel
// 2. Gets list of subscribed modules from MessageBus
// 3. Spawns a concurrent task for each subscriber
// 4. Waits for all subscribers to process the message
// 5. Logs any errors from subscriber processing
//
// CONCURRENCY MODEL:
// - Each subscriber processes messages in parallel (tokio::spawn per message)
// - Backpressure: Channel capacity limits memory usage
// - Error isolation: One module's error doesn't affect others
// - Panic isolation: Each subscriber task is independent

/// Statistics for a running dispatcher (for monitoring/debugging)
#[derive(Debug, Default)]
struct DispatcherStats {
    messages_processed: AtomicUsize,
    errors_encountered: AtomicUsize,
}

async fn run_message_dispatcher(
    registry: Arc<ModuleRegistry>,
    bus: Arc<MessageBus>,
    message_type: TypeId,
    mut receiver: mpsc::Receiver<MessageEnvelope>,
) {
    info!("Dispatcher started for message type: {:?}", message_type);
    
    let stats = Arc::new(DispatcherStats::default());
    
    while let Some(envelope) = receiver.recv().await {
        let msg_id = envelope.sequence;
        let subscribers = bus.get_subscribers(&envelope.message_type).await;
        
        if subscribers.is_empty() {
            warn!("Message {} has no subscribers (type: {:?})", msg_id, message_type);
            continue;
        }
        
        // Calculate latency for monitoring
        let latency = envelope.timestamp.elapsed();
        if latency > Duration::from_millis(100) {
            warn!("Message {} had high queue latency: {:?}", msg_id, latency);
        }
        
        // Channel for collecting results from all subscribers
        let (tx, mut rx) = mpsc::channel(subscribers.len());
        
        // Spawn concurrent tasks for each subscriber
        for module_name in subscribers {
            let tx_clone = tx.clone();
            let envelope_clone = envelope.clone_arc();
            let registry_clone = registry.clone();
            let stats_clone = stats.clone();
            
            tokio::spawn(async move {
                let result = async {
                    let modules_guard = registry_clone.modules.read().await;
                    if let Some(module) = modules_guard.get(&module_name) {
                        let process_result = module.process_message(envelope_clone).await;
                        drop(modules_guard);
                        process_result
                    } else {
                        // Module was unregistered between getting subscriber list and now
                        warn!("Module '{}' not found for message processing", module_name);
                        Ok(())
                    }
                }.await;
                
                if let Err(ref _e) = result {
                    stats_clone.errors_encountered.fetch_add(1, Ordering::SeqCst);
                }
                
                let _ = tx_clone.send((module_name.clone(), result)).await;
            });
        }
        
        drop(tx);  // Close sender so receiver knows when all are done
        
        // Wait for all subscribers to complete (backpressure)
        while let Some((module_name, result)) = rx.recv().await {
            if let Err(e) = result {
                error!("Module {} error processing message {}: {}", module_name, msg_id, e);
            }
        }
        
        stats.messages_processed.fetch_add(1, Ordering::SeqCst);
    }
    
    let processed = stats.messages_processed.load(Ordering::SeqCst);
    let errors = stats.errors_encountered.load(Ordering::SeqCst);
    info!("Dispatcher stopped for message type: {:?} (processed: {}, errors: {})", 
          message_type, processed, errors);
}

// ==============================================================================
// MAIN APPLICATION ENTRY POINT
// ==============================================================================
// FRAMEWORK BOOTSTRAPPING SEQUENCE
//
// Framework Core Responsibilities:
// 1. Setup panic handler for error isolation
// 2. Initialize tracing for structured logging
// 3. Parse command line arguments (--test mode)
// 4. Create MessageBus and ModuleRegistry - Infrastructure initialization
// 5. Auto-discover and register all modules via inventory - Compile-time discovery
// 6. Register built-in SystemMessage type - Built-in message type registration
// 7. Send initialization test message - System test
// 8. Wait for exit signal (Ctrl+C or test timeout) - Unified lifecycle management
// 9. Graceful shutdown: unregister all modules - Graceful shutdown
//
// [Framework Design Golden Rules]
// - Framework never distinguishes module types (GUI/CLI/Background service)
// - Framework never calls module private APIs
// - Framework never hardcodes specific modules
// - All modules decide independently in initialize() whether to start blocking loops
// - GUI modules use tokio::task::spawn_blocking internally, framework is unaware

/// Command line arguments
#[derive(Debug)]
struct Args {
    test_mode: bool,
    test_duration_secs: u64,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    let mut test_mode = false;
    let mut test_duration_secs = TEST_MODE_DEFAULT_DURATION_SECS;
    
    for i in 0..args.len() {
        if args[i] == "--test" {
            test_mode = true;
            // Check if next arg is a number (custom duration)
            if i + 1 < args.len() {
                if let Ok(secs) = args[i + 1].parse() {
                    test_duration_secs = secs;
                }
            }
        }
    }
    
    Args { test_mode, test_duration_secs }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize tracing for structured logging
    // This replaces println! with proper log levels
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()))
        .init();
    
    // Panic handler prevents module crashes from bringing down the entire system
    // Note: This doesn't catch all panics (e.g., std::process::abort), but helps
    // with most thread panics.
    std::panic::set_hook(Box::new(|panic_info| {
        error!("[Panic Handler] Caught panic: {}", panic_info);
        // Log backtrace if available
        #[cfg(debug_assertions)]
        {
            error!("Backtrace:\n{}", std::backtrace::Backtrace::capture());
        }
    }));

    // Parse command line arguments
    let args = parse_args();

    info!("=== Vibe_Synapse Framework Starting ===");
    
    // Create core framework components
    let bus = MessageBus::new();
    let registry = ModuleRegistry::new(bus.clone());
    
    // Auto-discover and register all modules
    // This uses inventory to find all modules that called module_init!()
    if let Err(e) = registry.register_all_modules().await {
        error!("Module registration failed: {}", e);
        // Continue anyway - some modules may have loaded successfully
    }
    
    // Confirm framework mode
    if args.test_mode {
        info!("\n=== Vibe_Synapse Framework Test Running ({}s) ===", args.test_duration_secs);
    } else {
        info!("\n=== Vibe_Synapse Framework Running ===");
    }
    
    // List all registered modules for debugging
    let modules = registry.list_modules().await;
    if modules.is_empty() {
        warn!("No modules registered! Framework has nothing to do.");
    } else {
        info!("Registered modules: {:?}", modules);
    }
    
    // Register built-in SystemMessage type
    info!("Registering built-in SystemMessage type...");
    bus.register_message_type::<SystemMessage>().await;
    info!("SystemMessage type registered, dispatcher auto-started");
    
    // Send test message to verify message system
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    info!("\n--- Testing message system ---");
    match bus.publish(SystemMessage {
        source: "main".to_string(),
        target: "all".to_string(),
        content: "System initialized and ready".to_string(),
    }).await {
        Ok(()) => info!("Published initialization message"),
        Err(e) => error!("Failed to publish: {}", e),
    }
    
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Main execution - framework waits for exit signal
    // Framework handles all modules uniformly, no special branches for specific modules
    if args.test_mode {
        // Test mode: Run for specified duration then exit
        info!("\n=== Test Mode - Framework will run for {} seconds ===", args.test_duration_secs);
        tokio::time::sleep(Duration::from_secs(args.test_duration_secs)).await;
        info!("\n=== Test completed ===");
    } else {
        // Normal mode: Wait for exit signal (Ctrl+C or GUI closed)
        info!("\n=== Framework Running ===");
        info!("Press Ctrl+C to exit...");
        
        // Create exit signal channel for GUI to notify exit
        let (exit_tx, mut exit_rx) = watch::channel(false);
        registry.set_exit_sender(exit_tx).await;
        
        // Wait for either Ctrl+C or GUI exit signal
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("\nCtrl+C received, shutting down...");
            }
            result = exit_rx.changed() => {
                if result.is_ok() && *exit_rx.borrow() {
                    info!("\nGUI closed, shutting down...");
                }
            }
        }
    }
    
    // Graceful shutdown
    info!("\n=== Vibe_Synapse Framework Shutting Down ===");
    
    for module_name in modules {
        if let Err(e) = registry.unregister_module(&module_name).await {
            error!("Error unregistering module {}: {}", module_name, e);
        }
    }
    
    info!("Shutdown complete");
    Ok(())
}

// ==============================================================================
// AI DEVELOPER REFERENCE - Complete Development Guide
// ==============================================================================
//
// ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
// ┃  Complete Workflow for Adding New Modules (Must complete both steps)       ┃
// ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
//
// ┌────────────────────────────────────────────────────────────────────────────┐
// │ Step 1: Declare Module (in main.rs)                                        │
// ├────────────────────────────────────────────────────────────────────────────┤
// │ 1. Find the "MODULE DECLARATION AREA" above                                │
// │ 2. Add in the mod model { } block:                                         │
// │      pub mod your_module_name;                                             │
// │ 3. Create directory under src/model/: your_module_name/                    │
// │ 4. Create mod.rs file in that directory                                    │
// └────────────────────────────────────────────────────────────────────────────┘
//                                      ↓
// ┌────────────────────────────────────────────────────────────────────────────┐
// │ Step 2: Implement and Register Module (in your mod.rs)                     │
// ├────────────────────────────────────────────────────────────────────────────┤
// │ Create your module file following the template below:                      │
// └────────────────────────────────────────────────────────────────────────────┘

/*
// File: src/model/my_module/mod.rs

use async_trait::async_trait;
use std::sync::Arc;
use crate::{Message, MessageEnvelope, MessageBus, Module};
use std::any::{Any, TypeId};
use tokio::sync::RwLock;
use tracing::{info, error};

// ========== Module Struct ==========
pub struct MyModule {
    name: &'static str,
    bus: Arc<RwLock<Option<Arc<MessageBus>>>>,
}

impl MyModule {
    pub fn new() -> Self {
        Self {
            name: "my_module",
            bus: Arc::new(RwLock::new(None)),
        }
    }
}

impl Default for MyModule {
    fn default() -> Self {
        Self::new()
    }
}

// ========== Module Lifecycle Implementation ==========
#[async_trait]
impl Module for MyModule {
    fn name(&self) -> &'static str {
        self.name
    }
    
    async fn initialize(&mut self, bus: Arc<MessageBus>) 
        -> Result<(), Box<dyn std::error::Error + Send + Sync>> 
    {
        // Save bus reference (optional, needed if sending messages)
        *self.bus.write().await = Some(bus.clone());
        
        // Register and subscribe to message types (if needed)
        // let msg_type = bus.register_message_type::<MyMessage>().await;
        // bus.subscribe(msg_type, self.name().to_string()).await;
        
        info!("[MyModule] Initialized");
        Ok(())
    }
    
    async fn process_message(&self, _envelope: MessageEnvelope) 
        -> Result<(), Box<dyn std::error::Error + Send + Sync>> 
    {
        // Handle received messages
        Ok(())
    }
    
    async fn shutdown(&mut self) 
        -> Result<(), Box<dyn std::error::Error + Send + Sync>> 
    {
        info!("[MyModule] Shutting down");
        Ok(())
    }
}

// ========== Auto-registration (Must be at bottom of file) ==========
crate::module_init!(MyModule, "my_module");

// ========== Optional: Define Message Types ==========
#[derive(Clone)]
pub struct MyMessage {
    pub data: String,
}

impl Message for MyMessage {
    fn as_any(&self) -> &dyn Any { self }
    fn message_type(&self) -> TypeId { TypeId::of::<Self>() }
    fn clone_box(&self) -> Box<dyn Message> { Box::new(self.clone()) }
}
*/

// ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
// ┃  Inter-Module Communication Guide                                          ┃
// ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
//
// [Defining Message Types]
// Define message structs in sender or common location, implement Message trait:
//
//   #[derive(Clone)]
//   pub struct MyMessage {
//       pub data: String,
//   }
//
//   impl Message for MyMessage {
//       fn as_any(&self) -> &dyn Any { self }
//       fn message_type(&self) -> TypeId { TypeId::of::<MyMessage>() }
//       fn clone_box(&self) -> Box<dyn Message> { Box::new(self.clone()) }
//   }
//
// [Sending Messages]
//   bus.publish(MyMessage { data: "hello".to_string() }).await?;
//
// [Receiving Messages]
// 1. Register and subscribe in initialize():
//    let msg_type = bus.register_message_type::<MyMessage>().await;
//    bus.subscribe(msg_type, self.name().to_string()).await;
//
// 2. Handle in process_message():
//    if envelope.message_type == TypeId::of::<MyMessage>() {
//        if let Some(msg) = envelope.payload.as_any().downcast_ref::<MyMessage>() {
//            info!("Received: {}", msg.data);
//        }
//    }
//
// ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
// ┃  Troubleshooting Guide                                                     ┃
// ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
//
// [Module not loading?]
// □ Did you add pub mod xxx; in MODULE DECLARATION AREA in main.rs?
// □ Did you add crate::module_init!(...); at bottom of your module file?
// □ Does your module struct implement Default trait?
// □ Does your module struct implement Module trait?
// □ Did initialize() complete within MODULE_INIT_TIMEOUT_SECS seconds?
// □ Any compilation errors? (run cargo check)
//
// [Messages not received?]
// □ Did you call bus.register_message_type::<M>().await?
// □ Did you call bus.subscribe(type_id, ...).await?
// □ Does message type match? (Sender and receiver use same type)
// □ Does subscriber name match module name() return value?
// □ Check logs for "0 subscribers" warnings
//
// [Module initialization failing?]
// □ Is initialize() blocking? (Should not block; use spawn_blocking for GUI/IO)
// □ Any unwrap() causing panic?
// □ Is module name unique? (Cannot duplicate other modules)
// □ Check for timeout in logs
//
// [High memory usage?]
// □ Check CHANNEL_CAPACITY setting - reduce if too high
// □ Look for memory leaks in custom modules
// □ Check message queue depths in logs
//
// ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
// ┃  Performance Best Practices                                                ┃
// ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
//
// - Keep clone_box() implementation efficient (avoid deep copies)
// - process_message() must not block; spawn tasks for heavy operations
// - Use Arc<RwLock<T>> for shared state (not Arc<Mutex<T>> unless needed)
// - Prefer message passing over direct function calls to other modules
// - Keep initialize() lightweight; heavy init operations in separate tasks
// - Use try_send() for non-blocking publish when backpressure is acceptable
// - Monitor logs for "high queue latency" warnings
//
// ==============================================================================
// HAPPY CODING! The framework handles all the boilerplate, just focus on your
// business logic!
// ==============================================================================
