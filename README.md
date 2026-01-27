# Vibe_Synapse

A minimalist, decoupled modular framework for Rust, designed for automated module discovery and type-safe message passing.

一个极简的、解耦的 Rust 模块化框架，旨在实现自动模块发现和类型安全的消息传递。

## Features / 特性

- **Zero-Configuration Registration**: Modules automatically register at compile time using the inventory crate. No manual registry maintenance in `main.rs`.
  
  零配置注册: 模块通过 inventory crate 在编译期自动注册。无需在 `main.rs` 中手动维护注册表。

- **Decoupled Architecture**: Modules communicate solely via a typed Message Bus (Pub/Sub). No direct dependencies between modules.
  
  解耦架构: 模块间仅通过类型化的消息总线（发布/订阅）通信。模块之间无直接依赖。

- **AI-Friendly**: The architecture isolates module logic, significantly reducing context requirements for AI coding assistants.
  
  AI 友好: 架构隔离了模块逻辑，显著降低了 AI 编程助手所需的上下文长度。

- **Async Core**: Built on tokio for concurrent message processing.
  
  异步核心: 基于 tokio 构建，支持并发消息处理。

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
