use crate::audio::{self, RecordedAudio};

const AUDIO_MODEL: &str = "gpt-4o-mini-transcribe";
const MIN_DURATION_SECONDS: f32 = 1.0;

pub async fn transcribe(
    audio: RecordedAudio,
    api_key: &str,
    api_base: &str,
    organization: Option<&str>,
    project: Option<&str>,
    context: Option<&str>,
) -> Result<String, String> {
    let duration_seconds = audio::clip_duration_seconds(&audio);
    if duration_seconds < MIN_DURATION_SECONDS {
        return Err(format!(
            "recording too short ({duration_seconds:.2}s); minimum is {MIN_DURATION_SECONDS:.2}s"
        ));
    }

    let wav_bytes = audio::encode_wav_normalized(&audio)?;
    let audio_kib = wav_bytes.len() as f32 / 1024.0;

    eprintln!(
        "  sending {audio_kib:.1} KiB ({duration_seconds:.1}s) to {AUDIO_MODEL}..."
    );

    let client = reqwest::Client::new();
    let part = reqwest::multipart::Part::bytes(wav_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("failed to set mime: {e}"))?;

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
        .map_err(|e| format!("transcription request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".to_string());
        return Err(format!("transcription failed: {status} {body}"));
    }

    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("failed to parse json: {e}"))?;

    let text = v
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    if text.is_empty() {
        Err("empty transcription result".to_string())
    } else {
        Ok(text)
    }
}
