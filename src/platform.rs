use anyhow::Result;
use std::path::Path;
#[cfg(target_os = "macos")]
pub fn apply(path: &Path) -> Result<usize> {
    use objc2::{MainThreadMarker, runtime::AnyObject};
    use objc2_app_kit::{NSScreen, NSWorkspace, NSWorkspaceDesktopImageOptionKey};
    use objc2_foundation::{NSDictionary, NSString, NSURL};
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| anyhow::anyhow!("Wallpaper must be applied on the main thread"))?;
    let path = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid image path"))?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let screens = NSScreen::screens(mtm);
    let workspace = NSWorkspace::sharedWorkspace();
    let options = NSDictionary::<NSWorkspaceDesktopImageOptionKey, AnyObject>::new();
    let count = screens.len();
    if count == 0 {
        anyhow::bail!("No connected displays found");
    }
    for applied in 0..count {
        let screen = screens.objectAtIndex(applied);
        // The URL is a validated cached local image; screen and options remain alive for the call.
        if let Err(error) =
            unsafe { workspace.setDesktopImageURL_forScreen_options_error(&url, &screen, &options) }
        {
            anyhow::bail!(
                "Applied to {applied} of {count} displays: {}",
                error.localizedDescription()
            );
        }
    }
    Ok(count)
}
#[cfg(not(target_os = "macos"))]
pub fn apply(_: &Path) -> Result<usize> {
    anyhow::bail!("Wallpaper support currently requires macOS")
}
