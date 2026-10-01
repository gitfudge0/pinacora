mod onboarding;
mod ui;
use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, SharedString,
    TitlebarOptions, WindowBounds, WindowOptions, actions, point, px, size,
};
actions!(reframed, [Quit]);
fn main() {
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
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu {
            name: "Reframed".into(),
            items: vec![MenuItem::action("Quit Reframed", Quit)],
        }]);
        let bounds = Bounds::centered(None, size(px(1280.), px(840.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(940.), px(620.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("Reframed")),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(18.), px(20.))),
                }),
                ..Default::default()
            },
            |_, cx| cx.new(ui::Gallery::new),
        )
        .expect("Could not open Reframed window");
        cx.activate(true);
    });
}
fn check_source() -> anyhow::Result<()> {
    let items = reframed::catalogue::fetch_page(1)?;
    println!("Catalogue: {} artworks", items.len());
    let art = &items[0];
    let detail = reframed::catalogue::fetch_detail(art)?;
    let (path, w, h) = reframed::download::fetch(&detail.original, true)?;
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
