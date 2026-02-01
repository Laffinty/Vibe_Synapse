// ==============================================================================
// Simple GUI Module - Single egui Window Demo
// ==============================================================================
// A basic GUI module demonstrating the framework capabilities:
// - Module auto-registration via module_init! macro
// - GUI initialization and lifecycle management
// - Message handling from the framework bus
// - User interaction with buttons and state updates
//
// FEATURES:
// - Displays a window with title "Vibe Synapse - Simple GUI Demo"
// - Shows a clickable button that increments a counter
// - Displays current click count
// - Exit button to close the window gracefully
//
// GUI ARCHITECTURE NOTES:
// - eframe::run_native() must run on main thread (blocks until window closes)
// - Use tokio::task::block_in_place() to integrate with tokio runtime
// - Keep all GUI state in Arc-wrapped fields for thread safety
// - update() method called every frame for UI rendering
//
// MODULE LIFECYCLE:
// 1. Default::default() - Module constructed by inventory system
// 2. initialize() - Store bus reference, prepare state
// 3. run_gui_blocking() - Open GUI window (blocks main thread)
// 4. update() - Render UI each frame, handle user input
// 5. shutdown() - Set is_running=false, wait for cleanup
//
// ⚠️ IMPORTANT SAFETY NOTE:
// This module uses spawn_blocking which runs on a thread pool, NOT the main
// thread. eframe on Windows can work on non-main threads with proper
// event_loop_builder configuration.
// ==============================================================================

use async_trait::async_trait;
use std::sync::Arc;
use crate::{MessageEnvelope, MessageBus, Module};
use std::any::TypeId;
use tokio::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, warn, error};

// ==============================================================================
// Configuration Constants
// ==============================================================================
/// Time to wait for GUI thread to start after spawn_blocking
/// 
/// # Rationale
/// The GUI needs time to initialize before we return from initialize().
/// If this is too short, the module appears initialized but GUI isn't ready.
const GUI_STARTUP_WAIT_MS: u64 = 500;

/// Time between shutdown signal checks during graceful shutdown
const SHUTDOWN_POLL_INTERVAL_MS: u64 = 50;

/// Maximum number of shutdown check iterations before giving up
const SHUTDOWN_MAX_ATTEMPTS: u32 = 100;  // 100 * 50ms = 5 seconds

// ==============================================================================
// Module State Structure
// ==============================================================================
// All fields are Arc-wrapped for thread-safe sharing between async tasks
// and the GUI thread.
//
// # Thread Safety Pattern
// All fields use lock-free atomics or async-aware locks to prevent blocking
// in async contexts.
//
// Fields:
// - name: Unique module identifier (matches module_init! macro)
// - bus: Optional Arc<MessageBus> for framework communication
// - is_running: Atomic flag to control GUI lifecycle (lock-free)
// - click_count: Shared counter for button clicks
// - shutdown_notify: Used for reliable shutdown synchronization
#[derive(Clone)]
pub struct SimpleGuiModule {
    name: &'static str,
    bus: Arc<RwLock<Option<Arc<MessageBus>>>>,
    is_running: Arc<AtomicBool>,
    click_count: Arc<RwLock<usize>>,
    /// Used for reliable shutdown signaling (avoids busy-waiting)
    shutdown_notify: Arc<tokio::sync::Notify>,
}

