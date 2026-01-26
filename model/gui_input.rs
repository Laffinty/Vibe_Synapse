// ==============================================================================
// GUI Input Module - Window with Text Input and Send Button
// ==============================================================================
// This module demonstrates best practices for GUI development in Vibe_Synapse:
// - Strongly-typed messages (not SystemMessage)
// - Clean module separation
// - Message-driven architecture
// - Thread-safe GUI operations
//
// DEPENDENCIES: windows crate
// ==============================================================================

use async_trait::async_trait;
use std::sync::Arc;
use std::sync::Mutex;
use crate::{MessageEnvelope, MessageBus, Module, SystemMessage};
use std::any::TypeId;

// Context structure passed to window procedure
struct WindowContext {
    bus: Arc<MessageBus>,
    module_name: String,
}

// Windows API imports
use windows::Win32::Foundation::{HWND, LRESULT, WPARAM, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::Graphics::Gdi::GetStockObject;

// ==============================================================================
// STRONGLY-TYPED MESSAGES
// ==============================================================================
// Following audit recommendation 2.1: Use type-safe messages instead of runtime downcasting

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
// Module Implementation
// ==============================================================================

pub struct GuiInputModule {
    name: &'static str,
    bus: Arc<Mutex<Option<Arc<MessageBus>>>>,
    window_handle: Option<HWND>,
}

impl GuiInputModule {
    pub fn new() -> Self {
        Self {
            name: "gui_input",
            bus: Arc::new(Mutex::new(None)),
            window_handle: None,
        }
    }

    fn create_gui(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn std::error::Error>> {
        println!("[GUI Input] Creating input window...");
        
        let class_name = w("VibeInputWindow");
        let hinstance = unsafe { GetModuleHandleW(None)? };
        
        // Create context for window procedure
        let context = Arc::new(WindowContext {
            bus: bus.clone(),
            module_name: self.name.to_string(),
        });
        
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(Self::window_proc_static),
            hInstance: hinstance.into(),
            lpszClassName: windows::core::PCWSTR(class_name.as_ptr()),
            hbrBackground: unsafe { std::mem::transmute(GetStockObject(std::mem::transmute(0u32))) },
            ..Default::default()
        };
        
        unsafe {
            RegisterClassW(&wc);
            
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::PCWSTR(class_name.as_ptr()),
                windows::core::PCWSTR(w("Vibe Synapse - Input Module").as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                500,
                250,
                HWND(0),
                HMENU(0),
                hinstance,
                Some(Arc::into_raw(context) as _), // Pass context
            );
            
            if hwnd.0 == 0 {
                return Err("Failed to create window".into());
            }
            
            // Create text input field
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::PCWSTR(w("EDIT").as_ptr()),
                windows::core::PCWSTR(w("").as_ptr()),
                WS_VISIBLE | WS_CHILD | WS_BORDER | WINDOW_STYLE(ES_LEFT as u32) | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                20,
                30,
                440,
                30,
                hwnd,
                HMENU(2001), // Input field ID
                hinstance,
                None,
            );
            
            // Create send button
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::PCWSTR(w("BUTTON").as_ptr()),
                windows::core::PCWSTR(w("Send to Popup Module").as_ptr()),
                WS_TABSTOP | WS_VISIBLE | WS_CHILD | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
                20,
                80,
                200,
                40,
                hwnd,
                HMENU(2002), // Send button ID
                hinstance,
                None,
            );
            
            // Create clear button
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::PCWSTR(w("BUTTON").as_ptr()),
                windows::core::PCWSTR(w("Clear").as_ptr()),
                WS_TABSTOP | WS_VISIBLE | WS_CHILD | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                240,
                80,
                100,
                40,
                hwnd,
                HMENU(2003), // Clear button ID
                hinstance,
                None,
            );
            
            // Create information label
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::PCWSTR(w("STATIC").as_ptr()),
                windows::core::PCWSTR(w("Enter text and click 'Send' to show in popup window").as_ptr()),
                WS_VISIBLE | WS_CHILD,
                20,
                140,
                440,
                20,
                hwnd,
                HMENU(2004), // Label ID
                hinstance,
                None,
            );
            
            self.window_handle = Some(hwnd);
            ShowWindow(hwnd, SW_SHOW);
            
            println!("[GUI Input] Window created successfully");
        }
        
        Ok(())
    }

    unsafe extern "system" fn window_proc_static(
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
                let context_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowContext;
                
                if !context_ptr.is_null() {
                    let context = &*context_ptr;
                    
                    match button_id {
                        2002 => { // Send button
                            Self::handle_send_click(hwnd, context);
                        }
                        2003 => { // Clear button
                            let input_hwnd = GetDlgItem(hwnd, 2001);
                            SetWindowTextW(input_hwnd, windows::core::PCWSTR(w("").as_ptr()));
                        }
                        _ => {}
                    }
                }
            }
            WM_DESTROY => {
                PostQuitMessage(0);
            }
            _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
        }
        LRESULT(0)
    }

    fn handle_send_click(hwnd: HWND, context: &WindowContext) {
        unsafe {
            let input_hwnd = GetDlgItem(hwnd, 2001);
            let mut buffer = [0u16; 1024];
            
            let text_length = GetWindowTextW(input_hwnd, &mut buffer) as usize;
            if text_length > 0 {
                let text = String::from_utf16_lossy(&buffer[..text_length]);
                println!("[GUI Input] Sending text: {}", text);
                
                // Send text message to popup module
                let message = TextMessage {
                    content: text,
                    sender: context.module_name.clone(),
                };
                
                // Important: Spawn async task to avoid blocking GUI thread
                let bus_clone = context.bus.clone();
                tokio::spawn(async move {
                    if let Err(e) = bus_clone.publish(message, 150).await {
                        eprintln!("[GUI Input] Failed to publish message: {}", e);
                    }
                });
            } else {
                MessageBoxW(
                    hwnd,
                    windows::core::PCWSTR(w("Please enter some text first!").as_ptr()),
                    windows::core::PCWSTR(w("Warning").as_ptr()),
                    MB_OK | MB_ICONWARNING,
                );
            }
        }
    }
}

