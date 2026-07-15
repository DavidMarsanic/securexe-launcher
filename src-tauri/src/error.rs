use std::fmt;

#[derive(Debug)]
pub enum LauncherError {
    InvalidUrl(String),
    Unauthorized(String),
    NotFound(String),
    Network(String),
    ChecksumMismatch,
    Io(String),
}

impl fmt::Display for LauncherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LauncherError::InvalidUrl(msg) => write!(f, "Invalid link: {msg}"),
            LauncherError::Unauthorized(msg) => write!(f, "Unauthorized link: {msg}"),
            LauncherError::NotFound(msg) => write!(f, "Not found: {msg}"),
            LauncherError::Network(msg) => write!(f, "Network error: {msg}"),
            LauncherError::ChecksumMismatch => {
                write!(f, "Checksum mismatch — refusing to run this file")
            }
            LauncherError::Io(msg) => write!(f, "Local error: {msg}"),
        }
    }
}

impl From<std::io::Error> for LauncherError {
    fn from(e: std::io::Error) -> Self {
        LauncherError::Io(e.to_string())
    }
}

impl From<reqwest::Error> for LauncherError {
    fn from(e: reqwest::Error) -> Self {
        LauncherError::Network(e.to_string())
    }
}
