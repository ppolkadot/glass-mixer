//! Error types for audio engine command application.

use super::WinResult;

#[derive(Debug)]
pub(super) enum ApplyError {
    Windows(String, windows::core::Error),
    Message(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::Windows(what, e) => {
                let msg = e.message();
                if msg.is_empty() {
                    write!(f, "{what} (error {:#010x}).", e.code().0)
                } else {
                    write!(f, "{what}: {}", msg.trim_end_matches(['.', '\r', '\n']))
                }
            }
            ApplyError::Message(m) => f.write_str(m),
        }
    }
}

pub(super) trait Context<T> {
    fn context(self, what: impl FnOnce() -> String) -> Result<T, ApplyError>;
}

impl<T> Context<T> for WinResult<T> {
    fn context(self, what: impl FnOnce() -> String) -> Result<T, ApplyError> {
        self.map_err(|e| ApplyError::Windows(what(), e))
    }
}
