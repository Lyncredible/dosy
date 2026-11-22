//! Ghostty-style plain-text configuration (`key = value`) for dosy.
//! See `docs/ARCHITECTURE.md` ("Configuration") and task 005.

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// File name of the config file inside the platform config directory.
pub const CONFIG_FILE_NAME: &str = "config";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_file_name_is_plain() {
        assert_eq!(CONFIG_FILE_NAME, "config");
    }
}
