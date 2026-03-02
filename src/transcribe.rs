use crate::audio::{self, RecordedAudio};
use crate::constants::{AUDIO_MODEL, MIN_DURATION_SECONDS};
use crate::error::VoxError;
use tokio::sync::mpsc;

#[allow(dead_code)]
pub enum StreamEvent {
    /// Incremental text fragment
    Delta(String),
    /// Final complete text
    Done(String),
    /// Error during streaming
    Error(String),
}

/// Streaming transcription — sends Delta events as text arrives, then Done.
#[allow(dead_code)]
pub async fn transcribe_streaming(
    audio: RecordedAudio,
    api_key: &str,
    api_base: &str,
    organization: Option<&str>,
    project: Option<&str>,
    context: Option<&str>,
    tx: mpsc::Sender<StreamEvent>,
) {
    match transcribe_streaming_inner(audio, api_key, api_base, organization, project, context, &tx)
        .await
    {
        Ok(()) => {}
        Err(e) => {
            let _ = tx.send(StreamEvent::Error(e.to_string())).await;
        }
    }
}

async fn transcribe_streaming_inner(
    audio: RecordedAudio,
    api_key: &str,
    api_base: &str,
    organization: Option<&str>,
    project: Option<&str>,
    context: Option<&str>,
    tx: &mpsc::Sender<StreamEvent>,
) -> Result<(), VoxError> {
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
        .text("stream", "true")
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

    let mut resp = req
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

    // Parse SSE stream using reqwest's chunk() method.
    // SSE format:
    //   event: transcript.text.delta
    //   data: {"type":"transcript.text.delta","delta":"Hello "}
    //
    //   event: transcript.text.done
    //   data: {"type":"transcript.text.done","text":"Hello world"}
    let mut buffer = String::new();
    let mut full_text = String::new();

    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| VoxError::TranscriptionRequest(e.to_string()))?
    {
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Normalize \r\n to \n for SSE parsing
        if buffer.contains('\r') {
            buffer = buffer.replace("\r\n", "\n");
        }

        process_sse_buffer(&mut buffer, &mut full_text, tx).await;
    }

    // Process any remaining data in the buffer (final event may lack trailing \n\n)
    if !buffer.trim().is_empty() {
        buffer.push_str("\n\n");
        process_sse_buffer(&mut buffer, &mut full_text, tx).await;
    }

    if full_text.is_empty() {
        return Err(VoxError::EmptyTranscription);
    }

    let _ = tx.send(StreamEvent::Done(full_text)).await;
    Ok(())
}

async fn process_sse_buffer(
    buffer: &mut String,
    full_text: &mut String,
    tx: &mpsc::Sender<StreamEvent>,
) {
    while let Some(pos) = buffer.find("\n\n") {
        let event_block = buffer[..pos].to_string();
        *buffer = buffer[pos + 2..].to_string();

        for line in event_block.lines() {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data: ") {
                // Skip [DONE] sentinel
                if data.trim() == "[DONE]" {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    match v.get("type").and_then(|t| t.as_str()) {
                        Some("transcript.text.delta") => {
                            if let Some(delta) = v.get("delta").and_then(|d| d.as_str()) {
                                full_text.push_str(delta);
                                let _ = tx.send(StreamEvent::Delta(delta.to_string())).await;
                            }
                        }
                        Some("transcript.text.done") => {
                            if let Some(text) = v.get("text").and_then(|t| t.as_str()) {
                                *full_text = text.to_string();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Non-streaming transcription (for --oneshot mode).
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
