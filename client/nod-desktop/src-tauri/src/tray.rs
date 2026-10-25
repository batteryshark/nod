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
        // macOS renders the tray as a template image (black plus alpha, tinted
        // by AppKit), so it gets no colour of its own. Windows and Linux draw
        // the PNG unchanged, which is why they load the light twin below.
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
    // Same 64 px mascot glyph, two inks. macOS tints the black master through
    // `icon_as_template`, so it keeps `tray-nod.png`. Everywhere else the PNG is
    // drawn as it is, and a dark taskbar needs light ink, so Windows and Linux
    // load `tray-nod-light.png` — identical geometry and alpha, white ink.
    #[cfg(target_os = "macos")]
    let bytes = include_bytes!("../icons/tray-nod.png");
    #[cfg(not(target_os = "macos"))]
    let bytes = include_bytes!("../icons/tray-nod-light.png");

    Image::from_bytes(bytes)
}
