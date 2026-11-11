//! Whether this copy runs from the MSIX package the Microsoft Store installs. A packaged app has
//! an identity of its own: its notifications must name it, and the Store, not GitHub, updates it.

/// The `Id` of the application in the package's manifest, `crates/senzor/msix/AppxManifest.xml`.
pub const APPLICATION_ID: &str = "Senzor";

/// The package family name of the MSIX package this process runs from, such as
/// `43614JaroslavKaka.Senzor_qrj5pzk60eqe4`; `None` when it runs as a plain program (from the MSI,
/// a zip or `cargo run`) or on another system.
pub fn family_name() -> Option<String> {
    #[cfg(windows)]
    return windows::family_name();
    #[cfg(not(windows))]
    None
}

/// The AppUserModelID Windows knows a packaged app by, `<family name>!<application ID>`: a
/// notification that names it shows as Senzor, with its icon.
pub fn app_user_model_id(family_name: &str) -> String {
    format!("{family_name}!{APPLICATION_ID}")
}

/// A name Windows wrote into `buffer`: UTF-16 up to the first NUL.
#[cfg_attr(not(windows), allow(dead_code))]
fn from_wide(buffer: &[u16]) -> Option<String> {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16(&buffer[..end])
        .ok()
        .filter(|name| !name.is_empty())
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFamilyName;

    /// Asks Windows for the package of this process. The first call only learns the length; a
    /// process without a package gets `APPMODEL_ERROR_NO_PACKAGE` instead.
    pub fn family_name() -> Option<String> {
        let mut length = 0u32;
        // SAFETY: a zero length with a null buffer asks only for the length.
        let first = unsafe { GetCurrentPackageFamilyName(&mut length, std::ptr::null_mut()) };
        if first != ERROR_INSUFFICIENT_BUFFER || length == 0 {
            return None;
        }
        let mut buffer = vec![0u16; length as usize];
        // SAFETY: the buffer holds `length` UTF-16 units, as the length says.
        let second = unsafe { GetCurrentPackageFamilyName(&mut length, buffer.as_mut_ptr()) };
        if second != 0 {
            return None;
        }
        super::from_wide(&buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_user_model_id_names_the_package_and_the_application() {
        assert_eq!(
            app_user_model_id("43614JaroslavKaka.Senzor_qrj5pzk60eqe4"),
            "43614JaroslavKaka.Senzor_qrj5pzk60eqe4!Senzor"
        );
    }

    #[test]
    fn the_application_id_is_the_one_in_the_manifest() {
        let manifest = include_str!("../msix/AppxManifest.xml");
        assert!(
            manifest.contains(&format!("<Application Id=\"{APPLICATION_ID}\"")),
            "AppxManifest.xml declares the application as {APPLICATION_ID}"
        );
    }

    #[test]
    fn a_name_from_windows_ends_at_its_nul() {
        let wide: Vec<u16> = "Senzor_abc\0\0".encode_utf16().collect();
        assert_eq!(from_wide(&wide).as_deref(), Some("Senzor_abc"));
        assert_eq!(from_wide(&[0, 0]), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_there_is_no_package() {
        assert_eq!(family_name(), None);
    }
}
