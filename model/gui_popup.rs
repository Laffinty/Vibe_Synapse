// ==============================================================================
// GUI Popup Module - Display popup windows for received messages
// ==============================================================================
// This module demonstrates:
// - Receiving strongly-typed messages from other modules
// - Dynamic popup window creation
// - Non-blocking message processing
// ==============================================================================

use async_trait::async_trait;
use std::sync::Arc;
use crate::{MessageEnvelope, MessageBus, Module, SystemMessage};
use std::any::TypeId;
use std::collections::VecDeque;
use std::sync::Mutex;

// Windows API imports
use windows::Win32::Foundation::{HWND, LRESULT, WPARAM, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW; 
use windows::Win32::Graphics::Gdi::GetStockObject;

// Windows style constants that might not be in the crate
const SS_CENTER: u32 = 1;
const SS_CENTERIMAGE: u32 = 512;

// Include the TextMessage from gui_input module
// In production, this would be in a shared messages module
#[derive(Clone)]
pub struct TextMessage {
    pub content: String,
    pub sender: String,
}

impl crate::Message for TextMessage {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn message_type(&self) -> TypeId {
        TypeId::of::<TextMessage>()
    }
    
    fn clone_box(&self) -> Box<dyn crate::Message> {
        Box::new(self.clone())
    }
}

// ==============================================================================
// Popup Module Structure
// ==============================================================================

pub struct GuiPopupModule {
    name: &'static str,
    bus: Arc<Mutex<Option<Arc<MessageBus>>>>,
    message_queue: Arc<Mutex<VecDeque<TextMessage>>>,
}

impl GuiPopupModule {
    pub fn new() -> Self {
        Self {
            name: "gui_popup",
            bus: Arc::new(Mutex::new(None)),
            message_queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Creates a simple popup window with the received message
    fn create_popup(&self, message: &TextMessage) -> Result<(), Box<dyn std::error::Error>> {
        println!("[GUI Popup] Creating popup for message: {}", message.content);
        
        let class_name = w("VibePopupWindow");
        let hinstance = unsafe { GetModuleHandleW(None)? };
        
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(Self::popup_window_proc),
            hInstance: hinstance.into(),
            lpszClassName: windows::core::PCWSTR(class_name.as_ptr()),
            hbrBackground: unsafe { std::mem::transmute(GetStockObject(std::mem::transmute(0u32))) },
            ..Default::default()
        };
        
        unsafe {
            RegisterClassW(&wc);
            
            // Create unique window title with timestamp
            let window_title = format!("Message from {} - {}", 
                message.sender, 
                chrono::Local::now().format("%H:%M:%S")
            );
            
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::PCWSTR(class_name.as_ptr()),
                windows::core::PCWSTR(w(&window_title).as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                300,
                200,
                400,
                200,
                HWND(0),
                HMENU(0),
                hinstance,
                Some(message as *const TextMessage as _), // Pass message context
            );
            
            if hwnd.0 == 0 {
                return Err("Failed to create popup window".into());
            }
            
            // Create text label to display the message
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::PCWSTR(w("STATIC").as_ptr()),
                windows::core::PCWSTR(w(format!("Message: {}", message.content).as_str()).as_ptr()),
                WS_VISIBLE | WS_CHILD | WINDOW_STYLE(SS_CENTER) | WINDOW_STYLE(SS_CENTERIMAGE),
                20,
                30,
                340,
                60,
                hwnd,
                HMENU(3001),
                hinstance,
                None,
            );
            
            // Create OK button
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::PCWSTR(w("BUTTON").as_ptr()),
                windows::core::PCWSTR(w("OK").as_ptr()),
                WS_TABSTOP | WS_VISIBLE | WS_CHILD | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
                150,
                110,
                100,
                30,
                hwnd,
                HMENU(3002),
                hinstance,
                None,
            );
            
            ShowWindow(hwnd, SW_SHOW);
            
            println!("[GUI Popup] Popup created successfully");
        }
        
        Ok(())
    }

    unsafe extern "system" fn popup_window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_CREATE => {
                let createstruct = lparam.0 as *const CREATESTRUCTW;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*createstruct).lpCreateParams as _);
            }
            WM_COMMAND => {
                let button_id = wparam.0 & 0xFFFF;
                if button_id == 3002 { // OK button or close
                    PostMessageW(hwnd, WM_DESTROY, WPARAM(0), LPARAM(0));
                }
            }
            WM_DESTROY => {
                // Note: Not calling PostQuitMessage here as it's a child window
            }
            _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
        }
        LRESULT(0)
    }

    /// Process queued messages and show popups
    fn process_queue(&self) {
        let mut queue = self.message_queue.lock().unwrap();
        while let Some(message) = queue.pop_front() {
            // Release lock before creating popup (avoid deadlock)
            drop(queue);
            
            if let Err(e) = self.create_popup(&message) {
                eprintln!("[GUI Popup] Failed to create popup: {}", e);
            }
            
            // Re-acquire lock for next iteration
            queue = self.message_queue.lock().unwrap();
        }
    }
}

