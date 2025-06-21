use std::path::PathBuf;
use std::env;

/// Get XDG config directory for mcproc
/// Falls back to ~/.config/mcproc if XDG_CONFIG_HOME is not set
pub fn get_config_dir() -> PathBuf {
    if let Ok(xdg_config) = env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg_config).join("mcproc")
    } else if let Some(home) = dirs::home_dir() {
        home.join(".config").join("mcproc")
    } else {
        PathBuf::from(".config/mcproc")
    }
}

/// Get XDG data directory for mcproc
/// Falls back to ~/.local/share/mcproc if XDG_DATA_HOME is not set
pub fn get_data_dir() -> PathBuf {
    if let Ok(xdg_data) = env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg_data).join("mcproc")
    } else if let Some(home) = dirs::home_dir() {
        home.join(".local").join("share").join("mcproc")
    } else {
        PathBuf::from(".local/share/mcproc")
    }
}

/// Get XDG state directory for mcproc
/// Falls back to ~/.local/state/mcproc if XDG_STATE_HOME is not set
pub fn get_state_dir() -> PathBuf {
    if let Ok(xdg_state) = env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg_state).join("mcproc")
    } else if let Some(home) = dirs::home_dir() {
        home.join(".local").join("state").join("mcproc")
    } else {
        PathBuf::from(".local/state/mcproc")
    }
}

/// Get XDG runtime directory for mcproc
/// Falls back to /tmp/mcproc-$UID or state dir if XDG_RUNTIME_DIR is not set
pub fn get_runtime_dir() -> PathBuf {
    if let Ok(xdg_runtime) = env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(xdg_runtime).join("mcproc")
    } else {
        // Fall back to /tmp/mcproc-$UID
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/tmp/mcproc-{}", uid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_config_dir_with_xdg() {
        env::set_var("XDG_CONFIG_HOME", "/custom/config");
        assert_eq!(get_config_dir(), PathBuf::from("/custom/config/mcproc"));
        env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    #[serial]
    fn test_data_dir_with_xdg() {
        env::set_var("XDG_DATA_HOME", "/custom/data");
        assert_eq!(get_data_dir(), PathBuf::from("/custom/data/mcproc"));
        env::remove_var("XDG_DATA_HOME");
    }

    #[test]
    #[serial]
    fn test_state_dir_with_xdg() {
        env::set_var("XDG_STATE_HOME", "/custom/state");
        assert_eq!(get_state_dir(), PathBuf::from("/custom/state/mcproc"));
        env::remove_var("XDG_STATE_HOME");
    }

    #[test]
    #[serial]
    fn test_runtime_dir_with_xdg() {
        env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        assert_eq!(get_runtime_dir(), PathBuf::from("/run/user/1000/mcproc"));
        env::remove_var("XDG_RUNTIME_DIR");
    }

    #[test]
    #[serial]
    fn test_runtime_dir_fallback() {
        env::remove_var("XDG_RUNTIME_DIR");
        let uid = unsafe { libc::getuid() };
        assert_eq!(get_runtime_dir(), PathBuf::from(format!("/tmp/mcproc-{}", uid)));
    }
}