// ==============================================================================
// GUI Application Implementation
// ==============================================================================
impl SimpleGuiModule {
    /// Creates a new module instance with default state
    /// 
    /// Called by:
    /// - Inventory system during auto-registration
    /// - ModuleRegistry when constructing modules
    /// - Default::default() implementation
    ///
    /// Sets up initial state but does NOT start GUI (see initialize())
    ///
    /// # Invariants
    /// - All Arc fields are newly allocated (no shared state yet)
    /// - is_running starts as false (GUI not started)
    pub fn new() -> Self {
        Self {
            name: "simple_gui",
            bus: Arc::new(RwLock::new(None)),
            is_running: Arc::new(AtomicBool::new(false)),
            click_count: Arc::new(RwLock::new(0)),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

/// Actual egui application that renders the UI
/// 
/// Implements eframe::App trait which requires update() method.
/// update() is called every frame to render the UI and handle events.
///
/// # Threading Model
/// This struct is owned by the eframe event loop (running on spawn_blocking
/// thread). It shares Arc fields with the SimpleGuiModule for coordination.
struct SimpleGuiApp {
    is_running: Arc<AtomicBool>,
    click_count: Arc<RwLock<usize>>,
    label_text: String,
    /// Reference to notify main thread on shutdown (if available)
    shutdown_notify: Option<Arc<tokio::sync::Notify>>,
}

impl eframe::App for SimpleGuiApp {
    /// Called every frame to render the UI
    ///
    /// UI STRUCTURE:
    /// - Vertical layout with centered content
    /// - Heading: "Vibe Synapse GUI Demo"
    /// - Separator line
    /// - Button: "Click me!" - increments counter when clicked
    /// - Label: Shows current click count
    /// - Spacer
    /// - Button: "Exit" - sets is_running=false to close window
    ///
    /// STATE MANAGEMENT:
    /// - Uses blocking_write() to update click_count (held for minimal time)
    /// - Updates label_text immediately after counter change
    /// - Uses is_running atomic flag for exit control
    ///
    /// # Performance
    /// This method is called ~60 times per second. Keep it lightweight:
    /// - No allocations in hot path
    /// - Minimal locking (just the click_count RwLock)
    /// - UI operations are immediate mode (no retained state)
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check if we should exit - signal viewport to close
        // Ordering::SeqCst ensures visibility across threads
        if !self.is_running.load(Ordering::SeqCst) {
            // Send close command to viewport
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        
        // Render UI
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                // Title
                ui.heading("Vibe Synapse Simple GUI Demo");
                ui.separator();
                ui.add_space(20.0);
                
                // Click button
                if ui.button("Click me!").clicked() {
                    // Update counter and label
                    // blocking_write() is OK here because we're on a dedicated GUI thread
                    match self.click_count.try_write() {
                        Ok(mut count) => {
                            *count += 1;
                            self.label_text = format!("You have clicked {} times!", *count);
                        }
                        Err(_) => {
                            // Lock contention - this shouldn't happen with proper design
                            // but we handle it gracefully
                            warn!("Could not acquire click_count lock");
                        }
                    }
                }
                
                ui.add_space(20.0);
                
                // Display counter label
                ui.label(&self.label_text);
                
                ui.add_space(40.0);
                
                // Exit button
                if ui.button("Exit").clicked() {
                    info!("Exit button clicked, signaling shutdown");
                    self.is_running.store(false, Ordering::SeqCst);
                    // Notify waiting shutdown code (if available)
                    if let Some(notify) = &self.shutdown_notify {
                        notify.notify_one();
                    }
                }
            });
        });
        
        // Request repaint for responsive UI (eframe does rate limiting internally)
        // This ensures animations and hover effects work smoothly
        ctx.request_repaint();
    }
    
}

// ==============================================================================
// Default Implementation for Auto-Registration
// ==============================================================================
// The inventory system requires Default trait to construct modules
//
/// Creates default module instance
///
/// Called by:
/// - Inventory system via module_init! macro
/// - ModuleRegistry when auto-registering modules
/// - Any code that uses SimpleGuiModule::default()
impl Default for SimpleGuiModule {
    fn default() -> Self {
        Self::new()
    }
}

// ==============================================================================
// Module Lifecycle Implementation
// ==============================================================================
// REQUIRED TRAIT METHODS:
// - name(): Returns unique module identifier
// - initialize(): Called once after construction, receive Arc<MessageBus>
// - process_message(): Called for each subscribed message
// - shutdown(): Called during graceful shutdown