// ==============================================================================
// Module Lifecycle Implementation
// ==============================================================================

#[async_trait]
impl Module for GuiPopupModule {
    fn name(&self) -> &'static str {
        self.name
    }
    
    async fn initialize(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn std::error::Error>> {
        println!("[GUI Popup] Initializing...");
        
        *self.bus.lock().unwrap() = Some(bus.clone());
        
        // Register and subscribe to TextMessage
        let text_msg_type = bus.register_message_type::<TextMessage>().await;
        bus.subscribe(text_msg_type, self.name.to_string()).await;
        
        // Also subscribe to system messages for control
        let system_msg_type = bus.register_message_type::<SystemMessage>().await;
        bus.subscribe(system_msg_type, self.name.to_string()).await;
        
        // Spawn GUI processing thread
        let queue_clone = self.message_queue.clone();
        let process_interval = std::time::Duration::from_millis(500);
        
        std::thread::spawn(move || {
            println!("[GUI Popup] Popup processing thread started");
            
            loop {
                // Check for messages and create popups
                {
                    let queue = queue_clone.lock().unwrap();
                    if !queue.is_empty() {
                        // Note: Can't call process_queue here due to self reference
                        // The processing will be handled in process_message
                    }
                }
                
                std::thread::sleep(process_interval);
            }
        });
        
        println!("[GUI Popup] Initialized successfully");
        Ok(())
    }
    
    async fn process_message(&self, envelope: MessageEnvelope) -> Result<(), Box<dyn std::error::Error>> {
        // Handle TextMessage (from gui_input)
        if envelope.message_type == TypeId::of::<TextMessage>() {
            if let Some(msg) = envelope.payload.as_any().downcast_ref::<TextMessage>() {
                println!("[GUI Popup] Received text message from {}: {}", msg.sender, msg.content);
                
                // Queue the message and process it
                self.message_queue.lock().unwrap().push_back(msg.clone());
                self.process_queue();
            }
        }
        // Handle SystemMessage (for control commands)
        else if envelope.message_type == TypeId::of::<SystemMessage>() {
            if let Some(msg) = envelope.payload.as_any().downcast_ref::<SystemMessage>() {
                println!("[GUI Popup] System message: {} from {}", msg.content, msg.source);
                
                match msg.content.as_str() {
                    "popup_test" => {
                        // Test message
                        let test_msg = TextMessage {
                            content: "This is a test popup!".to_string(),
                            sender: "system".to_string(),
                        };
                        self.message_queue.lock().unwrap().push_back(test_msg);
                        self.process_queue();
                    }
                    _ => {}
                }
            }
        }
        
        Ok(())
    }
    
    async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("[GUI Popup] Shutting down...");
        
        // Clear any remaining messages
        self.message_queue.lock().unwrap().clear();
        
        println!("[GUI Popup] Shutdown complete");
        Ok(())
    }
}

// Helper function for wide strings
fn w(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}
