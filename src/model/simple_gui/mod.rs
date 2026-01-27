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
// ==============================================================================

use async_trait::async_trait;
use std::sync::Arc;
use crate::{MessageEnvelope, MessageBus, Module};
use std::any::TypeId;
use tokio::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

// ============================================================================
// Module State Structure
// ============================================================================
// All fields are Arc-wrapped for thread-safe sharing between async tasks
// and the GUI thread.
//
// Fields:
// - name: Unique module identifier (matches module_init! macro)
// - bus: Optional Arc<MessageBus> for framework communication
// - is_running: Atomic flag to control GUI lifecycle
// - click_count: Shared counter for button clicks
#[derive(Clone)]
pub struct SimpleGuiModule {
    name: &'static str,
    bus: Arc<RwLock<Option<Arc<MessageBus>>>>,
    is_running: Arc<AtomicBool>,
    click_count: Arc<RwLock<usize>>,
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
    ///
    /// Sets up initial state but does NOT start GUI (see run_gui_blocking())
    pub fn new() -> Self {
        Self {
            name: "simple_gui",
            bus: Arc::new(RwLock::new(None)),
            is_running: Arc::new(AtomicBool::new(false)),
            click_count: Arc::new(RwLock::new(0)),
        }
    }


}

/// Actual egui application that renders the UI
/// 
/// Implements eframe::App trait which requires update() method.
/// update() is called every frame to render the UI and handle events.
struct SimpleGuiApp {
    is_running: Arc<AtomicBool>,
    click_count: Arc<RwLock<usize>>,
    label_text: String,
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Don't render if shutting down
        if !self.is_running.load(Ordering::SeqCst) {
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
                    let mut count = self.click_count.blocking_write();
                    *count += 1;
                    self.label_text = format!("You have clicked {} times!", *count);
                }
                
                ui.add_space(20.0);
                
                // Display counter label
                ui.label(&self.label_text);
                
                ui.add_space(40.0);
                
                // Exit button
                if ui.button("Exit").clicked() {
                    self.is_running.store(false, Ordering::SeqCst);
                }
            });
        });
        
        // Request repaint for responsive UI (eframe does rate limiting internally)
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
    fn name(&self) -> &'static str {
        self.name
    }
    
    /// Initializes module with bus access
    /// 
    /// CALLED BY:
    /// - ModuleRegistry::register_all_modules() after module construction
    ///
    /// RESPONSIBILITIES:
    /// - Store Arc<MessageBus> for future message operations
    /// - Register message types (optional, for this demo module)
    /// - Subscribe to message types (optional)
    /// - Perform lightweight setup (no blocking operations)
    ///
    /// FOR GUI MODULES:
    /// - Store bus reference
    /// - GUI is started via spawn_blocking in initialize() - module controls its own lifecycle
    ///
    /// ERROR HANDLING:
    /// - Return Err to prevent module from loading
    /// - This module will be skipped but others will still load
    async fn initialize(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        println!("[Simple GUI] Initializing...");
        
        // Store bus reference for potential future message operations
        *self.bus.write().await = Some(bus.clone());
        
        // Start GUI on main thread using spawn_blocking
        // This allows the module to control its own blocking main loop
        let is_running = self.is_running.clone();
        let click_count = self.click_count.clone();
        
        println!("[Simple GUI] Starting GUI on main thread...");
        self.is_running.store(true, Ordering::SeqCst);
        
        // Clone bus for signaling exit when GUI closes
        let bus_for_exit = bus.clone();
        
        tokio::task::spawn_blocking(move || {
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
            #[cfg(windows)]
            {
                use winit::platform::windows::EventLoopBuilderExtWindows;
                native_options.event_loop_builder = Some(Box::new(|builder| {
                    builder.with_any_thread(true);
                }));
            }
            
            // Clone for the closure to avoid move issues
            let is_running_for_app = is_running.clone();
            
            // Use run_native with event_loop_builder to allow any thread
            let result = eframe::run_native(
                "Vibe Synapse Simple GUI",
                native_options,
                Box::new(move |_cc| {
                    Ok(Box::new(SimpleGuiApp {
                        is_running: is_running_for_app,
                        click_count,
                        label_text: "Click the button to start counting".to_string(),
                    }))
                }),
            ).map_err(|e| format!("eframe error: {:?}", e));
            
            match result {
                Ok(()) => println!("[Simple GUI] GUI window closed"),
                Err(e) => eprintln!("[Simple GUI] GUI error: {}", e),
            }
            
            // Signal that GUI has closed
            is_running.store(false, Ordering::SeqCst);
            
            // Signal main thread to exit (for Windows GUI mode)
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                bus_for_exit.signal_exit().await;
            });
        });
        
        // Give GUI time to start
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        
        println!("[Simple GUI] Initialization complete - GUI started on main thread");
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
    /// - Could be extended to handle control messages
    async fn process_message(&self, envelope: MessageEnvelope) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Handle SystemMessage for potential control messages
        if envelope.message_type == TypeId::of::<crate::SystemMessage>() {
            if let Some(msg) = envelope.payload.as_any().downcast_ref::<crate::SystemMessage>() {
                println!("[Simple GUI] Received system message: {} - {}", msg.source, msg.content);
            }
        }
        
        // Could add handling for other message types here
        Ok(())
    }
    
    /// Graceful shutdown of module
    /// 
    /// CALLED BY:
    /// - ModuleRegistry::unregister_module() during framework shutdown
    ///
    /// RESPONSIBILITIES:
    /// - Set is_running=false to signal GUI thread to exit
    /// - Wait for GUI to fully close (poll with timeout)
    /// - Clean up any resources
    /// - Return Ok(()) even if cleanup has issues
    async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        println!("[Simple GUI] Shutting down...");
        
        // Signal GUI to close
        self.is_running.store(false, Ordering::SeqCst);
        
        // Wait for GUI to fully close (with timeout to avoid hanging)
        let mut attempts = 0;
        while self.is_running.load(Ordering::SeqCst) && attempts < 20 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            attempts += 1;
        }
        
        println!("[Simple GUI] Shutdown complete");
        Ok(())
    }
}

// ==============================================================================
// Inventory Auto-Registration
// ==============================================================================
// This single line automatically registers the module with the framework
// at compile time. The inventory system collects all module_init! calls
// and makes them discoverable via inventory::iter::<ModuleBuildInfo>().
// No changes to main.rs or any config files needed.
crate::module_init!(SimpleGuiModule, "simple_gui");

// ==============================================================================
// Example Message Type (for extension)
// ==============================================================================
// If you want to add messages that other modules can send to this GUI,
// define them here and handle them in process_message().
//
/// Demo message that could be sent to update GUI text
#[allow(dead_code)]
#[derive(Clone)]
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
//
// 3. Send messages to other modules:
//    - In update(), get bus reference from self.bus
//    - Create and publish messages when UI events occur
//    - Example: bus.publish(UserInputEvent { ... }).await?;
//
// 4. Add multiple windows:
//    - Each window could be a separate module
//    - Or create sub-panels within this module
//
// 5. Persistence:
//    - Save click_count to file on exit
//    - Load previous state in initialize()
//
// 6. Theme customization:
//    - Use egui::Style to customize appearance
//    - Add UI themes (dark/light mode toggle)
//
// HAPPY CODING! This module is just a starting point - extend it however you need!
// ==============================================================================
