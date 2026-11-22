//! DOS error codes.

use std::fmt;

/// Errors returned by drives and files; [`code`](Self::code) is the DOS
/// error number (INT 21h AX on CF=1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DosError {
    InvalidFunction,
    FileNotFound,
    PathNotFound,
    TooManyOpenFiles,
    AccessDenied,
    InvalidHandle,
    InsufficientMemory,
    InvalidAccess,
    InvalidDrive,
    CurrentDirectory,
    NotSameDevice,
    NoMoreFiles,
    WriteProtected,
    Exists,
}

impl DosError {
    pub fn code(self) -> u16 {
        match self {
            Self::InvalidFunction => 0x01,
            Self::FileNotFound => 0x02,
            Self::PathNotFound => 0x03,
            Self::TooManyOpenFiles => 0x04,
            Self::AccessDenied => 0x05,
            Self::InvalidHandle => 0x06,
            Self::InsufficientMemory => 0x08,
            Self::InvalidAccess => 0x0C,
            Self::InvalidDrive => 0x0F,
            Self::CurrentDirectory => 0x10,
            Self::NotSameDevice => 0x11,
            Self::NoMoreFiles => 0x12,
            Self::WriteProtected => 0x13,
            Self::Exists => 0x50,
        }
    }
}

impl fmt::Display for DosError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?} (DOS error {:#04x})", self.code())
    }
}

impl std::error::Error for DosError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_dos() {
        assert_eq!(DosError::FileNotFound.code(), 2);
        assert_eq!(DosError::PathNotFound.code(), 3);
        assert_eq!(DosError::TooManyOpenFiles.code(), 4);
        assert_eq!(DosError::AccessDenied.code(), 5);
        assert_eq!(DosError::NoMoreFiles.code(), 0x12);
        assert_eq!(DosError::Exists.code(), 0x50);
    }
}
