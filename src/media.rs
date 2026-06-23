//! Resolving image and audio attachments for models that lack the matching input modality.
//!
//! When the active model cannot read images, a separate vision model describes each image; when it
//! cannot read audio, a separate audio model transcribes each voice message. The text is cached and
//! fed to the LLM instead. An audio-capable model instead receives the clip natively as base64,
//! since `OpenRouter` rejects audio URLs.

use tracing::warn;

use crate::{
    llm::{LlmManager, fetch_audio_base64},
    models::{
        history::History,
        message::{AttachmentMode, DescribedAttachment, MediaMode, Message},
    },
    util::report_error,
};

/// Decides, per modality, whether the active model receives attachments natively or as cached text,
/// describing/transcribing and caching as needed.
pub async fn resolve_attachments(
    requester: &LlmManager,
    history: &mut History,
    context: &mut [Message],
) -> AttachmentMode {
    AttachmentMode {
        image: resolve_images(requester, history, context).await,
        audio: resolve_audio(requester, history, context).await,
    }
}

/// Resolves how images are sent: natively when the model has vision, otherwise described and cached.
///
/// When vision support cannot be determined, falls back to describing: a text description is
/// accepted by any model, whereas an image hard-fails (404) on a text-only one.
async fn resolve_images(
    requester: &LlmManager,
    history: &mut History,
    context: &mut [Message],
) -> MediaMode {
    if !context.iter().any(Message::has_image_attachment) {
        return MediaMode::Native;
    }
    match requester.supports_vision().await {
        Ok(true) => return MediaMode::Native,
        Ok(false) => {}
        Err(why) => {
            warn!("could not check vision support, describing images to be safe");
            report_error(why);
        }
    }
    let described = describe_images(requester, context).await;
    apply_descriptions(history, context, &described);
    MediaMode::Describe
}

/// Resolves how voice messages are sent: natively (base64) when the model accepts audio, otherwise
/// transcribed and cached.
///
/// When audio support cannot be determined, falls back to transcribing, for the same reason images
/// fall back to describing.
async fn resolve_audio(
    requester: &LlmManager,
    history: &mut History,
    context: &mut [Message],
) -> MediaMode {
    if !context.iter().any(Message::has_audio_attachment) {
        return MediaMode::Native;
    }
    match requester.supports_audio().await {
        Ok(true) => {
            encode_audio(context).await;
            return MediaMode::Native;
        }
        Ok(false) => {}
        Err(why) => {
            warn!("could not check audio support, transcribing to be safe");
            report_error(why);
        }
    }
    let transcribed = transcribe_audio(requester, context).await;
    apply_descriptions(history, context, &transcribed);
    MediaMode::Describe
}

/// Merges `descriptions` (image captions or audio transcriptions) into the in-memory `context` (for
/// this request) and into the history's context messages, so they are saved with the turn and
/// reused on future turns.
fn apply_descriptions(
    history: &mut History,
    context: &mut [Message],
    descriptions: &[DescribedAttachment],
) {
    if descriptions.is_empty() {
        return;
    }
    for message in context.iter_mut() {
        message.add_descriptions(descriptions);
    }
    history.apply_descriptions(descriptions);
}

/// Asks the vision model to describe each unique undescribed image URL found across `context`.
async fn describe_images(
    requester: &LlmManager,
    context: &[Message],
) -> Vec<DescribedAttachment> {
    let mut urls: Vec<String> = Vec::new();
    for message in context {
        for url in message.undescribed_image_urls() {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    let mut described = Vec::new();
    for url in urls {
        match requester.describe_image(&url).await {
            Ok(description) => described.push(DescribedAttachment { url, description }),
            Err(why) => {
                warn!("could not describe an image, leaving it undescribed");
                report_error(why);
            }
        }
    }
    described
}

/// Asks the audio model to transcribe each unique untranscribed voice URL found across `context`.
async fn transcribe_audio(
    requester: &LlmManager,
    context: &[Message],
) -> Vec<DescribedAttachment> {
    let mut urls: Vec<String> = Vec::new();
    for message in context {
        for url in message.undescribed_audio_urls() {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    let mut described = Vec::new();
    for url in urls {
        match requester.transcribe_audio(&url).await {
            Ok(transcription) => described.push(DescribedAttachment {
                url,
                description: transcription,
            }),
            Err(why) => {
                warn!("could not transcribe a voice message, leaving it untranscribed");
                report_error(why);
            }
        }
    }
    described
}

/// Downloads and base64-encodes each unique voice URL across `context` into the messages, so an
/// audio-capable model can receive the clips natively. The encodings are transient (never persisted)
/// and a failed download leaves that clip unencoded, falling back to a placeholder at render time.
async fn encode_audio(context: &mut [Message]) {
    let mut urls: Vec<String> = Vec::new();
    for message in context.iter() {
        for url in message.audio_attachment_urls() {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    let mut encoded = Vec::new();
    for url in urls {
        match fetch_audio_base64(&url).await {
            Ok(audio) => encoded.push(audio),
            Err(why) => {
                warn!("could not encode a voice message, leaving it unencoded");
                report_error(why);
            }
        }
    }
    for message in context.iter_mut() {
        message.add_encoded_audio(&encoded);
    }
}
