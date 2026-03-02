use thiserror::Error;

#[derive(Debug, Error)]
pub enum VoxError {
    #[error("no input audio device available")]
    NoInputDevice,

    #[error("no API key found. Set OPENAI_API_KEY, CODEX_API_KEY, use --api-key, or run `codex login`")]
    NoApiKey,

    #[error("recording too short ({duration:.2}s); minimum is {min:.2}s")]
    RecordingTooShort { duration: f32, min: f32 },

    #[error("empty transcription result")]
    EmptyTranscription,

    #[error("audio error: {0}")]
    Audio(String),

    #[error("transcription request failed: {0}")]
    TranscriptionRequest(String),

    #[error("transcription failed: {status} {body}")]
    TranscriptionApi { status: String, body: String },

    #[error("terminal error: {0}")]
    Terminal(String),

    #[error("wav encoding error: {0}")]
    WavEncode(String),
}

impl From<std::io::Error> for VoxError {
    fn from(e: std::io::Error) -> Self {
        VoxError::Terminal(e.to_string())
    }
}

#[allow(unused)]
pub type VoxResult<T> = Result<T, VoxError>;
