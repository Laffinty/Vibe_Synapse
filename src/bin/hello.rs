use vibe_synapse::prelude::*;

#[derive(Debug)]
struct HelloEvent;

impl Event for HelloEvent {
    fn as_any(&self) -> &dyn std::any::Any { self }
}

struct HelloPlugin;

#[async_trait]
impl Plugin for HelloPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("hello", "1.0.0")
    }
    
    async fn load(&mut self, ctx: &mut Context) -> Result<()> {
        ctx.subscribe::<_, HelloEvent>(handler("hello", |_| async move {
            println!("Hello, World!");
            Ok(())
        })).await?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = VibeApp::build()
        .add_plugin(HelloPlugin)
        .build()
        .await?;
    
    app.event_bus().start().await?;
    app.plugin_manager().start_all().await?;
    app.context().publish(HelloEvent)?;
    
    Ok(())
}
