use crate::audio::{self, RecordedAudio};
use crate::constants::{AUDIO_MODEL, MIN_DURATION_SECONDS};
use crate::error::VoxError;

pub async fn transcribe(
    audio: RecordedAudio,
    api_key: &str,
    api_base: &str,
    organization: Option<&str>,
    project: Option<&str>,
    context: Option<&str>,
) -> Result<String, VoxError> {
    let duration_seconds = audio::clip_duration_seconds(&audio);
    if duration_seconds < MIN_DURATION_SECONDS {
        return Err(VoxError::RecordingTooShort {
            duration: duration_seconds,
            min: MIN_DURATION_SECONDS,
        });
    }

    let wav_bytes = audio::encode_wav_normalized(&audio)?;

    let client = reqwest::Client::new();
    let part = reqwest::multipart::Part::bytes(wav_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| VoxError::TranscriptionRequest(format!("failed to set mime: {e}")))?;

    let mut form = reqwest::multipart::Form::new()
        .text("model", AUDIO_MODEL)
        .part("file", part);

    if let Some(ctx) = context {
        form = form.text("prompt", ctx.to_string());
    }

    let endpoint = format!("{api_base}/audio/transcriptions");

    let mut req = client
        .post(&endpoint)
        .bearer_auth(api_key)
        .header("User-Agent", "vox/0.1");

    if let Some(org) = organization {
        req = req.header("OpenAI-Organization", org);
    }
    if let Some(proj) = project {
        req = req.header("OpenAI-Project", proj);
    }

    let resp = req
        .multipart(form)
        .send()
        .await
        .map_err(|e| VoxError::TranscriptionRequest(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".to_string());
        return Err(VoxError::TranscriptionApi {
            status: status.to_string(),
            body,
        });
    }

    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| VoxError::TranscriptionRequest(format!("failed to parse json: {e}")))?;

    let text = v
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    if text.is_empty() {
        Err(VoxError::EmptyTranscription)
    } else {
        Ok(text)
    }
}
