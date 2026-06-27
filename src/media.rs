//! Resolving image and audio attachments for models that lack the matching input modality.
//!
//! When the active model cannot read images, a separate vision model describes each image; when it
//! cannot read audio, a separate audio model transcribes each voice message. The text is cached and
//! fed to the LLM instead. An audio-capable model instead receives the clip natively as base64,
//! since `OpenRouter` rejects audio URLs.

use tracing::warn;

use crate::{
    llm::{LlmError, LlmManager, fetch_audio_base64},
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

/// Collects the distinct URLs that `select` produces across every message in
/// `context`, keeping first-seen order.
fn unique_urls(context: &[Message], select: impl Fn(&Message) -> Vec<String>) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for message in context {
        for url in select(message) {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    urls
}

/// Resolves each unique URL that `select` produces across `context` through the async `describe`
/// call, collecting the successful descriptions and logging `failure` for any that error. Shared by
/// image description and audio transcription.
async fn describe_modality(
    requester: &LlmManager,
    context: &[Message],
    select: impl Fn(&Message) -> Vec<String>,
    describe: impl AsyncFn(&LlmManager, String) -> Result<String, LlmError>,
    failure: &str,
) -> Vec<DescribedAttachment> {
    let urls = unique_urls(context, select);
    let mut described = Vec::new();
    for url in urls {
        match describe(requester, url.clone()).await {
            Ok(description) => described.push(DescribedAttachment { url, description }),
            Err(why) => {
                warn!("{failure}");
                report_error(why);
            }
        }
    }
    described
}

/// Asks the vision model to describe each unique undescribed image URL found across `context`.
async fn describe_images(
    requester: &LlmManager,
    context: &[Message],
) -> Vec<DescribedAttachment> {
    describe_modality(
        requester,
        context,
        Message::undescribed_image_urls,
        async |manager, url| manager.describe_image(&url).await,
        "could not describe an image, leaving it undescribed",
    )
    .await
}

/// Asks the audio model to transcribe each unique untranscribed voice URL found across `context`.
async fn transcribe_audio(
    requester: &LlmManager,
    context: &[Message],
) -> Vec<DescribedAttachment> {
    describe_modality(
        requester,
        context,
        Message::undescribed_audio_urls,
        async |manager, url| manager.transcribe_audio(&url).await,
        "could not transcribe a voice message, leaving it untranscribed",
    )
    .await
}

/// Downloads and base64-encodes each unique voice URL across `context` into the messages, so an
/// audio-capable model can receive the clips natively. The encodings are transient (never persisted)
/// and a failed download leaves that clip unencoded, falling back to a placeholder at render time.
async fn encode_audio(context: &mut [Message]) {
    let urls = unique_urls(context, Message::audio_attachment_urls);
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

/// Tests for the pure URL-collection and description-merging helpers.
#[cfg(test)]
mod tests {
    use super::{apply_descriptions, unique_urls};
    use crate::models::history::History;
    use crate::models::message::{DescribedAttachment, Message, Role};
    use nonempty::NonEmpty;
    use serenity::all::MessageId;

    /// URLs collapse to their distinct set across messages, in first-seen order.
    #[test]
    fn unique_urls_dedups_and_keeps_first_seen_order() {
        let first = Message::new_system("one");
        let second = Message::new_system("two");
        let first_id = first.id().to_owned();
        let context = vec![first, second];

        let urls = unique_urls(&context, |message| {
            if message.id() == first_id {
                vec!["a".to_owned(), "b".to_owned(), "a".to_owned()]
            } else {
                vec!["b".to_owned(), "c".to_owned()]
            }
        });
        assert_eq!(
            urls,
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            "duplicate URLs collapse while first-seen order is kept"
        );
    }

    /// An empty description list leaves the context untouched, while a matching
    /// description marks the owning image as described.
    #[test]
    fn apply_descriptions_is_a_noop_when_empty_and_describes_otherwise() {
        let url = "https://cdn/pic.png".to_owned();
        let message = Message::builder()
            .parts(("Alice".to_owned(), "Alice: hi".to_owned(), Role::User))
            .attachments(vec![url.clone()])
            .build();
        let mut context = vec![message];
        let mut history = History::builder()
            .id(MessageId::new(1))
            .character("character-id")
            .choices(NonEmpty::new(Message::new_system("reply")))
            .current(0_usize)
            .previous(Vec::new())
            .build();

        apply_descriptions(&mut history, &mut context, &[]);
        assert_eq!(
            context.first().map(Message::undescribed_image_urls),
            Some(vec![url.clone()]),
            "an empty description list leaves the image undescribed"
        );

        let described = vec![DescribedAttachment {
            url,
            description: "a picture".to_owned(),
        }];
        apply_descriptions(&mut history, &mut context, &described);
        assert_eq!(
            context.first().map(Message::undescribed_image_urls),
            Some(Vec::new()),
            "a matching description removes the image from the undescribed set"
        );
    }
}
