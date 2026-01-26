// ==============================================================================
// Vibe_Synapse 框架 - 模块配置文件
// ==============================================================================
// 本文件是框架的唯一配置文件，用户只需修改此文件即可管理所有模块
// 注释使用中文，方便用户理解和配置
// ==============================================================================

// ==============================================================================
// 第1步：模块声明 - 在此处声明所有模块
// 每个模块对应 model/ 目录下的一个 .rs 文件
// ==============================================================================

// 示例：声明 GUI 演示模块
// 格式：#[path = "../model/模块文件名.rs"] mod 模块名;
#[path = "../model/gui_input.rs"]
mod gui_input;

#[path = "../model/gui_popup.rs"]
mod gui_popup;

// ==============================================================================
// 第2步：预加载模块列表 - 在此处指定要加载的模块
// 只需将模块名称添加到 PRELOAD_MODULES 数组中
// 框架启动时会自动加载所有指定的模块
// 注意：模块名称必须唯一，且与上面的声明一致
// ==============================================================================

pub const PRELOAD_MODULES: &[&str] = &[
    "gui_input",   // GUI输入模块 - 包含文本输入框和发送按钮，向popup模块发送消息
    "gui_popup",   // GUI弹窗模块 - 接收并显示来自input模块的消息
    // 添加新模块时，只需在此数组中添加模块名称字符串
    // 例如: "logger", "monitor"
];

// ==============================================================================
// 第3步：模块加载逻辑 - 实现模块的实例化和加载
// 当框架加载模块时，会调用此函数创建模块实例
// 添加新模块时，在此处添加对应的匹配分支
// ==============================================================================

pub async fn load_selected_modules(
    registry: &crate::ModuleRegistry,
    selected_modules: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    
    println!("\n========== 模块加载阶段 ==========");
    
    // 遍历需要加载的所有模块
    for module_name in selected_modules {
        println!("正在加载模块: {}", module_name);
        
        // 根据模块名称创建对应的实例
        // 每个分支对应一个模块的加载逻辑
        match module_name.as_str() {
            "gui_input" => {
                // 创建GUI输入模块实例并加载
                let module = Box::new(gui_input::GuiInputModule::new());
                registry.load_module("gui_input".to_string(), module).await?;
                println!("✓ GUI输入模块加载成功");
            }
            "gui_popup" => {
                // 创建GUI弹窗模块实例并加载
                let module = Box::new(gui_popup::GuiPopupModule::new());
                registry.load_module("gui_popup".to_string(), module).await?;
                println!("✓ GUI弹窗模块加载成功");
            }
            /*
            // 添加新模块的模板（复制并修改）：
            "新模块名称" => {
                let module = Box::new(模块名::结构体名::new());
                registry.load_module("模块显示名称".to_string(), module).await?;
                println!("✓ 新模块加载成功");
            }
            */
            unknown => {
                // 如果模块未定义，显示错误信息
                eprintln!("✗ 错误：未知模块 '{}'，请检查模块声明和加载逻辑", unknown);
                return Err(format!("未知模块: {}", unknown).into());
            }
        }
    }
    
    println!("========== 模块加载完成 ==========\n");
    Ok(())
}
