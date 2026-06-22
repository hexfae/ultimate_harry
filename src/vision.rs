//! Resolving image attachments for models that lack vision.
//!
//! When the active model cannot read images, a separate vision model is asked to describe each
//! attachment; the descriptions are cached and fed to the LLM as text instead of the image.

use tracing::warn;

use crate::{
    database::Database,
    llm::LlmManager,
    models::message::{AttachmentMode, DescribedAttachment, Message},
};

/// Decides whether the active model receives images or text descriptions, describing and caching
/// any undescribed images when the model lacks vision.
///
/// When vision support cannot be determined, falls back to describing: a text description is
/// accepted by any model, whereas an image hard-fails (404) on a text-only one.
pub async fn resolve_attachments(
    db: &Database,
    requester: &LlmManager,
    context: &mut [Message],
) -> AttachmentMode {
    if !context.iter().any(Message::has_image_attachment) {
        return AttachmentMode::Image;
    }
    match requester.supports_vision().await {
        Ok(true) => return AttachmentMode::Image,
        Ok(false) => {}
        Err(why) => warn!("could not check vision support, describing images to be safe: {why}"),
    }
    describe_and_cache(db, requester, context).await;
    AttachmentMode::Describe
}

/// Describes every undescribed image across `context`, merging the captions into the in-memory
/// messages and caching them onto their stored records for future turns.
async fn describe_and_cache(db: &Database, requester: &LlmManager, context: &mut [Message]) {
    let described = describe_images(requester, context).await;
    if described.is_empty() {
        return;
    }
    for message in context.iter_mut() {
        message.add_descriptions(&described);
    }
    for message in context.iter().filter(|message| message.has_image_attachment()) {
        if let Err(why) = db
            .cache_attachment_descriptions(message.id(), &described)
            .await
        {
            warn!("could not cache image descriptions: {why}");
        }
    }
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
            Err(why) => warn!("could not describe an image, leaving it undescribed: {why}"),
        }
    }
    described
}
