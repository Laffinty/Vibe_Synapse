# Vibe_Synapse

A minimalist modular framework in Rust for AI development, using a message-passing mechanism to achieve complete decoupling between modules.

一个用于 AI 开发的极简 Rust 模块化框架，采用消息传递机制实现模块间的完全解耦。

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



## License / 许可

本框架采用 **MIT** 和 **Apache-2.0** 双许可。

This framework is dual-licensed under **MIT** and **Apache-2.0**.

您可以选择以下任一许可：
- [MIT License](./LICENSE-MIT) - 简洁宽松，允许闭源商用
- [Apache License 2.0](./LICENSE-APACHE) - 提供专利保护

您可以自由选择适用于您的场景。
