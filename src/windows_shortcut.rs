//! Per-user Start Menu integration for the portable Windows executable.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
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
    let script = r#"
$ErrorActionPreference='Stop'
if (Test-Path -LiteralPath $env:WOO_SHORTCUT) { exit 0 }
$link=(New-Object -ComObject WScript.Shell).CreateShortcut($env:WOO_SHORTCUT)
$link.TargetPath=$env:WOO_EXECUTABLE
$link.WorkingDirectory=[System.IO.Path]::GetDirectoryName($env:WOO_EXECUTABLE)
$link.Description='World of Osso'
$link.IconLocation=$env:WOO_ICON_PATH+',0'
$link.Save()
"#;
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("WOO_SHORTCUT", shortcut)
        .env("WOO_EXECUTABLE", executable)
        .env("WOO_ICON_PATH", icon)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| {
            format!(
                "Create Start Menu shell link {}: {error}",
                shortcut.display()
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "Create Start Menu shell link {}: {}",
        shortcut.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::process::Command;

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
        let link = read_link(&shortcut);
        assert_eq!(
            link["target"].as_str().unwrap(),
            executable.to_str().unwrap()
        );
        assert_eq!(
            link["icon"].as_str().unwrap(),
            format!("{},0", icons.join("world-of-osso.ico").display())
        );
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

    fn read_link(path: &Path) -> Value {
        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                "$link=(New-Object -ComObject WScript.Shell).CreateShortcut($env:WOO_SHORTCUT); @{target=$link.TargetPath;icon=$link.IconLocation}|ConvertTo-Json -Compress",
            ])
            .env("WOO_SHORTCUT", path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
