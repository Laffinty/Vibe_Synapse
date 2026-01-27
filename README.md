# Vibe Synapse

A minimalist modular framework with automatic module discovery.

## What It Is

A simple framework for building applications from independent modules that communicate via typed messages. Uses `inventory` for compile-time module collection, so there's no configuration file to maintain.

## Quick Start

```bash
cargo run --release
```

This starts the framework and opens a demo GUI window with a button that counts clicks.

## Adding Your Own Module

Create a file at `src/model/your_module/mod.rs`:

```rust
use async_trait::async_trait;
use std::sync::Arc;
use crate::{MessageEnvelope, MessageBus, Module};

pub struct YourModule {
    name: &'static str,
    // your state here
}

impl Default for YourModule {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Module for YourModule {
    fn name(&self) -> &'static str { self.name }
    
    async fn initialize(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn Error>> {
        // setup code
        Ok(())
    }
    
    async fn process_message(&self, envelope: MessageEnvelope) -> Result<(), Box<dyn Error>> {
        // message handling
        Ok(())
    }
    
    async fn shutdown(&mut self) -> Result<(), Box<dyn Error>> {
        // cleanup code
        Ok(())
    }
}

// Add this at the bottom for auto-registration:
crate::module_init!(YourModule, "your_module");
```

That's it. No changes to any other files needed.

## How It Works

1. Each module calls `module_init!()` which registers it with the inventory system
2. At compile time, all modules are collected into a registry
3. Framework creates a message bus and loads all discovered modules
4. Modules communicate by publishing/subscribing to typed messages
5. GUI modules (if present) run on the main thread

## Project Structure

```
src/
├── main.rs                      # Framework core
└── model/
    └── simple_gui/             # Demo GUI module
        └── mod.rs
```

## Requirements

- Rust 1.70 or later
- Works on Windows, macOS, and Linux

## Dependencies

- `tokio` - Async runtime
- `inventory` - Compile-time module collection
- `eframe`/`egui` - GUI (only used by gui modules)
- `async-trait` - Async trait support

## Building

```bash
# Development build
cargo build

# Release build
cargo build --release

# Run tests
cargo test
```

## License

MIT