#[async_trait]
impl Module for SimpleGuiModule {
    /// Returns the module's unique name
    /// 
    /// Must match the second argument to module_init! macro
    /// 
    /// # Uniqueness
    /// This name must be unique across all modules. The framework will warn
    /// if duplicates are detected.
    fn name(&self) -> &'static str {
        self.name
    }
    
    /// Initializes module with bus access and starts the GUI
    /// 
    /// CALLED BY:
    /// - ModuleRegistry::register_all_modules() after module construction
    ///
    /// RESPONSIBILITIES:
    /// - Store Arc<MessageBus> for future message operations
    /// - Start GUI in a blocking task (GUI frameworks typically block the thread)
    /// - Return quickly (don't wait for GUI to fully start)
    ///
    /// FOR GUI MODULES:
    /// - GUI is started via spawn_blocking in initialize() - module controls its own lifecycle
    /// - The spawn_blocking task will run until window is closed
    ///
    /// ERROR HANDLING:
    /// - Return Err to prevent module from loading
    /// - This module will be skipped but others will still load
    /// - Any panics in spawn_blocking are isolated to that task
    ///
    /// # Thread Safety
    /// This method is async and runs on the tokio runtime. The GUI runs on
    /// a separate thread (via spawn_blocking). Communication is via Arc shared state.
    async fn initialize(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("[Simple GUI] Initializing...");
        
        // Store bus reference for potential future message operations
        *self.bus.write().await = Some(bus.clone());
        
        // Start GUI on a blocking thread pool
        // eframe::run_native blocks until the window closes, so we use spawn_blocking
        let is_running = self.is_running.clone();
        let click_count = self.click_count.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        
        // Clone bus for signaling exit when GUI closes
        let bus_for_exit = bus.clone();
        
        info!("[Simple GUI] Starting GUI in spawn_blocking task...");
        self.is_running.store(true, Ordering::SeqCst);
        
        tokio::task::spawn_blocking(move || {
            // SAFETY: We're on a blocking thread, not the async runtime thread
            // This is the correct place to run the GUI event loop
            
            // Configure eframe to allow running on any thread (cross-platform compatibility)
            // This is required for GUI modules running in spawn_blocking
            let mut native_options = eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_title("Vibe Synapse - Simple GUI Demo")
                    .with_inner_size([400.0, 300.0])
                    .with_resizable(true),
                ..Default::default()
            };
            
            // On Windows, we need to configure the event loop to allow any thread
            // Without this, eframe may panic or fail to create window on non-main thread
            #[cfg(windows)]
            {
                use winit::platform::windows::EventLoopBuilderExtWindows;
                native_options.event_loop_builder = Some(Box::new(|builder| {
                    builder.with_any_thread(true);
                }));
            }
            
            // Clone for the closure to avoid move issues
            let is_running_for_app = is_running.clone();
            let shutdown_notify_for_app = Some(shutdown_notify.clone());
            
            // Run the native application - this blocks until window closes
            let result = eframe::run_native(
                "Vibe Synapse Simple GUI",
                native_options,
                Box::new(move |_cc| {
                    Ok(Box::new(SimpleGuiApp {
                        is_running: is_running_for_app,
                        click_count,
                        label_text: "Click the button to start counting".to_string(),
                        shutdown_notify: shutdown_notify_for_app,
                    }))
                }),
            );
            
            // Handle result (errors are logged but don't panic)
            match result {
                Ok(()) => info!("[Simple GUI] GUI window closed normally"),
                Err(e) => {
                    error!("[Simple GUI] GUI error: {:?}", e);
                    // Even on error, we need to signal shutdown
                }
            }
            
            // Signal that GUI has closed
            is_running.store(false, Ordering::SeqCst);
            shutdown_notify.notify_one();
            
            // Signal main thread to exit (for Windows GUI mode)
            // Use try_current() to safely get the runtime handle
            match tokio::runtime::Handle::try_current() {
                Ok(rt) => {
                    // We have a runtime handle, can use block_on
                    rt.block_on(async {
                        info!("[Simple GUI] Signaling framework exit");
                        bus_for_exit.signal_exit().await;
                    });
                }
                Err(e) => {
                    // No runtime available (shouldn't happen in normal operation)
                    warn!("[Simple GUI] Cannot signal exit: no tokio runtime: {}", e);
                }
            }
        });
        
        // Give GUI time to start before returning
        // This is a heuristic - the GUI may not be fully ready yet,
        // but we've done our best effort
        tokio::time::sleep(tokio::time::Duration::from_millis(GUI_STARTUP_WAIT_MS)).await;
        
        // Check if GUI started successfully (is_running should still be true)
        if !self.is_running.load(Ordering::SeqCst) {
            return Err("GUI failed to start".into());
        }
        
        info!("[Simple GUI] Initialization complete - GUI started");
        Ok(())
    }
    
    /// Processes incoming messages from the bus
    /// 
    /// CALLED BY:
    /// - Message dispatcher when subscribed message types are published
    ///
    /// PATTERN:
    /// - Check envelope.message_type for type matching
    /// - Downcast envelope.payload to specific message type
    /// - Handle message if type matches
    /// - Return Ok(()) even if message type is unrelated
    ///
    /// FOR THIS DEMO:
    /// - Subscribes to SystemMessage type
    /// - Logs any system messages received
    /// - Could be extended to handle control messages (e.g., "show notification")
    ///
    /// # Performance
    /// This is called for every SystemMessage published. Keep it fast!
    async fn process_message(&self, envelope: MessageEnvelope) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Handle SystemMessage for potential control messages
        if envelope.message_type == TypeId::of::<crate::SystemMessage>() {
            if let Some(msg) = envelope.payload.as_any().downcast_ref::<crate::SystemMessage>() {
                info!("[Simple GUI] Received system message: {} - {}", msg.source, msg.content);
                
                // Could add handling here, e.g.:
                // - Show notification popup
                // - Update status bar
                // - Change window title based on content
            }
        }
        
        // Could add handling for other message types here
        // Example: GuiMessage for updating specific UI elements
        Ok(())
    }
    
    /// Graceful shutdown of module
    /// 
    /// CALLED BY:
    /// - ModuleRegistry::unregister_module() during framework shutdown
    ///
    /// RESPONSIBILITIES:
    /// - Signal GUI thread to exit (via is_running flag)
    /// - Wait for GUI to fully close (with timeout to avoid hanging)
    /// - Clean up any resources
    /// - Return Ok(()) even if cleanup has issues
    ///
    /// # Shutdown Sequence
    /// 1. Set is_running=false to signal GUI to close
    /// 2. Wait for shutdown_notify (GUI will notify when closed)
    /// 3. Timeout after SHUTDOWN_MAX_ATTEMPTS * SHUTDOWN_POLL_INTERVAL_MS
    ///
    /// # Thread Safety
    /// This is async and runs on tokio runtime. The GUI runs on another thread.
    /// We use the shutdown_notify for reliable cross-thread signaling.
    async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("[Simple GUI] Shutting down...");
        
        // Signal GUI to close
        let was_running = self.is_running.swap(false, Ordering::SeqCst);
        
        if !was_running {
            // GUI was already closing or never started
            info!("[Simple GUI] GUI was already stopped");
            return Ok(());
        }
        
        // Wait for GUI to fully close using Notify (avoids busy-waiting)
        // Use timeout to prevent indefinite waiting
        let wait_result = tokio::time::timeout(
            std::time::Duration::from_millis(SHUTDOWN_MAX_ATTEMPTS as u64 * SHUTDOWN_POLL_INTERVAL_MS),
            self.shutdown_notify.notified()
        ).await;
        
        match wait_result {
            Ok(()) => {
                info!("[Simple GUI] GUI confirmed shutdown");
            }
            Err(_) => {
                warn!("[Simple GUI] Shutdown wait timed out, proceeding anyway");
            }
        }
        
        // Clear bus reference to help with memory cleanup
        *self.bus.write().await = None;
        
        info!("[Simple GUI] Shutdown complete");
        Ok(())
    }
}

