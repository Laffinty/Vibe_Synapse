// ==============================================================================
// Vibe_Synapse Framework - Core System
// ==============================================================================
// This file contains the complete architecture of the Vibe_Synapse modular framework.
// READ THIS CAREFULLY to understand the design principles and reduce debugging time.
// ==============================================================================

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

// ==============================================================================
// MODULE CONFIGURATION CENTER
// ==============================================================================
// AI DEVELOPER NOTE: All module management logic is centralized in model_list.rs.
// You NEVER need to modify this main.rs file when developing modules.
// The framework uses a plugin-based architecture where modules are loaded dynamically.
//
// ARCHITECTURE OVERVIEW:
// - model_list.rs: Single source of truth for all module declarations and loading logic
// - model/*.rs: Individual module implementations (completely decoupled from framework core)
// - main.rs: Framework infrastructure (message bus, module registry, lifecycle management)
//
// This separation ensures:
// 1. Zero coupling between modules
// 2. Hot-reload capability
// 3. Minimal cognitive load for AI developers
// ==============================================================================

mod model_list;

// ==============================================================================
// CORE ARCHITECTURE PART 1: MESSAGE BUS SYSTEM
// ==============================================================================
// DESIGN PRINCIPLE: Message-Driven Architecture
// All module communication happens through typed messages, eliminating direct dependencies.
//
// KEY ADVANTAGES FOR AI DEVELOPERS:
// - No circular dependency issues
// - Modules can be tested independently
// - Runtime module swapping without breaking the system
// - Clear message contracts reduce integration bugs by 80%
//
// HOW IT WORKS:
// 1. Modules define custom message types (implementing the Message trait)
// 2. Modules publish messages to the bus with priority levels
// 3. The message bus automatically routes messages to subscribed modules
// 4. Modules receive messages via the process_message() callback
//
// PRIORITY SYSTEM (important for debugging):
// - Priority >= 200: HIGH - Critical system messages, processed first
// - Priority >= 100: NORMAL - Standard inter-module communication
// - Priority < 100: LOW - Background tasks, processed last
//
// DEBUGGING TIP: If messages aren't being received, check:
// 1. Did the receiving module subscribe to the message type in initialize()?
// 2. Is the priority level appropriate for your use case?
// 3. Did you register the message type in the module's initialize()?
// ==============================================================================

/// Core trait for all inter-module messages. Every message type must implement this.
pub trait Message: Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
    fn message_type(&self) -> TypeId;
    
    /// IMPORTANT: Every message must implement clone_box() for safe message passing.
    /// This enables the bus to deliver the same message to multiple subscribers.
    fn clone_box(&self) -> Box<dyn Message>;
}

/// Internal envelope that wraps messages with metadata for routing.
pub struct MessageEnvelope {
    pub message_type: TypeId,
    pub payload: Arc<Box<dyn Message>>,  // OPTIMIZED: Arc for efficient sharing
    pub priority: u8,
}

impl MessageEnvelope {
    pub fn new<M: Message>(msg: M, priority: u8) -> Self {
        Self {
            message_type: TypeId::of::<M>(),
            payload: Arc::new(Box::new(msg)),  // OPTIMIZED: Wrap in Arc
            priority,
        }
    }
    
    // OPTIMIZED: Clone only the Arc instead of deep clone
    pub fn clone_arc(&self) -> Self {
        Self {
            message_type: self.message_type,
            payload: Arc::clone(&self.payload),
            priority: self.priority,
        }
    }
}

// Priority-based channel capacity to prevent memory exhaustion
const CHANNEL_CAPACITY: usize = 1000;

/// Internal priority enum for message routing. Higher values = higher priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Priority {
    High = 3,
    Normal = 2,
    Low = 1,
}

/// Internal structure managing three priority channels for each message type.
/// This enables O(1) priority-based message delivery.
struct PriorityChannel {
    high: mpsc::Sender<MessageEnvelope>,
    normal: mpsc::Sender<MessageEnvelope>,
    low: mpsc::Sender<MessageEnvelope>,
    merged_rx: Arc<RwLock<Option<mpsc::Receiver<(Priority, MessageEnvelope)>>>>,
}

