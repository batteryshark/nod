use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App,
};

use crate::window::focus_main_window;

pub(crate) fn install_tray(app: &mut App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Nod", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(nod_tray_icon()?)
        // The tray art is the Nod mascot glyph as a black-plus-alpha template.
        // macOS only: let AppKit tint it for light/dark menu bars and the
        // selected state. Windows and Linux ignore this and draw the PNG as-is.
        .icon_as_template(true)
        .tooltip("Nod")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, request| match request.id().as_ref() {
            "show" => focus_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, request| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = request
            {
                focus_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn nod_tray_icon() -> tauri::Result<Image<'static>> {
    Image::from_bytes(include_bytes!("../icons/tray-nod.png"))
}
