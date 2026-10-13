//! Real executable lifetime boundary: preflight succeeds, normal startup fails.
#![cfg(not(feature = "desktop"))]

use game_launcher::self_update::replace_and_restart;
use std::path::PathBuf;

#[test]
fn failed_relaunch_rolls_back_after_successful_self_check() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let directory =
            std::env::temp_dir().join(format!("launcher-rollback-process-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let name = if cfg!(windows) {
            "launcher.exe"
        } else {
            "launcher"
        };
        let executable = directory.join(name);
        let staged = directory.join(format!("{name}.new"));
        std::fs::write(&executable, b"original launcher").unwrap();
        std::fs::copy(PathBuf::from(env!("CARGO_BIN_EXE_game-launcher")), &staged).unwrap();
        let result = replace_and_restart(
            &executable,
            &staged,
            env!("CARGO_PKG_VERSION"),
            &["no-desktop-startup".into()],
        )
        .await;
        assert!(result.unwrap_err().contains("before startup handshake"));
        assert_eq!(std::fs::read(&executable).unwrap(), b"original launcher");
        assert!(!directory.join(format!("{name}.old")).exists());
        std::fs::remove_dir_all(directory).unwrap();
    });
}
