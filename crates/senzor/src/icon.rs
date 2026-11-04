//! How the window identifies itself to the desktop: its icon and its application ID.

use eframe::egui::IconData;

/// The application ID: Wayland's `app_id`, X11's `WM_CLASS`, and the name of the Linux
/// `.desktop` file (`assets/linux/cz.kankaj.Senzor.desktop`), so the desktop can match the
/// window to its menu entry and icon.
pub const APP_ID: &str = "cz.kankaj.Senzor";

/// macOS shows the icon in the Dock as it is, so it gets the variant drawn on Apple's grid.
#[cfg(target_os = "macos")]
const PNG: &[u8] = include_bytes!("../../../assets/icon/senzor-macos-1024.png");
#[cfg(not(target_os = "macos"))]
const PNG: &[u8] = include_bytes!("../../../assets/icon/senzor-256.png");

/// The icon of the window: the taskbar, Alt+Tab, and the Dock on macOS. Installed packages
/// show the icon files from `assets/icon/` instead.
pub fn app_icon() -> IconData {
    eframe::icon_data::from_png_bytes(PNG).expect("the embedded icon is a valid PNG")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_icon_is_a_square_rgba_image() {
        let icon = app_icon();
        assert_eq!(icon.width, icon.height);
        assert!(icon.width >= 256);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }
}
