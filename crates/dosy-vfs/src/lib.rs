//! DOS drive abstraction: host directory, single-file and zip mounts with 8.3
//! name mapping. Uses only `std::fs` for host access; no emulator knowledge.
//! See `docs/ARCHITECTURE.md` ("dosy-vfs") for the planned `Drive` trait.

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Drive index (`A` = 0 .. `Z` = 25) for a drive letter, case-insensitive.
pub fn drive_index(letter: char) -> Option<u8> {
    let up = letter.to_ascii_uppercase();
    up.is_ascii_uppercase().then(|| up as u8 - b'A')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_letters_map_to_indices() {
        assert_eq!(drive_index('a'), Some(0));
        assert_eq!(drive_index('C'), Some(2));
        assert_eq!(drive_index('Z'), Some(25));
        assert_eq!(drive_index('1'), None);
    }
}
