# Vibe Synapse

Version: v0.1

A lightweight Rust framework with plugin architecture.

## Features

- Plugin system
- Event-driven architecture
- Async runtime based on tokio
- Type-safe context and dependency injection

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
vibe-synapse = { path = "path/to/vibe-synapse" }
```

Example:

```rust
use vibe_synapse::prelude::*;

#[derive(Debug)]
struct MyEvent;

impl Event for MyEvent {
    fn as_any(&self) -> &dyn std::any::Any { self }
}

struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("my", "1.0.0")
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = VibeApp::build()
        .add_plugin(MyPlugin)
        .build()
        .await?;
    
    app.run().await?;
    Ok(())
}
```

## License

MIT

---

# Vibe Synapse

版本：v0.1

一个轻量级的 Rust 插件框架。

## 特性

- 插件系统
- 事件驱动架构
- 基于 tokio 的异步运行时
- 类型安全的上下文和依赖注入

## 使用

在 `Cargo.toml` 中添加：

```toml
[dependencies]
vibe-synapse = { path = "path/to/vibe-synapse" }
```

示例：

```rust
use vibe_synapse::prelude::*;

#[derive(Debug)]
struct MyEvent;

impl Event for MyEvent {
    fn as_any(&self) -> &dyn std::any::Any { self }
}

struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("my", "1.0.0")
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = VibeApp::build()
        .add_plugin(MyPlugin)
        .build()
        .await?;
    
    app.run().await?;
    Ok(())
}
```

## 许可证

MIT