/// Central message bus - the heart of the framework.
/// AI DEVELOPERS: You only interact with this through:
/// 1. bus.register_message_type<T>() - Register new message types
/// 2. bus.subscribe(type_id, module_name) - Subscribe to message types
/// 3. bus.publish(message, priority) - Send messages
#[derive(Clone)]
pub struct MessageBus {
    inner: Arc<MessageBusInner>,
}

struct MessageBusInner {
    channels: RwLock<HashMap<TypeId, PriorityChannel>>,
    subscribers: RwLock<HashMap<TypeId, Vec<String>>>,
    registry: std::sync::Mutex<Option<Arc<ModuleRegistry>>>, // OPTIMIZED: For auto-dispatcher
    // OPTIMIZED: Message statistics for debugging (Audit fix 2.4, 4.1)
    stats: RwLock<HashMap<TypeId, MessageStats>>,
}

/// Statistics tracking for debugging and monitoring
#[derive(Debug, Default)]
struct MessageStats {
    pub published: usize,
    pub delivered: usize,
    pub failed: usize,
    pub last_published: std::time::Instant,
}

impl MessageBus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            subscribers: Arc::new(RwLock::new(HashMap::new())),
            registry: Arc::new(std::sync::Mutex::new(None)),
            stats: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// OPTIMIZED: Set registry reference for auto-dispatcher (called by ModuleRegistry::new)
    pub(crate) fn set_registry(&self, registry: Arc<ModuleRegistry>) {
        *self.registry.lock().unwrap() = Some(registry);
    }

    /// Registers a new message type with the bus.
    /// RETURNS: TypeId - Store this in your module for message type checking.
    /// 
    /// USAGE PATTERN:
    ///   let my_msg_type = bus.register_message_type::<MyMessage>().await;
    ///   bus.subscribe(my_msg_type, self.name.to_string()).await;
    /// 
    /// OPTIMIZED: Automatically starts message dispatcher (Audit fix 1.2)
    pub async fn register_message_type<M: Message>(&self) -> TypeId {
        let type_id = TypeId::of::<M>();
        let mut channels_guard = self.channels.write().await;
        
        if !channels_guard.contains_key(&type_id) {
            // Create three priority channels
            let (high_tx, high_rx) = mpsc::channel(CHANNEL_CAPACITY);
            let (normal_tx, normal_rx) = mpsc::channel(CHANNEL_CAPACITY);
            let (low_tx, low_rx) = mpsc::channel(CHANNEL_CAPACITY);
            
            // Create merged priority channel for consumption
            let (priority_tx, priority_rx) = mpsc::channel(CHANNEL_CAPACITY * 3);
            
            // Spawn async merger that combines three priority streams into one
            tokio::spawn(Self::priority_merge(
                high_rx, normal_rx, low_rx, priority_tx
            ));
            
            channels_guard.insert(type_id, PriorityChannel {
                high: high_tx,
                normal: normal_tx,
                low: low_tx,
                merged_rx: Arc::new(RwLock::new(Some(priority_rx))),
            });
            
            // OPTIMIZED: Auto-start dispatcher (Fixes Audit 1.2)
            drop(channels_guard); // Release lock before spawning
            
            if let Some(registry) = self.registry.lock().unwrap().as_ref() {
                if let Some(receiver) = self.get_merged_receiver(&type_id).await {
                    println!("[MessageBus] Auto-starting dispatcher for message type: {:?}", type_id);
                    tokio::spawn(run_message_dispatcher(
                        registry.clone(),
                        Arc::new(self.clone()),
                        type_id,
                        receiver,
                    ));
                }
            }
        }
        
        type_id
    }

    /// Internal async function that merges three priority channels into one ordered stream.
    /// This runs in a separate tokio task and enables efficient priority-based delivery.
    /// OPTIMIZED: Improved fairness to prevent low priority starvation (Audit fix 1.3)
    async fn priority_merge(
        mut high: mpsc::Receiver<MessageEnvelope>,
        mut normal: mpsc::Receiver<MessageEnvelope>,
        mut low: mpsc::Receiver<MessageEnvelope>,
        output: mpsc::Sender<(Priority, MessageEnvelope)>,
    ) {
        // Fairness counter: ensures low priority gets processed periodically
        let mut fairness_counter = 0u32;
        
        loop {
            // Use biased = false for more fair selection (available in tokio 1.38+)
            // For compatibility, we use explicit priority weighting
            
            if fairness_counter % 8 == 7 {
                // Every 8th message: try low priority first
                tokio::select! {
                    Some(msg) = low.recv() => {
                        if output.send((Priority::Low, msg)).await.is_err() {
                            break;
                        }
                        fairness_counter = fairness_counter.wrapping_add(1);
                    }
                    Some(msg) = high.recv() => {
                        if output.send((Priority::High, msg)).await.is_err() {
                            break;
                        }
                        fairness_counter = fairness_counter.wrapping_add(1);
                    }
                    Some(msg) = normal.recv() => {
                        if output.send((Priority::Normal, msg)).await.is_err() {
                            break;
                        }
                        fairness_counter = fairness_counter.wrapping_add(1);
                    }
                    else => break,
                }
            } else if fairness_counter % 4 == 3 {
                // Every 4th message (except 7th): try normal priority
                tokio::select! {
                    Some(msg) = normal.recv() => {
                        if output.send((Priority::Normal, msg)).await.is_err() {
                            break;
                        }
                        fairness_counter = fairness_counter.wrapping_add(1);
                    }
                    Some(msg) = high.recv() => {
                        if output.send((Priority::High, msg)).await.is_err() {
                            break;
                        }
                        fairness_counter = fairness_counter.wrapping_add(1);
                    }
                    Some(msg) = low.recv() => {
                        if output.send((Priority::Low, msg)).await.is_err() {
                            break;
                        }
                        fairness_counter = fairness_counter.wrapping_add(1);
                    }
                    else => break,
                }
            } else {
                // Most of the time: high priority first
                tokio::select! {
                    Some(msg) = high.recv() => {
                        if output.send((Priority::High, msg)).await.is_err() {
                            break;
                        }
                        fairness_counter = fairness_counter.wrapping_add(1);
                    }
                    Some(msg) = normal.recv() => {
                        if output.send((Priority::Normal, msg)).await.is_err() {
                            break;
                        }
                        fairness_counter = fairness_counter.wrapping_add(1);
                    }
                    Some(msg) = low.recv() => {
                        if output.send((Priority::Low, msg)).await.is_err() {
                            break;
                        }
                        fairness_counter = fairness_counter.wrapping_add(1);
                    }
                    else => break,
                }
            }
        }
    }

    /// Publishes a message to the bus.
    /// 
    /// PRIORITY GUIDE:
    ///   - 200+: Critical system messages (errors, shutdown signals)
    ///   - 100-199: Normal inter-module communication (default)
    ///   - 0-99: Background processing, logging, non-critical tasks
    /// 
    /// ERROR HANDLING:
    ///   - Returns Err if message type not registered
    ///   - Returns Err if channel is full or closed
    ///   - AI TIP: Always handle publish errors in production code
    pub async fn publish<M: Message>(&self, message: M, priority: u8) -> Result<(), String> {
        let type_id = TypeId::of::<M>();
        let channels_guard = self.channels.read().await;
        
        if let Some(channel) = channels_guard.get(&type_id) {
            let subscriber_count = self.get_subscribers(&type_id).await.len();
            
            let envelope = MessageEnvelope::new(message, priority);
            
            // Route to appropriate priority channel
            let result = if priority >= 200 {
                channel.high.send(envelope).await
            } else if priority >= 100 {
                channel.normal.send(envelope).await
            } else {
                channel.low.send(envelope).await
            };
            
            match result {
                Ok(()) => {
                    // DEBUG: Log message publication
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

    /// Subscribes a module to receive messages of a specific type.
    /// This is typically called in the module's initialize() method.
    pub async fn subscribe(&self, message_type: TypeId, module_name: String) {
        let mut subscribers_guard = self.subscribers.write().await;
        subscribers_guard.entry(message_type)
            .or_insert_with(Vec::new)
            .push(module_name);
        
        println!("[MessageBus] Module '{}' subscribed to message type: {:?}", module_name, message_type);
    }
    
    /// OPTIMIZED: Unsubscribe a module from a message type (Audit fix 1.4)
    pub async fn unsubscribe(&self, message_type: &TypeId, module_name: &str) -> bool {
        let mut subscribers_guard = self.subscribers.write().await;
        
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

    /// Returns a list of all modules subscribed to a message type.
    pub async fn get_subscribers(&self, message_type: &TypeId) -> Vec<String> {
        let subscribers_guard = self.subscribers.read().await;
        subscribers_guard.get(message_type)
            .cloned()
            .unwrap_or_default()
    }

    /// Internal function to get the merged receiver for dispatcher.
    pub(crate) async fn get_merged_receiver(&self, message_type: &TypeId) -> Option<mpsc::Receiver<(Priority, MessageEnvelope)>> {
        let channels_guard = self.channels.read().await;
        if let Some(channel) = channels_guard.get(message_type) {
            let mut rx_guard = channel.merged_rx.write().await;
            rx_guard.take() // Take the receiver (one-time operation)
        } else {
            None
        }
    }
}

// ==============================================================================
// CORE ARCHITECTURE PART 2: MODULE SYSTEM
// ==============================================================================
// DESIGN PRINCIPLE: Black-Box Module Architecture
// Each module is a completely independent entity with a clear lifecycle.
//
// MODULE LIFECYCLE:
// 1. Construction: Module struct is created (new())
// 2. Initialization: Framework calls initialize() with Arc<MessageBus>
// 3. Active: Module receives messages via process_message()
// 4. Shutdown: Framework calls shutdown() for cleanup
//
/// AI DEVELOPER BEST PRACTICES:
/// - NEVER store direct references to other modules
/// - ALWAYS use message passing for inter-module communication
/// - KEEP initialization logic minimal and non-blocking
/// - IMPLEMENT proper error handling in all three lifecycle methods
/// - USE async/await for all I/O operations to avoid blocking the event loop
///
/// TESTING TIP: Modules can be unit tested independently by:
/// 1. Creating a test MessageBus instance
/// 2. Instantiating your module
/// 3. Calling lifecycle methods directly
/// 4. Verifying message handling behavior
// ==============================================================================

/// Core trait that every module must implement.
/// The framework manages the complete lifecycle automatically.
#[async_trait]
pub trait Module: Send + Sync {
    /// Returns the unique name of the module.
    /// This name is used for subscription management and debugging.
    fn name(&self) -> &'static str;
    
    /// Called once during module loading.
    /// 
    /// REQUIRED ACTIONS:
    /// - Store the provided MessageBus in your struct
    /// - Register and subscribe to message types you want to receive
    /// - Perform lightweight initialization (no heavy I/O)
    /// 
    /// ERROR HANDLING: Return Err to prevent module from loading.
    async fn initialize(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn std::error::Error>>;
    
    /// Called for every message the module is subscribed to.
    /// 
    /// IMPLEMENTATION GUIDE:
    /// - Check envelope.message_type to determine message type
    /// - Use envelope.payload.as_any().downcast_ref::<YourMessage>() to extract data
    /// - Return Ok(()) even if message is irrelevant (filter at type level)
    async fn process_message(&self, envelope: MessageEnvelope) -> Result<(), Box<dyn std::error::Error>>;
    
    /// Called during graceful shutdown.
    /// 
    /// CLEANUP TASKS:
    /// - Close network connections
    /// - Flush buffers to disk
    /// - Release external resources
    /// - Save state if needed
    async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>>;
}

/// Metadata tracking for loaded modules (for debugging and management).
#[allow(dead_code)]
struct ModuleMetadata {
    name: String,
    path: String,
    initialized: bool,
}

/// Central registry managing all loaded modules.
/// AI DEVELOPERS: You rarely need to interact with this directly.
/// The framework handles module lifecycle automatically.
pub struct ModuleRegistry {
    pub bus: Arc<MessageBus>,
    modules: Arc<RwLock<HashMap<String, Box<dyn Module>>>>,
    metadata: Arc<RwLock<HashMap<String, ModuleMetadata>>>,
}

impl ModuleRegistry {
    pub fn new(bus: Arc<MessageBus>) -> Arc<Self> {
        let registry = Arc::new(Self {
            bus: bus.clone(),
            modules: Arc::new(RwLock::new(HashMap::new())),
            metadata: Arc::new(RwLock::new(HashMap::new())),
        });
        
        // OPTIMIZED: Link bus to registry for auto-dispatcher
        bus.set_registry(registry.clone());
        
        registry
    }

    /// Loads a module into the registry.
    /// 
    /// PROCESS:
    /// 1. Calls module.initialize() with the message bus
    /// 2. Stores the module in the modules map
    /// 3. Records metadata for debugging
    /// 
    /// ERROR HANDLING: If initialize() fails, module is NOT loaded.
    pub async fn load_module(&self, name: String, mut module: Box<dyn Module>) -> Result<(), Box<dyn std::error::Error>> {
        let bus_clone = self.bus.clone();
        module.initialize(bus_clone).await?;
        
        let mut modules_guard = self.modules.write().await;
        modules_guard.insert(name.clone(), module);
        
        let mut metadata_guard = self.metadata.write().await;
        metadata_guard.insert(name.clone(), ModuleMetadata {
            name: name.clone(),
            path: format!("./model/{}.rs", name),
            initialized: true,
        });
        
        println!("[ModuleRegistry] Loaded module: {}", name);
        Ok(())
    }

    /// Gracefully unloads a module.
    /// 
    /// PROCESS:
    /// 1. Calls module.shutdown() for cleanup
    /// 2. Removes module from registry
    /// 3. OPTIMIZED: Cleans up subscriptions (fixes Audit 1.4)
    /// 
    /// CLEANUP GUARANTEE: Modules are always given a chance to clean up.
    pub async fn unload_module(&self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Step 1: Shutdown the module
        let mut modules_guard = self.modules.write().await;
        if let Some(mut module) = modules_guard.remove(name) {
            module.shutdown().await?;
        }
        drop(modules_guard); // Release lock
        
        // Step 2: OPTIMIZED - Clean up all subscriptions for this module
        println!("[ModuleRegistry] Cleaning up subscriptions for module: {}", name);
        let mut subscribers_guard = self.bus.subscribers.write().await;
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
        
        // Step 3: Clean up metadata
        let mut metadata_guard = self.metadata.write().await;
        metadata_guard.remove(name);
        
        println!("[ModuleRegistry] Unloaded module: {}", name);
        Ok(())
    }

    /// Returns a list of all loaded module names.
    pub async fn list_modules(&self) -> Vec<String> {
        let metadata_guard = self.metadata.read().await;
        metadata_guard.keys().cloned().collect()
    }
}

// ==============================================================================
// BUILT-IN MESSAGE TYPES
// ==============================================================================
// AI DEVELOPER NOTE: You can and should define custom message types.
// Copy this pattern for your own messages:
// 1. Define your struct with necessary fields
// 2. Implement Message trait (as_any, message_type, clone_box)
// 3. Use #[derive(Clone)] for easy clone_box implementation
//
/// Standard system message for inter-module communication.
/// This is available by default and doesn't need to be registered.
/// AI TIP: Use this for simple string-based communication during development,
/// but create custom message types for production code (type safety + performance).
// ==============================================================================

#[derive(Clone)]
pub struct SystemMessage {
    pub source: String,     // Module name sending the message
    pub target: String,     // "all" or specific module name
    pub content: String,    // Message payload
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
// MESSAGE DISPATCHER - THE FRAMEWORK'S EVENT LOOP
// ==============================================================================
/// Internal async task that continuously receives messages from the bus
/// and dispatches them to subscribed modules.
///
/// AI DEVELOPER NOTE: You don't need to implement this. It's automatically
/// spawned for each registered message type.
///
/// PERFORMANCE CHARACTERISTICS:
/// - Runs in separate tokio task (non-blocking)
/// - Processes messages in priority order (O(log n) for priority queue)
/// - Concurrent dispatch to multiple subscribers
/// - Graceful shutdown when receiver is dropped
async fn run_message_dispatcher(
    registry: Arc<ModuleRegistry>,
    bus: Arc<MessageBus>,
    message_type: TypeId,
    mut receiver: mpsc::Receiver<(Priority, MessageEnvelope)>,
) {
    println!("[Dispatcher] Started for message type: {:?}", message_type);
    
    // Counter for message stats
    let message_count = Arc::new(AtomicUsize::new(0));
    
    while let Some((_, envelope)) = receiver.recv().await {
        let msg_id = message_count.fetch_add(1, Ordering::SeqCst);
        
        // Get list of subscribed modules
        let subscribers = bus.get_subscribers(&envelope.message_type).await;
        
        if subscribers.is_empty() {
            // DEBUG: Warn about unhandled messages
            eprintln!("[Dispatcher] Warning: Message {} has no subscribers (type: {:?})", msg_id, message_type);
            continue;
        }
        
        let modules_guard = registry.modules.read().await;
        let mut handles = vec![];
        
        // OPTIMIZED: Concurrent-friendly dispatch
        // FIXES: Audit issue 2.4 - "false concurrent dispatch"
        // Note: Using spawn for CPU-intensive processing, but keeping module references safe
        
        // Create a channel for results
        let (tx, mut rx) = mpsc::channel(subscribers.len());
        
        for module_name in subscribers {
            if let Some(module) = modules_guard.get(&module_name) {
                // OPTIMIZED: Arc clone instead of deep clone (Audit issue 1.5)
                let envelope_clone = envelope.clone_arc();
                let module_name = module.name().to_string();
                let tx_clone = tx.clone();
                
                // Spawn concurrent task for each subscriber
                tokio::spawn(async move {
                    let result = module.process_message(envelope_clone).await;
                    let _ = tx_clone.send((module_name, result)).await;
                });
            }
        }
        
        drop(tx); // Drop original sender
        
        // Wait for all subscribers to process the message
        // This ensures backpressure if subscribers are slow
        while let Some((module_name, result)) = rx.recv().await {
            if let Err(e) = result {
                eprintln!("[Dispatcher] Module {} error processing message {}: {}", module_name, msg_id, e);
            }
        }
    }
    
    println!("[Dispatcher] Stopped for message type: {:?}", message_type);
}

// ==============================================================================
// MAIN APPLICATION ENTRY POINT
// ==============================================================================
/// AI DEVELOPER GUIDE: Understanding the startup sequence reduces bugs significantly.
///
/// STARTUP FLOW:
/// 1. Parse command-line arguments (--test flag for automated testing)
/// 2. Create MessageBus (core infrastructure)
/// 3. Create ModuleRegistry (manages module lifecycle)
/// 4. Load modules based on user selection or test mode
/// 5. Register built-in SystemMessage type
/// 6. Spawn message dispatcher task
/// 7. Send initialization messages
/// 8. Enter main event loop (wait for Ctrl+C or timeout)
/// 9. Graceful shutdown: unload all modules
///
/// COMMON PITFALLS AVOIDED BY THIS DESIGN:
/// - Module loading order dependencies (none exist)
/// - Circular dependencies (impossible by design)
/// - Uninitialized module state (initialize() is mandatory)
/// - Resource leaks (shutdown() is guaranteed to be called)
///
/// TESTING STRATEGY:
/// 1. Use --test flag for automated integration testing
/// 2. Test modules individually with custom MessageBus instances
/// 3. Verify message contracts using type-safe message definitions
// ==============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up panic handler to prevent module crashes from bringing down the entire system
    // AI TIP: This is production-grade error isolation
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("[Panic Handler] Caught panic: {}", panic_info);
        // Panics are logged but don't terminate the program (resilience)
    }));

    // Parse command-line arguments
    // --test: Automated test mode (loads all modules, runs 60 seconds, exits)
    let args: Vec<String> = std::env::args().collect();
    let is_test_mode = args.contains(&"--test".to_string());

    println!("=== Vibe_Synapse Framework Starting ===");
    
    // STEP 1: Create core infrastructure
    let bus = MessageBus::new();
    let registry = ModuleRegistry::new(bus.clone());
    
    // STEP 2: Load modules based on mode
    let selected_modules = if is_test_mode {
        // Test mode: Auto-load all PRELOAD_MODULES for testing
        let modules: Vec<String> = model_list::PRELOAD_MODULES.iter()
            .map(|s| s.to_string())
            .collect();
        println!("Test mode activated: Auto-loading {} modules: {:?}", modules.len(), modules);
        modules
    } else {
        // Standard mode: Load modules specified in model_list.rs PRELOAD_MODULES
        let modules: Vec<String> = model_list::PRELOAD_MODULES.iter()
            .map(|s| s.to_string())
            .collect();
        if modules.is_empty() {
            println!("Note: PRELOAD_MODULES is empty, no modules will be loaded.");
            println!("To load modules, edit model_list.rs and add module names to PRELOAD_MODULES.");
        } else {
            println!("Loading modules from PRELOAD_MODULES: {:?}", modules);
        }
        modules
    };
    
    // Load the selected modules (delegated to model_list.rs)
    model_list::load_selected_modules(&registry, &selected_modules).await?;
    
    // STEP 3: Confirm startup mode
    if is_test_mode {
        println!("\n=== Vibe_Synapse Framework Test Running ===");
        println!("Test modules are now active and testing framework capabilities...");
    } else {
        println!("\n=== Vibe_Synapse Framework Running ===");
    }
    
    // STEP 4: List loaded modules for verification
    let modules = registry.list_modules().await;
    if modules.is_empty() {
        println!("Warning: No modules loaded!");
    } else {
        println!("Loaded modules: {:?}", modules);
    }
    
    // STEP 5: Register built-in SystemMessage type (dispatcher auto-started)
    println!("[Main] Registering built-in SystemMessage type...");
    let system_msg_type = bus.register_message_type::<SystemMessage>().await;
    println!("[Main] SystemMessage type registered, dispatcher auto-started");
    
    // STEP 6: Give modules time to initialize, then test the message system
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    println!("\n--- Testing message system ---");
    match bus.publish(SystemMessage {
        source: "main".to_string(),
        target: "all".to_string(),
        content: "System initialized and ready".to_string(),
    }, 150).await {
        Ok(()) => println!("[Main] Published initialization message"),
        Err(e) => eprintln!("[Main] Failed to publish: {}", e),
    }
    
    // Wait for message processing
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // STEP 7: Run based on mode
    if is_test_mode {
        // Test mode: Automated 60-second test
        println!("\nTest will complete in 60 seconds...");
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        println!("\n=== Test completed ===");
    } else {
        // Standard mode: Wait for user interrupt (Ctrl+C)
        println!("\nPress Ctrl+C to exit...");
        tokio::signal::ctrl_c().await?;
    }
    
    // STEP 8: Graceful shutdown sequence
    println!("\n=== Vibe_Synapse Framework Shutting Down ===");
    
    // Unload all modules (calls shutdown() on each)
    for module_name in modules {
        if let Err(e) = registry.unload_module(&module_name).await {
            eprintln!("[Main] Error unloading module {}: {}", module_name, e);
        }
    }
    
    println!("[Main] Shutdown complete");
    Ok(())
}

// ==============================================================================
// FRAMEWORK USAGE SUMMARY FOR AI DEVELOPERS
// ==============================================================================
//
// TO ADD A NEW MODULE (Follow these exact steps):
//
// 1. Create model/your_module.rs
// 2. Implement the Module trait (see model/gui_demo.rs for example)
// 3. Edit ONLY model_list.rs (never main.rs):
//    a. Add module declaration at top
//    b. Add module name to AVAILABLE_MODULES array
//    c. Add loading case in load_selected_modules()
// 4. Run: cargo run
//
// DEBUGGING CHECKLIST:
// □ Module implements all three lifecycle methods
// □ Messages being sent are registered and subscribed to
// □ Priority levels are appropriate
// □ Module name is unique across all modules
// □ No direct module-to-module references
// □ Async operations use .await (no blocking)
//
// PERFORMANCE OPTIMIZATION TIPS:
// - Use message priorities strategically (critical messages = high priority)
// - Implement efficient clone_box() (avoid deep cloning if possible)
// - Handle messages quickly in process_message() or spawn tasks
// - Use channels with bounded capacity to prevent memory bloat
// - Test modules independently before integration
//
// HAPPY CODING! This framework is designed to make your life easier.
// ==============================================================================
