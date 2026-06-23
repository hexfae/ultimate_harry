//! Resolving image attachments for models that lack vision.
//!
//! When the active model cannot read images, a separate vision model is asked to describe each
//! attachment; the descriptions are cached and fed to the LLM as text instead of the image.

use tracing::warn;

use crate::{
    llm::LlmManager,
    models::{
        history::History,
        message::{AttachmentMode, DescribedAttachment, Message},
    },
    util::report_error,
};

/// Decides whether the active model receives images or text descriptions, describing and caching
/// any undescribed images when the model lacks vision.
///
/// When vision support cannot be determined, falls back to describing: a text description is
/// accepted by any model, whereas an image hard-fails (404) on a text-only one.
pub async fn resolve_attachments(
    requester: &LlmManager,
    history: &mut History,
    context: &mut [Message],
) -> AttachmentMode {
    if !context.iter().any(Message::has_image_attachment) {
        return AttachmentMode::Image;
    }
    match requester.supports_vision().await {
        Ok(true) => return AttachmentMode::Image,
        Ok(false) => {}
        Err(why) => {
            warn!("could not check vision support, describing images to be safe");
            report_error(why);
        }
    }
    let described = describe_images(requester, context).await;
    apply_descriptions(history, context, &described);
    AttachmentMode::Describe
}

/// Merges `described` into the in-memory `context` (for this request) and into the history's context
/// messages, so the descriptions are saved with the turn and reused on future turns.
fn apply_descriptions(
    history: &mut History,
    context: &mut [Message],
    described: &[DescribedAttachment],
) {
    if described.is_empty() {
        return;
    }
    for message in context.iter_mut() {
        message.add_descriptions(described);
    }
    history.apply_descriptions(described);
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