// ==============================================================================
// Module Lifecycle Implementation
// ==============================================================================

#[async_trait]
impl Module for GuiInputModule {
    fn name(&self) -> &'static str {
        self.name
    }
    
    async fn initialize(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn std::error::Error>> {
        println!("[GUI Input] Initializing...");
        
        *self.bus.lock().unwrap() = Some(bus.clone());
        
        // Register message types
        let text_msg_type = bus.register_message_type::<TextMessage>().await;
        let system_msg_type = bus.register_message_type::<SystemMessage>().await;
        
        // Subscribe to system messages
        bus.subscribe(system_msg_type, self.name.to_string()).await;
        
        // Create GUI with clone of bus
        self.create_gui(bus.clone())?;
        
        // Spawn message loop in separate thread
        let bus_clone = bus.clone();
        std::thread::spawn(move || {
            println!("[GUI Input] Message loop started");
            unsafe {
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, HWND(0), 0, 0).as_bool() {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            println!("[GUI Input] Message loop ended");
        });
        
        println!("[GUI Input] Initialized successfully");
        Ok(())
    }
    
    async fn process_message(&self, envelope: MessageEnvelope) -> Result<(), Box<dyn std::error::Error>> {
        if envelope.message_type == TypeId::of::<SystemMessage>() {
            if let Some(msg) = envelope.payload.as_any().downcast_ref::<SystemMessage>() {
                println!("[GUI Input] System message: {} says {}", msg.source, msg.content);
            }
        }
        Ok(())
    }
    
    async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("[GUI Input] Shutting down...");
        
        if let Some(hwnd) = self.window_handle {
            unsafe {
                PostMessageW(hwnd, WM_DESTROY, WPARAM(0), LPARAM(0));
            }
        }
        
        println!("[GUI Input] Shutdown complete");
        Ok(())
    }
}

// Helper function for wide strings
fn w(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}
