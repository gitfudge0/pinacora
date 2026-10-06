mod accessibility;
mod onboarding;
mod search_input;
mod ui;
use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, SharedString,
    TitlebarOptions, WindowBounds, WindowOptions, actions, point, px, size,
};
actions!(pinacora, [Quit, CheckForUpdates]);
fn main() {
    if let Some(result) = pinacora::updater::run_update_helper_if_requested() {
        if let Err(error) = result {
            eprintln!("Update failed: {error:#}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().any(|arg| arg == "--version") {
        println!("Pinacora {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    for arg in std::env::args().skip(1) {
        let result = match arg.as_str() {
            "--rotation-service" => Some(pinacora::rotation_service::run()),
            "--rotation-service-bootstrap" => {
                Some(pinacora::rotation_service_manager::ensure_running())
            }
            _ => None,
        };
        if let Some(result) = result {
            if let Err(error) = result {
                eprintln!("Rotation service failed: {error:#}");
                std::process::exit(1);
            }
            return;
        }
    }
    if std::env::args().any(|s| s == "--check-source") {
        match check_source() {
            Ok(()) => return,
            Err(e) => {
                eprintln!("Source check failed: {e:#}");
                std::process::exit(1);
            }
        }
    }
    Application::new().run(|cx: &mut App| {
        set_application_icon();
        search_input::bind_keys(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-q"
            } else {
                "ctrl-q"
            },
            Quit,
            None,
        )]);
        cx.set_menus(vec![Menu {
            name: "Pinacora".into(),
            items: vec![
                MenuItem::action("Check for updates…", CheckForUpdates),
                MenuItem::action("Quit Pinacora", Quit),
            ],
        }]);
        let bounds = Bounds::centered(None, size(px(1280.), px(840.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                #[cfg(target_os = "linux")]
                app_id: Some("pinacora".into()),
                window_min_size: Some(size(px(940.), px(620.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("Pinacora")),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(18.), px(20.))),
                }),
                ..Default::default()
            },
            |window, cx| {
                #[cfg(target_os = "linux")]
                window.set_window_title("Pinacora");
                #[cfg(not(target_os = "linux"))]
                let _ = window;
                cx.new(ui::Gallery::new)
            },
        )
        .expect("Could not open Pinacora window");
        cx.activate(true);
    });
}
fn check_source() -> anyhow::Result<()> {
    let items = pinacora::catalogue::fetch_page(1)?;
    println!("Catalogue: {} artworks", items.len());
    let art = &items[0];
    let detail = pinacora::catalogue::fetch_detail(art)?;
    let (path, w, h) = pinacora::download::fetch(&detail.original, true)?;
    println!(
        "Original: {} — {} ({w} × {h}); {}",
        detail.artist,
        detail.title,
        path.display()
    );
    println!("Wallpaper was not changed.");
    Ok(())
}

#[cfg(target_os = "macos")]
fn set_application_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    if let Some(mtm) = MainThreadMarker::new() {
        let data = NSData::with_bytes(include_bytes!("../resources/app-icon.png"));
        if let Some(icon) = NSImage::initWithData(NSImage::alloc(), &data) {
            // The decoded image is valid and retained for the duration of this setter.
            unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&icon)) };
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn set_application_icon() {}