// ==============================================================================
// Inventory Auto-Registration
// ==============================================================================
// This single line automatically registers the module with the framework
// at compile time. The inventory system collects all module_init! calls
// and makes them discoverable via inventory::iter::<ModuleBuildInfo>()
// No changes to main.rs or any config files needed.
crate::module_init!(SimpleGuiModule, "simple_gui");

// ==============================================================================
// Example Message Type (for extension)
// ==============================================================================
// If you want to add messages that other modules can send to this GUI,
// define them here and handle them in process_message().
//
/// Demo message that could be sent to update GUI text
/// 
/// USAGE EXAMPLE:
/// ```rust
/// bus.publish(GuiMessage { 
///     content: "Hello from another module!".to_string() 
/// }).await?;
/// ```
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct GuiMessage {
    pub content: String,
}

/// Implement Message trait to enable bus communication
impl crate::Message for GuiMessage {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn message_type(&self) -> TypeId {
        TypeId::of::<GuiMessage>()
    }
    
    fn clone_box(&self) -> Box<dyn crate::Message> {
        Box::new(self.clone())
    }
}

// ==============================================================================
// Extension Ideas for AI Developers
// ==============================================================================
//
// 1. Add more UI elements:
//    - Text input fields
//    - Sliders for numeric values
//    - Checkboxes for boolean flags
//    - Dropdown menus for selections
//
// 2. Handle messages from other modules:
//    - Remove #[allow(dead_code)] from GuiMessage
//    - In process_message(), handle GuiMessage type
//    - Update GUI state based on received messages
//    - Consider using channels to communicate with GUI thread
//
// 3. Send messages to other modules:
//    - Store a channel in SimpleGuiApp to send events back to module
//    - In update(), send messages when UI events occur
//    - Module can forward these to the bus
//
// 4. Add multiple windows:
//    - Each window could be a separate module
//    - Or create sub-panels within this module
//
// 5. Persistence:
//    - Save click_count to file on exit
//    - Load previous state in initialize()
//    - Use serde for serialization
//
// 6. Theme customization:
//    - Use egui::Style to customize appearance
//    - Add UI themes (dark/light mode toggle)
//
// HAPPY CODING! This module is just a starting point - extend it however you need!
// ==============================================================================
