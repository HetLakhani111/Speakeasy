use std::path::PathBuf;

/// Every fallible runtime path in Speakeasy funnels through this type. It is
/// serializable so it can be handed to the frontend as a command error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not access {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not parse settings: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid setting: {0}")]
    Invalid(String),

    #[error("could not resolve a system path: {0}")]
    Path(String),

    #[error("tray unavailable: {0}")]
    Tray(String),

    #[error("window `{0}` is not available")]
    WindowMissing(String),

    #[error("launch at login unavailable: {0}")]
    Autostart(String),

    #[error("audio: {0}")]
    Audio(String),

    #[error("hotkey: {0}")]
    Hotkey(String),

    #[error("model: {0}")]
    Model(String),

    #[error("transcription failed: {0}")]
    Transcription(String),

    #[error("could not paste the transcript: {0}")]
    Paste(String),

    #[error("{0}")]
    Tauri(#[from] tauri::Error),
}

impl serde::Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> Error {
    Error::Io {
        path: path.into(),
        source,
    }
}
