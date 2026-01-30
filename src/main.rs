// ==============================================================================
// AI DEVELOPER GUIDE - Vibe_Synapse Framework
// ==============================================================================
//
// [FRAMEWORK OVERVIEW]
// This is a Rust-based modular application framework with core features:
// - Automatic module registration via inventory crate
// - Message bus-driven inter-module communication
// - Automatic lifecycle management
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
#![windows_subsystem = "windows"]

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, watch};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

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
#[derive(Clone, Copy)]
pub struct ModuleBuildInfo {
    pub name: &'static str,
    pub construct_fn: fn() -> Box<dyn Module>,
}

impl ModuleBuildInfo {
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
#[macro_export]
macro_rules! module_init {
    ($module_ty:ty, $name:expr) => {
        // Module constructor - called by registry to create instances
        fn construct_module() -> Box<dyn $crate::Module> {
            Box::new(<$module_ty>::default())
        }
        
        // Static build info - stored in inventory at compile time
        #[used]  // Prevents the compiler from optimizing this away
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
/// REQUIREMENTS FOR IMPLEMENTATION:
/// - Must be Send + Sync + 'static (for thread safety)
/// - Must implement clone_box() for Arc-based sharing
/// - Should be Clone for easy implementation
/// - Message type is identified by compile-time TypeId
/// 
/// EXAMPLE MESSAGE TYPE:
/// ```
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
    fn as_any(&self) -> &dyn Any;
    fn message_type(&self) -> TypeId;
    fn clone_box(&self) -> Box<dyn Message>;
}

/// Wraps a message with routing metadata
/// 
/// Fields:
/// - message_type: TypeId for routing to correct subscribers
/// - payload: Arc<Box<dyn Message>> for efficient sharing
/// 
/// The Arc enables multiple subscribers to receive the same message
/// without cloning the entire payload (clone_box only called once).
pub struct MessageEnvelope {
    pub message_type: TypeId,
    pub payload: Arc<Box<dyn Message>>,
}

impl MessageEnvelope {
    /// Creates a new envelope from a typed message
    pub fn new<M: Message>(msg: M) -> Self {
        Self {
            message_type: TypeId::of::<M>(),
            payload: Arc::new(Box::new(msg)),
        }
    }
    
    /// Efficient cloning - only clones the Arc, not the inner message
    pub fn clone_arc(&self) -> Self {
        Self {
            message_type: self.message_type,
            payload: Arc::clone(&self.payload),
        }
    }
}

// Channel capacity to prevent memory exhaustion under high load
const CHANNEL_CAPACITY: usize = 1000;

/// Internal channel structure for a single message type
struct MessageChannel {
    sender: mpsc::Sender<MessageEnvelope>,
    receiver: Arc<RwLock<Option<mpsc::Receiver<MessageEnvelope>>>>,
}

/// Central message bus for publish/subscribe operations
/// 
/// Thread-safe via RwLock and Arc. Handles:
/// - Message type registration (creates channels)
/// - Message publication (routes to subscribers)
/// - Subscription management (add/remove subscribers)
/// - Auto-starting dispatchers for each message type
#[derive(Clone)]
pub struct MessageBus {
    inner: Arc<MessageBusInner>,
}

struct MessageBusInner {
    channels: RwLock<HashMap<TypeId, MessageChannel>>,
    subscribers: RwLock<HashMap<TypeId, Vec<String>>>,
    registry: std::sync::Mutex<Option<Arc<ModuleRegistry>>>,
}

impl MessageBus {
    /// Creates a new message bus instance
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(MessageBusInner {
                channels: RwLock::new(HashMap::new()),
                subscribers: RwLock::new(HashMap::new()),
                registry: std::sync::Mutex::new(None),
            }),
        })
    }
    
    /// Links the bus to a registry (called by ModuleRegistry::new)
    pub(crate) fn set_registry(&self, registry: Arc<ModuleRegistry>) {
        *self.inner.registry.lock().unwrap() = Some(registry);
    }

    /// Registers a new message type with the bus
    /// 
    /// USAGE:
    ///   let my_message_type = bus.register_message_type::<MyMessage>().await;
    ///   bus.subscribe(my_message_type, "my_module".to_string()).await;
    ///
    /// Side effect: Automatically starts a dispatcher for this message type
    pub async fn register_message_type<M: Message>(&self) -> TypeId {
        let type_id = TypeId::of::<M>();
        let mut channels_guard = self.inner.channels.write().await;
        
        if !channels_guard.contains_key(&type_id) {
            // Create single FIFO channel (simplified from priority system)
            let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
            
            channels_guard.insert(type_id, MessageChannel {
                sender,
                receiver: Arc::new(RwLock::new(Some(receiver))),
            });
            
            // Release lock before spawning async tasks
            drop(channels_guard);
            
            // Auto-start dispatcher for this message type
            let registry_opt = self.inner.registry.lock().unwrap().clone();
            if let Some(registry) = registry_opt {
                if let Some(receiver) = self.get_receiver(&type_id).await {
                    println!("[MessageBus] Auto-starting dispatcher for message type: {:?}", type_id);
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
    /// - Err(String) if message type not registered or channel full
    ///
    /// MESSAGE TYPE SAFETY:
    /// - TypeId automatically derived from generic parameter M
    /// - Must call register_message_type::<M>() before publishing first message of type M
    pub async fn publish<M: Message>(&self, message: M) -> Result<(), String> {
        let type_id = TypeId::of::<M>();
        let channels_guard = self.inner.channels.read().await;
        
        if let Some(channel) = channels_guard.get(&type_id) {
            let subscriber_count = self.get_subscribers(&type_id).await.len();
            let envelope = MessageEnvelope::new(message);
            
            // Send to single FIFO channel (simplified routing)
            let result = channel.sender.send(envelope).await;
            
            match result {
                Ok(()) => {
                    if subscriber_count == 0 {
                        eprintln!("[MessageBus] Warning: Published message to type {:?} with 0 subscribers", type_id);
                    } else {
                        eprintln!("[MessageBus] Published message to type {:?}, {} subscribers", type_id, subscriber_count);
                    }
                    Ok(())
                }
                Err(_) => Err(format!("Channel closed or full for message type {:?}", type_id)),
            }
        } else {
            Err(format!("Message type {:?} not registered. Call register_message_type first.", type_id))
        }
    }

    /// Subscribes a module to receive messages of a specific type
    /// 
    /// USAGE (in module's initialize()):
    ///   let msg_type = bus.register_message_type::<MyMessage>().await;
    ///   bus.subscribe(msg_type, self.name().to_string()).await;
    pub async fn subscribe(&self, message_type: TypeId, module_name: String) {
        let mut subscribers_guard = self.inner.subscribers.write().await;
        subscribers_guard.entry(message_type)
            .or_insert_with(Vec::new)
            .push(module_name.clone());
        
        println!("[MessageBus] Module '{}' subscribed to message type: {:?}", module_name, message_type);
    }
    
    /// Unsubscribes a module from a message type
    /// 
    /// CALLED AUTOMATICALLY by ModuleRegistry::unregister_module
    pub async fn unsubscribe(&self, message_type: &TypeId, module_name: &str) -> bool {
        let mut subscribers_guard = self.inner.subscribers.write().await;
        
        if let Some(subscribers) = subscribers_guard.get_mut(message_type) {
            let before = subscribers.len();
            subscribers.retain(|s| s != module_name);
            let removed = before != subscribers.len();
            
            if removed {
                println!("[MessageBus] Module '{}' unsubscribed from message type: {:?}", module_name, message_type);
            }
            
            return removed;
        }
        
        false
    }

    /// Returns list of modules subscribed to a message type
    pub async fn get_subscribers(&self, message_type: &TypeId) -> Vec<String> {
        let subscribers_guard = self.inner.subscribers.read().await;
        subscribers_guard.get(message_type)
            .cloned()
            .unwrap_or_default()
    }

    /// Internal: Gets receiver channel for dispatcher
    async fn get_receiver(&self, message_type: &TypeId) -> Option<mpsc::Receiver<MessageEnvelope>> {
        let channels_guard = self.inner.channels.read().await;
        if let Some(channel) = channels_guard.get(message_type) {
            let mut rx_guard = channel.receiver.write().await;
            rx_guard.take()
        } else {
            None
        }
    }
    
    /// Signals the application to exit (called by GUI modules when window closes)
    pub async fn signal_exit(&self) {
        let registry_opt = self.inner.registry.lock().unwrap().clone();
        if let Some(registry) = registry_opt {
            registry.signal_exit().await;
        }
    }
}

// ==============================================================================
// CORE ARCHITECTURE: MODULE SYSTEM
// ==============================================================================
// MODULE LIFECYCLE AND TRAITS
//
// Each module goes through three phases:
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

/// Core trait for all modules
/// 
/// LIFECYCLE METHODS (called by ModuleRegistry):
/// 
/// 1. name() -> &'static str
///    - Returns unique module identifier
///    - Used for logging, subscription management, and debugging
///    - Must be unique across all modules
/// 
/// 2. initialize(&mut self, bus: Arc<MessageBus>) 
///    - Called once after module construction
///    - Receives Arc<MessageBus> for message operations
///    - Register message types: bus.register_message_type::<M>().await
///    - Subscribe to messages: bus.subscribe(type_id, self.name().to_string()).await
///    - Perform lightweight setup (no heavy I/O or blocking)
///    - Return Err to prevent module from loading
/// 
/// 3. process_message(&self, envelope: MessageEnvelope)
///    - Called for every message the module is subscribed to
///    - Check message type: envelope.message_type == TypeId::of::<MyMessage>()
///    - Extract message: envelope.payload.as_any().downcast_ref::<MyMessage>()
///    - Must be non-blocking - spawn tasks for heavy work
///    - Return Ok(()) even if message type is irrelevant
/// 
/// 4. shutdown(&mut self)
///    - Called during graceful shutdown
///    - Clean up resources: close connections, flush buffers, etc.
///    - Called before module is removed from registry
#[async_trait]
pub trait Module: Send + Sync {
    /// Returns unique module name (must be static for inventory)
    fn name(&self) -> &'static str;
    
    /// Initializes module with message bus access
    /// 
    /// TYPICAL IMPLEMENTATION:
    ///   async fn initialize(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn Error>> {
    ///       // Store bus reference for later use
    ///       self.bus.write().await = Some(bus.clone());
    ///       
    ///       // Register message types this module publishes/receives
    ///       let msg_type = bus.register_message_type::<MyMessage>().await;
    ///       
    ///       // Subscribe to message types
    ///       bus.subscribe(msg_type, self.name().to_string()).await;
    ///       
    ///       // Lightweight setup only - don't block!
    ///       Ok(())
    ///   }
    async fn initialize(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// Processes incoming messages - called by dispatcher
    /// 
    /// IMPLEMENTATION PATTERN:
    ///   async fn process_message(&self, envelope: MessageEnvelope) -> Result<(), Box<dyn Error>> {
    ///       if envelope.message_type == TypeId::of::<MyMessage>() {
    ///           if let Some(msg) = envelope.payload.as_any().downcast_ref::<MyMessage>() {
    ///               // Handle MyMessage
    ///               self.handle_my_message(msg).await?;
    ///           }
    ///       }
    ///       Ok(())  // Always return Ok, even for irrelevant messages
    ///   }
    async fn process_message(&self, envelope: MessageEnvelope) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// Cleanup when module is being unloaded
    /// 
    /// RESPONSIBLE FOR:
    /// - Closing network connections
    /// - Flushing buffers to disk
    /// - Releasing external resources
    /// - Saving state if needed
    /// Must return Ok(()) even if cleanup fails (log errors but don't panic)
    async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Registry managing all loaded modules
/// 
/// RESPONSIBILITIES:
/// - Auto-discovery of modules via inventory system
/// - Module lifecycle management (initialize -> run -> shutdown)
/// - Cleanup subscriptions when modules are unloaded
/// - Signal application exit when GUI closes (Windows GUI mode)
pub struct ModuleRegistry {
    pub bus: Arc<MessageBus>,
    modules: Arc<RwLock<HashMap<String, Box<dyn Module>>>>,
    exit_tx: Arc<RwLock<Option<watch::Sender<bool>>>>,
}

impl ModuleRegistry {
    /// Creates a new module registry linked to a message bus
    pub fn new(bus: Arc<MessageBus>) -> Arc<Self> {
        let registry = Arc::new(Self {
            bus: bus.clone(),
            modules: Arc::new(RwLock::new(HashMap::new())),
            exit_tx: Arc::new(RwLock::new(None)),
        });
        
        // Link bus to registry for auto-dispatcher startup
        bus.set_registry(registry.clone());
        
        registry
    }
    
    /// Sets the exit signal sender for GUI graceful shutdown
    /// 
    /// CALLED BY: main() to receive exit notification from GUI
    pub async fn set_exit_sender(&self, sender: watch::Sender<bool>) {
        *self.exit_tx.write().await = Some(sender);
    }
    
    /// Signals the application to exit (called by GUI when window closes)
    pub async fn signal_exit(&self) {
        if let Some(tx) = self.exit_tx.read().await.as_ref() {
            let _ = tx.send(true);
            println!("[ModuleRegistry] Exit signal sent");
        }
    }

    /// Auto-discovers and registers all modules using inventory system
    /// 
    /// ALGORITHM:
    /// 1. Iterate over all ModuleBuildInfo submitted via inventory::submit!
    /// 2. For each module info: construct -> initialize -> store in map
    /// 3. Log each registration for debugging
    /// 
   /// ERROR HANDLING:
    /// - If a module's initialize() fails, the module is NOT loaded
    /// - Other modules continue loading (error isolation)
    /// - Returns Err if any module fails to load (fail-fast)
    pub async fn register_all_modules(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        println!("\n========== Auto Module Registration ==========");
        
        // Get all module build info from inventory
        let build_infos: Vec<_> = inventory::iter::<ModuleBuildInfo>.into_iter().collect();
        
        if build_infos.is_empty() {
            println!("⚠  Warning: No modules discovered. Ensure modules call module_init! macro.");
            return Ok(());
        }
        
        // Construct and initialize each module
        for info in build_infos {
            let module_name = info.name;
            println!("Registering module: {}", module_name);
            
            // Construct module instance via stored constructor function
            let mut module = (info.construct_fn)();
            
            // Initialize module with bus access
            module.initialize(self.bus.clone()).await?;
            
            // Store in module map
            let mut modules_guard = self.modules.write().await;
            modules_guard.insert(module_name.to_string(), module);
            
            println!("✓ Module '{}' registered successfully", module_name);
        }
        
        println!("========== Module Registration Complete ==========\n");
        Ok(())
    }

    /// Gracefully unloads a module and cleans up subscriptions
    /// 
    /// STEPS:
    /// 1. Call module.shutdown() for cleanup
    /// 2. Remove module from registry map
    /// 3. Remove all subscriptions for this module
    /// 4. Return Ok(()) even if cleanup fails
    pub async fn unregister_module(&self, name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Step 1: Shutdown the module
        let mut modules_guard = self.modules.write().await;
        if let Some(mut module) = modules_guard.remove(name) {
            module.shutdown().await?;
        }
        drop(modules_guard);
        
        // Step 2: Clean up all subscriptions for this module
        println!("[ModuleRegistry] Cleaning up subscriptions for module: {}", name);
        let mut subscribers_guard = self.bus.inner.subscribers.write().await;
        let mut cleaned_types = Vec::new();
        
        for (msg_type, subscribers) in subscribers_guard.iter_mut() {
            let before = subscribers.len();
            subscribers.retain(|s| s != name);
            let after = subscribers.len();
            
            if before != after {
                cleaned_types.push(*msg_type);
                println!("  - Removed subscription to {:?}", msg_type);
            }
        }
        
        // Remove empty subscriber lists
        subscribers_guard.retain(|_, subscribers| !subscribers.is_empty());
        drop(subscribers_guard);
        
        println!("[ModuleRegistry] Unregistered module: {}", name);
        Ok(())
    }

    /// Returns list of all registered module names
    pub async fn list_modules(&self) -> Vec<String> {
        let modules_guard = self.modules.read().await;
        modules_guard.keys().cloned().collect()
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
/// - target: "all" or specific module name
/// - content: String payload
#[derive(Clone)]
pub struct SystemMessage {
    pub source: String,
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
// 1. Receives (Priority, MessageEnvelope) from merged priority channels
// 2. Gets list of subscribed modules from MessageBus
// 3. Spawns a concurrent task for each subscriber
// 4. Waits for all subscribers to process the message
// 5. Logs any errors from subscriber processing
//
// CONCURRENCY MODEL:
// - Each subscriber processes messages in parallel (tokio::spawn per message)
// - Backpressure: Channel capacity limits memory usage
// - Error isolation: One module's error doesn't affect others
async fn run_message_dispatcher(
    registry: Arc<ModuleRegistry>,
    bus: Arc<MessageBus>,
    message_type: TypeId,
    mut receiver: mpsc::Receiver<MessageEnvelope>,
) {
    println!("[Dispatcher] Started for message type: {:?}", message_type);
    
    let message_count = Arc::new(AtomicUsize::new(0));
    
    while let Some(envelope) = receiver.recv().await {
        let msg_id = message_count.fetch_add(1, Ordering::SeqCst);
        let subscribers = bus.get_subscribers(&envelope.message_type).await;
        
        if subscribers.is_empty() {
            eprintln!("[Dispatcher] Warning: Message {} has no subscribers (type: {:?})", msg_id, message_type);
            continue;
        }
        
        // Channel for collecting results from all subscribers
        let (tx, mut rx) = mpsc::channel(subscribers.len());
        
        // Spawn concurrent tasks for each subscriber
        for module_name in subscribers {
            let tx_clone = tx.clone();
            let envelope_clone = envelope.clone_arc();
            let registry_clone = registry.clone();
            
            tokio::spawn(async move {
                let modules_guard = registry_clone.modules.read().await;
                if let Some(module) = modules_guard.get(&module_name) {
                    let result = module.process_message(envelope_clone).await;
                    drop(modules_guard);
                    let _ = tx_clone.send((module_name.clone(), result)).await;
                }
            });
        }
        
        drop(tx);  // Close sender so receiver knows when all are done
        
        // Wait for all subscribers to complete (backpressure)
        while let Some((module_name, result)) = rx.recv().await {
            if let Err(e) = result {
                eprintln!("[Dispatcher] Module {} error processing message {}: {}", module_name, msg_id, e);
            }
        }
    }
    
    println!("[Dispatcher] Stopped for message type: {:?}", message_type);
}

// ==============================================================================
// MAIN APPLICATION ENTRY POINT - Pure Framework Layer
// ==============================================================================
// FRAMEWORK BOOTSTRAPPING SEQUENCE - Framework only provides infrastructure,
//                                    zero business logic
//
// Framework Core Responsibilities (Strict Adherence):
// 1. Setup panic handler for error isolation
// 2. Parse command line arguments (--test mode)
// 3. Create MessageBus and ModuleRegistry - Infrastructure initialization
// 4. Auto-discover and register all modules via inventory - Compile-time discovery
// 5. Register built-in SystemMessage type - Built-in message type registration
// 6. Send initialization test message - System test
// 7. Wait for exit signal (Ctrl+C or test timeout) - Unified lifecycle management
// 8. Graceful shutdown: unregister all modules - Graceful shutdown
//
// [Framework Design Golden Rules]
// - Framework never distinguishes module types (GUI/CLI/Background service)
// - Framework never calls module private APIs
// - Framework never hardcodes specific modules
// - All modules decide independently in initialize() whether to start blocking loops
// - GUI modules use tokio::task::spawn_blocking internally, framework is unaware

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Panic handler prevents module crashes from bringing down the entire system
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("[Panic Handler] Caught panic: {}", panic_info);
    }));

    // Command line arguments
    let args: Vec<String> = std::env::args().collect();
    let is_test_mode = args.contains(&"--test".to_string());

    println!("=== Vibe_Synapse Framework Starting ===");
    
    // Create core framework components
    let bus = MessageBus::new();
    let registry = ModuleRegistry::new(bus.clone());
    
    // Auto-discover and register all modules
    // This uses inventory to find all modules that called module_init!()
    registry.register_all_modules().await?;
    
    // Confirm framework mode
    if is_test_mode {
        println!("\n=== Vibe_Synapse Framework Test Running ===");
    } else {
        println!("\n=== Vibe_Synapse Framework Running ===");
    }
    
    // List all registered modules for debugging
    let modules = registry.list_modules().await;
    if modules.is_empty() {
        println!("Warning: No modules registered!");
    } else {
        println!("Registered modules: {:?}", modules);
    }
    
    // Register built-in SystemMessage type
    println!("[Main] Registering built-in SystemMessage type...");
    bus.register_message_type::<SystemMessage>().await;
    println!("[Main] SystemMessage type registered, dispatcher auto-started");
    
    // Send test message to verify message system
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    println!("\n--- Testing message system ---");
    match bus.publish(SystemMessage {
        source: "main".to_string(),
        target: "all".to_string(),
        content: "System initialized and ready".to_string(),
    }).await {
        Ok(()) => println!("[Main] Published initialization message"),
        Err(e) => eprintln!("[Main] Failed to publish: {}", e),
    }
    
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Main execution - framework waits for exit signal
    // [Framework Layer Responsibility] Framework never distinguishes module types
    // All modules decide independently in initialize() whether to start blocking loops
    // GUI modules use tokio::task::spawn_blocking internally
    if is_test_mode {
        // Test mode: Run for 60 seconds then exit
        println!("\n=== Test Mode - Framework will run for 60 seconds ===");
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        println!("\n=== Test completed ===");
    } else {
        // Normal mode: Wait for exit signal (Ctrl+C or GUI closed)
        // Framework handles all modules uniformly, no special branches for specific modules
        println!("\n=== Framework Running ===");
        println!("Press Ctrl+C to exit...");
        
        // Create exit signal channel for GUI to notify exit
        let (exit_tx, mut exit_rx) = watch::channel(false);
        registry.set_exit_sender(exit_tx).await;
        
        // Wait for either Ctrl+C or GUI exit signal
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n[Main] Ctrl+C received, shutting down...");
            }
            _ = exit_rx.changed() => {
                if *exit_rx.borrow() {
                    println!("\n[Main] GUI closed, shutting down...");
                }
            }
        }
    }
    
    // Graceful shutdown
    println!("\n=== Vibe_Synapse Framework Shutting Down ===");
    
    for module_name in modules {
        if let Err(e) = registry.unregister_module(&module_name).await {
            eprintln!("[Main] Error unregistering module {}: {}", module_name, e);
        }
    }
    
    println!("[Main] Shutdown complete");
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
        
        println!("[MyModule] Initialized");
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
        println!("[MyModule] Shutting down");
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
//            println!("Received: {}", msg.data);
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
// □ Any compilation errors? (run cargo check)
//
// [Messages not received?]
// □ Did you call bus.register_message_type::<M>().await?
// □ Did you call bus.subscribe(type_id, ...).await?
// □ Does message type match? (Sender and receiver use same type)
// □ Does subscriber name match module name() return value?
//
// [Module initialization failing?]
// □ Is initialize() blocking? (Should not block; use spawn_blocking for GUI/IO)
// □ Any unwrap() causing panic?
// □ Is module name unique? (Cannot duplicate other modules)
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
//
// ==============================================================================
// HAPPY CODING! The framework handles all the boilerplate, just focus on your
// business logic!
// ==============================================================================
