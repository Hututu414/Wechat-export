#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod ui;
fn main() -> eframe::Result {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/app.png"))
        .map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([500.0, 790.0])
            .with_min_inner_size([420.0, 640.0])
            .with_title("微信聊天导出")
            .with_icon(icon),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "wechat-export",
        options,
        Box::new(|cc| Ok(Box::new(ui::App::new(cc)))),
    )
}
