//! Per-user Start Menu integration for the portable Windows executable.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{Interface, PCWSTR, w};

const ICON: &[u8] = include_bytes!("../assets/world-of-osso.ico");

pub fn install_portable_start_menu_entry() -> Result<(), String> {
    let programs = dirs_next::data_dir()
        .ok_or("Windows roaming app-data directory unavailable")?
        .join("Microsoft/Windows/Start Menu/Programs");
    let icons = dirs_next::data_local_dir()
        .ok_or("Windows local app-data directory unavailable")?
        .join("World of Osso");
    let executable =
        std::env::current_exe().map_err(|error| format!("Locate launcher executable: {error}"))?;
    create_start_menu_shortcut(&programs, &executable, &icons)
}

pub fn create_start_menu_shortcut(
    programs: &Path,
    executable: &Path,
    icon_directory: &Path,
) -> Result<(), String> {
    let shortcut = programs.join("World of Osso.lnk");
    let managed = programs.join("World of Osso/World of Osso.lnk");
    if shortcut.is_file() || managed.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(programs).map_err(|error| {
        format!(
            "Create Start Menu directory {}: {error}",
            programs.display()
        )
    })?;
    std::fs::create_dir_all(icon_directory).map_err(|error| {
        format!(
            "Create launcher icon directory {}: {error}",
            icon_directory.display()
        )
    })?;
    let icon = icon_directory.join("world-of-osso.ico");
    std::fs::write(&icon, ICON)
        .map_err(|error| format!("Write launcher icon {}: {error}", icon.display()))?;
    save_shell_link(&shortcut, executable, &icon)
}

fn save_shell_link(shortcut: &Path, executable: &Path, icon: &Path) -> Result<(), String> {
    // Shell interfaces live only inside this paired COM initialization.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|error| format!("Initialize Start Menu COM: {error}"))?;
    let result = save_com_shell_link(shortcut, executable, icon);
    unsafe { CoUninitialize() };
    result.map_err(|error| format!("Save Start Menu shell link {}: {error}", shortcut.display()))
}

fn save_com_shell_link(
    shortcut: &Path,
    executable: &Path,
    icon: &Path,
) -> windows::core::Result<()> {
    let target = encode_wide_path(executable);
    let icon = encode_wide_path(icon);
    let shortcut = encode_wide_path(shortcut);
    let directory = encode_wide_path(
        executable
            .parent()
            .expect("launcher executable has a parent"),
    );
    // NUL-terminated UTF-16 buffers remain alive throughout these Shell calls.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(target.as_ptr()))?;
        link.SetWorkingDirectory(PCWSTR(directory.as_ptr()))?;
        link.SetDescription(w!("World of Osso"))?;
        link.SetIconLocation(PCWSTR(icon.as_ptr()), 0)?;
        let file: IPersistFile = link.cast()?;
        file.Save(PCWSTR(shortcut.as_ptr()), true)
    }
}

fn encode_wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::STGM_READ;
    use windows::Win32::UI::Shell::SLGP_RAWPATH;

    #[test]
    fn creates_real_shell_link_and_keeps_existing_target() {
        let root = std::env::temp_dir().join(format!(
            "woo-shortcut-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let programs = root.join("User's Programs 日本語");
        let icons = root.join("Launcher");
        let executable = std::env::current_exe().unwrap();
        create_start_menu_shortcut(&programs, &executable, &icons).unwrap();
        let shortcut = programs.join("World of Osso.lnk");
        assert!(
            shortcut.is_file(),
            "First run must create a real Start Menu link"
        );
        let (target, icon, index) = read_link(&shortcut);
        assert_eq!(target, executable.to_str().unwrap());
        assert_eq!(icon, icons.join("world-of-osso.ico").to_str().unwrap());
        assert_eq!(index, 0);
        assert!(icons.join("world-of-osso.ico").is_file());
        let original = std::fs::read(&shortcut).unwrap();
        create_start_menu_shortcut(&programs, &root.join("different.exe"), &icons).unwrap();
        assert_eq!(std::fs::read(&shortcut).unwrap(), original);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn does_not_duplicate_installer_start_menu_entry() {
        let root = std::env::temp_dir().join(format!("woo-managed-link-{}", std::process::id()));
        let managed = root.join("World of Osso/World of Osso.lnk");
        std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
        std::fs::write(&managed, b"installer-owned link").unwrap();
        create_start_menu_shortcut(&root, Path::new("game-launcher.exe"), &root.join("icons"))
            .unwrap();
        assert!(!root.join("World of Osso.lnk").exists());
        assert_eq!(std::fs::read(&managed).unwrap(), b"installer-owned link");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn read_link(path: &Path) -> (String, String, i32) {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .unwrap();
        let result = unsafe {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            let file: IPersistFile = link.cast().unwrap();
            let path = encode_wide_path(path);
            file.Load(PCWSTR(path.as_ptr()), STGM_READ).unwrap();
            let mut target = [0; 1024];
            let mut icon = [0; 1024];
            let mut index = 0;
            link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
                .unwrap();
            link.GetIconLocation(&mut icon, &mut index).unwrap();
            (
                decode_wide_string(&target),
                decode_wide_string(&icon),
                index,
            )
        };
        unsafe { CoUninitialize() };
        result
    }

    fn decode_wide_string(value: &[u16]) -> String {
        let length = value.iter().position(|c| *c == 0).unwrap();
        String::from_utf16(&value[..length]).unwrap()
    }
}
