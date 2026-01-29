# Vibe_Synapse

A minimalist, decoupled modular framework for Rust with an AI-friendly architecture, designed for automated module discovery and type-safe message passing.

一个极简的、解耦的 Rust 模块化框架，采用 AI 友好的架构设计，实现自动模块发现和类型安全的消息传递。

## Usage / 使用方法

Create a module in `src/model/your_module/mod.rs`.

在 `src/model/your_module/mod.rs` 中创建模块。

Implement the `Module` trait and add the registration macro at the bottom.

实现 `Module` trait 并在文件底部添加注册宏。

```rust
use crate::{module_init, Module, MessageBus};

#[derive(Default)]
struct MyModule;

#[async_trait]
impl Module for MyModule {
    fn name(&self) -> &'static str { "my_module" }

    async fn initialize(&mut self, bus: Arc<MessageBus>) -> Result<(), Box<dyn Error>> {
        // Subscribe to messages here
        Ok(())
    }
    
    // ... implement process_message and shutdown
}

// Auto-registration / 自动注册
module_init!(MyModule, "my_module");
```

That's it. The framework will handle the rest.

仅此而已。框架将处理其余工作。

## License / 许可

本框架采用 **MIT** 和 **Apache-2.0** 双许可。

This framework is dual-licensed under **MIT** and **Apache-2.0**.

您可以选择以下任一许可：
- [MIT License](./LICENSE-MIT) - 简洁宽松，允许闭源商用
- [Apache License 2.0](./LICENSE-APACHE) - 提供专利保护

您可以自由选择适用于您的场景。
