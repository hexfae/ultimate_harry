//! Attachment and media handling for [`Message`]: how a message's images and
//! voice notes are sent to the LLM (natively or as cached text), the cache
//! bookkeeping for descriptions and encodings, and the rig-message conversion.
//!
//! Split out from the model in `message.rs`; as a child module this can still
//! reach `Message`'s private fields and helper methods.

use rig::{
    OneOrMany,
    agent::Text,
    message::{
        AssistantContent, Audio, AudioMediaType, DocumentSourceKind, Message as RigMessage,
        UserContent,
    },
};
use serde::{Deserialize, Serialize};

use super::{Message, Role};

/// Placeholder text fed to a non-vision model when an image has no cached description.
const UNDESCRIBED_IMAGE: &str = "[Bild kunde inte tolkas]";

/// Placeholder text fed to a model when a voice message could not be transcribed or encoded.
const UNDESCRIBED_AUDIO: &str = "[Ljud kunde inte tolkas]";

/// Whether one kind of attachment is sent to the model natively or as cached text.
///
/// `Native` is the normal path (an image URL or base64 audio); `Describe` is used when the active
/// model lacks the matching input modality and a separate model has described or transcribed the
/// attachment instead.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MediaMode {
    /// Send the attachment to the model in its native form.
    #[default]
    Native,
    /// Replace the attachment with its cached text (an image description or audio transcription).
    Describe,
}

/// How a message's attachments are sent to the LLM, decided independently per modality.
///
/// Images and audio are separate: a model may accept images but not audio (most vision models do),
/// so each modality carries its own [`MediaMode`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentMode {
    /// How image attachments are sent.
    pub image: MediaMode,
    /// How audio attachments (Discord voice messages) are sent.
    pub audio: MediaMode,
}

/// A cached vision-model description of a single attachment, looked up by its URL.
///
/// Used both for image descriptions and for voice-message transcriptions; the stored `description`
/// is the raw text, and the renderer wraps it differently per attachment kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DescribedAttachment {
    /// The attachment URL this describes.
    pub url: String,
    /// The model's text description (an image caption or an audio transcription).
    pub description: String,
}

/// A base64-encoded voice attachment, looked up by its URL.
///
/// Filled transiently when the active model accepts audio, so the clip can be sent natively;
/// never persisted (the bytes live only for the duration of one request).
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedAudio {
    /// The attachment URL this encodes.
    pub url: String,
    /// The base64-encoded audio bytes.
    pub data: String,
    /// The audio media type, as detected from the URL.
    pub format: AudioMediaType,
}

/// How one attachment is rendered into LLM content under a given [`AttachmentMode`].
#[derive(Debug, PartialEq)]
enum AttachmentRender {
    /// Sent as an image URL.
    Image(String),
    /// Sent as base64-encoded audio (Discord voice messages on an audio-capable model).
    Audio {
        /// The base64-encoded audio bytes.
        data: String,
        /// The audio media type.
        format: AudioMediaType,
    },
    /// Sent as plain text (a description, a transcription, or a placeholder).
    Text(String),
}

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the attachment and rig-conversion methods are split into this child module to separate the Message model from media handling"
)]
impl Message {
    /// Converts this message into a vector of Rig messages for the LLM.
    ///
    /// `mode` selects whether image attachments are sent as images or replaced with their cached
    /// text descriptions (for models without vision).
    #[must_use]
    pub fn to_rig_messages(&self, mode: AttachmentMode) -> Vec<RigMessage> {
        let chosen = self.chosen_revision();
        let last_user = chosen
            .0
            .iter()
            .rposition(|part| matches!(part.role, Role::User));

        let mut messages: Vec<RigMessage> = chosen
            .0
            .iter()
            .enumerate()
            .map(|(index, part)| match part.role {
                Role::System => RigMessage::System {
                    content: part.content.clone(),
                },
                Role::Assistant => RigMessage::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::Text(Text::new(
                        part.content.clone(),
                    ))),
                },
                Role::User => {
                    let renders = if Some(index) == last_user {
                        self.render_attachments(mode)
                    } else {
                        Vec::new()
                    };
                    Self::user_message_with_renders(&part.content, renders)
                }
            })
            .collect();

        // Attachments live only on user-authored messages, but a user can prefix every line with
        // `ai:`/`system:`, leaving no user part to carry them. Append the renders as a trailing
        // user message so the image (or its description) still reaches the model instead of
        // vanishing silently.
        if last_user.is_none() {
            let renders = self.render_attachments(mode);
            if !renders.is_empty() {
                messages.push(Self::user_message_with_renders("", renders));
            }
        }

        messages
    }

    /// Builds a user message from `base_text` plus rendered attachments.
    ///
    /// Folds text renders (descriptions and placeholders) into the message's single text content;
    /// only images/audio become separate content parts. A text-only model rejects a multi-part
    /// content array, so a described image must arrive as a plain string, exactly like a normal
    /// text message.
    fn user_message_with_renders(base_text: &str, renders: Vec<AttachmentRender>) -> RigMessage {
        let mut text = base_text.to_owned();
        let mut media = Vec::new();
        for render in renders {
            match render {
                AttachmentRender::Image(url) => {
                    media.push(UserContent::image_url(&url, None, None));
                }
                AttachmentRender::Audio { data, format } => {
                    media.push(UserContent::Audio(Audio {
                        data: DocumentSourceKind::Base64(data),
                        media_type: Some(format),
                        ..Default::default()
                    }));
                }
                AttachmentRender::Text(description) => {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&description);
                }
            }
        }

        let mut content = OneOrMany::one(UserContent::text(&text));
        for item in media {
            content.push(item);
        }

        RigMessage::User { content }
    }

    /// Renders this message's attachments under `mode`, deciding per modality: a voice note is sent
    /// as native audio (when encoded) or as its cached transcription; an image is sent as an image
    /// URL or as its cached description. A missing encoding or text falls back to a placeholder.
    fn render_attachments(&self, mode: AttachmentMode) -> Vec<AttachmentRender> {
        self.attachments
            .iter()
            .map(|url| {
                if is_audio_url(url) {
                    self.render_audio(url, mode.audio)
                } else {
                    self.render_image(url, mode.image)
                }
            })
            .collect()
    }

    /// Renders a single voice attachment under `mode`.
    fn render_audio(&self, url: &str, mode: MediaMode) -> AttachmentRender {
        match mode {
            MediaMode::Native => self.encoded_audio_for(url).map_or_else(
                || AttachmentRender::Text(UNDESCRIBED_AUDIO.to_owned()),
                |encoded| AttachmentRender::Audio {
                    data: encoded.data.clone(),
                    format: encoded.format.clone(),
                },
            ),
            MediaMode::Describe => AttachmentRender::Text(self.description_for(url).map_or_else(
                || UNDESCRIBED_AUDIO.to_owned(),
                |transcription| format!("[Ljud: {transcription}]"),
            )),
        }
    }

    /// Renders a single image attachment under `mode`.
    fn render_image(&self, url: &str, mode: MediaMode) -> AttachmentRender {
        match mode {
            MediaMode::Native => AttachmentRender::Image(url.to_owned()),
            MediaMode::Describe => AttachmentRender::Text(self.description_for(url).map_or_else(
                || UNDESCRIBED_IMAGE.to_owned(),
                |description| format!("[Bild: {description}]"),
            )),
        }
    }

    /// The non-voice attachment URLs that do not yet have a cached description.
    #[must_use]
    pub fn undescribed_image_urls(&self) -> Vec<String> {
        self.attachments
            .iter()
            .filter(|url| !is_audio_url(url))
            .filter(|url| self.description_for(url).is_none())
            .cloned()
            .collect()
    }

    /// Whether this message has at least one image (non-voice) attachment.
    #[must_use]
    pub fn has_image_attachment(&self) -> bool {
        self.attachments.iter().any(|url| !is_audio_url(url))
    }

    /// Whether this message has at least one voice (audio) attachment.
    #[must_use]
    pub fn has_audio_attachment(&self) -> bool {
        self.attachments.iter().any(|url| is_audio_url(url))
    }

    /// All voice attachment URLs on this message.
    #[must_use]
    pub fn audio_attachment_urls(&self) -> Vec<String> {
        self.attachments
            .iter()
            .filter(|url| is_audio_url(url))
            .cloned()
            .collect()
    }

    /// The voice attachment URLs that do not yet have a cached transcription.
    #[must_use]
    pub fn undescribed_audio_urls(&self) -> Vec<String> {
        self.attachments
            .iter()
            .filter(|url| is_audio_url(url))
            .filter(|url| self.description_for(url).is_none())
            .cloned()
            .collect()
    }

    /// The cached base64 encoding for the voice attachment at `url`, if one has been made.
    #[must_use]
    pub fn encoded_audio_for(&self, url: &str) -> Option<&EncodedAudio> {
        self.encoded_audio
            .iter()
            .find(|encoded| encoded.url.as_str() == url)
    }

    /// Merges in any encodings whose URL matches one of this message's attachments, skipping URLs
    /// that are already encoded.
    pub fn add_encoded_audio(&mut self, new: &[EncodedAudio]) {
        merge_owned(&self.attachments, &mut self.encoded_audio, new, |encoded| {
            encoded.url.as_str()
        });
    }

    /// The cached description for `url`, if one has been generated.
    #[must_use]
    pub fn description_for(&self, url: &str) -> Option<&str> {
        self.described
            .iter()
            .find(|described| described.url.as_str() == url)
            .map(|described| described.description.as_str())
    }

    /// Merges in any descriptions whose URL matches one of this message's attachments, skipping
    /// URLs that are already described.
    pub fn add_descriptions(&mut self, new: &[DescribedAttachment]) {
        merge_owned(&self.attachments, &mut self.described, new, |described| {
            described.url.as_str()
        });
    }

    /// The number of cached attachment descriptions, for tests.
    #[cfg(test)]
    #[must_use]
    pub const fn described_count(&self) -> usize {
        self.described.len()
    }
}

/// Merges each item from `new` into `target` when its URL (via `url_of`) belongs to one of
/// `attachments` and is not already cached. Shared by the encoded-audio and description caches.
fn merge_owned<T: Clone>(
    attachments: &[String],
    target: &mut Vec<T>,
    new: &[T],
    url_of: impl Fn(&T) -> &str,
) {
    for item in new {
        let url = url_of(item);
        let owned = attachments
            .iter()
            .any(|attachment| attachment.as_str() == url);
        let already = target.iter().any(|existing| url_of(existing) == url);
        if owned && !already {
            target.push(item.clone());
        }
    }
}

/// The audio media type for `url`'s extension, or `None` when it is not a recognized audio file.
///
/// Discord voice messages are `.ogg`; users may also upload other common audio files, most
/// notably `.mp3`. The query string is ignored and the extension matched case-insensitively.
pub fn audio_format_from_url(url: &str) -> Option<AudioMediaType> {
    let path = url.split('?').next().unwrap_or(url);
    let extension = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())?;
    match extension.as_str() {
        "ogg" | "oga" | "opus" => Some(AudioMediaType::OGG),
        "mp3" => Some(AudioMediaType::MP3),
        "wav" => Some(AudioMediaType::WAV),
        "m4a" => Some(AudioMediaType::M4A),
        "aac" => Some(AudioMediaType::AAC),
        "flac" => Some(AudioMediaType::FLAC),
        _ => None,
    }
}

/// Whether `url` points at a recognized audio attachment (a voice message or an audio file).
fn is_audio_url(url: &str) -> bool {
    audio_format_from_url(url).is_some()
}

/// Tests for attachment rendering, the description/encoding caches, and audio-format detection.
#[cfg(test)]
mod tests {
    use super::super::{Message, Part, Role};
    use super::{
        AttachmentMode, AttachmentRender, DescribedAttachment, EncodedAudio, MediaMode,
        audio_format_from_url,
    };
    use rig::message::{AudioMediaType, Message as RigMessage, UserContent};

    /// Each part's role maps to the matching rig message variant.
    #[test]
    fn rig_messages_map_each_part_role() {
        let message = Message::builder()
            .parts(vec![
                Part::builder()
                    .content("sys".to_owned())
                    .role(Role::System)
                    .build(),
                Part::builder()
                    .content("reply".to_owned())
                    .role(Role::Assistant)
                    .build(),
                Part::builder()
                    .content("hi".to_owned())
                    .role(Role::User)
                    .build(),
            ])
            .build();
        let rig_messages = message.to_rig_messages(AttachmentMode::default());
        assert_eq!(rig_messages.len(), 3, "each part maps to one rig message");
        assert!(
            matches!(rig_messages.first(), Some(RigMessage::System { .. })),
            "a system part maps to a system message"
        );
        assert!(
            matches!(rig_messages.get(1), Some(RigMessage::Assistant { .. })),
            "an assistant part maps to an assistant message"
        );
        assert!(
            matches!(rig_messages.get(2), Some(RigMessage::User { .. })),
            "a user part maps to a user message"
        );
    }

    /// Builds a user message with the given attachment URLs and cached descriptions.
    fn message_with_attachments(
        attachments: Vec<String>,
        described: Vec<DescribedAttachment>,
    ) -> Message {
        Message::builder()
            .parts(("Alice: hi".to_owned(), Role::User))
            .attachments(attachments)
            .described(described)
            .build()
    }

    /// The attachment mode with both modalities set to `mode`.
    const fn both(mode: MediaMode) -> AttachmentMode {
        AttachmentMode {
            image: mode,
            audio: mode,
        }
    }

    /// Builds a user message carrying the given attachment URLs and cached audio encodings.
    fn message_with_encoded_audio(attachments: Vec<String>, encoded: Vec<EncodedAudio>) -> Message {
        Message::builder()
            .parts(("Alice: hi".to_owned(), Role::User))
            .attachments(attachments)
            .encoded_audio(encoded)
            .build()
    }

    /// Native mode keeps an image as an image URL and an encoded voice note as base64 audio.
    #[test]
    fn render_native_mode_keeps_images_and_encoded_audio() {
        let message = message_with_encoded_audio(
            vec![
                "https://cdn/img.png".to_owned(),
                "https://cdn/voice.ogg".to_owned(),
            ],
            vec![EncodedAudio {
                url: "https://cdn/voice.ogg".to_owned(),
                data: "AAAA".to_owned(),
                format: AudioMediaType::OGG,
            }],
        );
        assert_eq!(
            message.render_attachments(both(MediaMode::Native)),
            vec![
                AttachmentRender::Image("https://cdn/img.png".to_owned()),
                AttachmentRender::Audio {
                    data: "AAAA".to_owned(),
                    format: AudioMediaType::OGG,
                },
            ],
            "native mode sends images as URLs and encoded voice notes as base64 audio"
        );
    }

    /// Native audio falls back to a placeholder when the clip was not encoded (download failed).
    #[test]
    fn render_native_audio_falls_back_when_unencoded() {
        let message =
            message_with_attachments(vec!["https://cdn/voice.ogg".to_owned()], Vec::new());
        assert_eq!(
            message.render_attachments(both(MediaMode::Native)),
            vec![AttachmentRender::Text(
                "[Ljud kunde inte tolkas]".to_owned()
            )],
            "an unencoded voice note falls back to a placeholder under native mode"
        );
    }

    /// Describe mode swaps a described image and a transcribed voice note for their cached text.
    #[test]
    fn render_describe_mode_uses_cached_descriptions() {
        let message = message_with_attachments(
            vec![
                "https://cdn/img.png".to_owned(),
                "https://cdn/voice.ogg".to_owned(),
            ],
            vec![
                DescribedAttachment {
                    url: "https://cdn/img.png".to_owned(),
                    description: "en katt".to_owned(),
                },
                DescribedAttachment {
                    url: "https://cdn/voice.ogg".to_owned(),
                    description: "hej där".to_owned(),
                },
            ],
        );
        assert_eq!(
            message.render_attachments(both(MediaMode::Describe)),
            vec![
                AttachmentRender::Text("[Bild: en katt]".to_owned()),
                AttachmentRender::Text("[Ljud: hej där]".to_owned()),
            ],
            "describe mode swaps a described image and a transcribed voice note for their text"
        );
    }

    /// Describe mode falls back to per-kind placeholders for an undescribed image and voice note.
    #[test]
    fn render_describe_mode_falls_back_for_undescribed_attachments() {
        let message = message_with_attachments(
            vec![
                "https://cdn/img.png".to_owned(),
                "https://cdn/voice.ogg".to_owned(),
            ],
            Vec::new(),
        );
        assert_eq!(
            message.render_attachments(both(MediaMode::Describe)),
            vec![
                AttachmentRender::Text("[Bild kunde inte tolkas]".to_owned()),
                AttachmentRender::Text("[Ljud kunde inte tolkas]".to_owned()),
            ],
            "undescribed attachments fall back to their own placeholders"
        );
    }

    /// Image and audio modes are independent: an image can stay native while audio is transcribed.
    #[test]
    fn render_mixes_native_image_with_described_audio() {
        let message = message_with_attachments(
            vec![
                "https://cdn/img.png".to_owned(),
                "https://cdn/voice.ogg".to_owned(),
            ],
            vec![DescribedAttachment {
                url: "https://cdn/voice.ogg".to_owned(),
                description: "hej".to_owned(),
            }],
        );
        assert_eq!(
            message.render_attachments(AttachmentMode {
                image: MediaMode::Native,
                audio: MediaMode::Describe,
            }),
            vec![
                AttachmentRender::Image("https://cdn/img.png".to_owned()),
                AttachmentRender::Text("[Ljud: hej]".to_owned()),
            ],
            "a native-image, describe-audio mode renders each modality on its own terms"
        );
    }

    /// `undescribed_image_urls` lists only non-voice attachments without a cached description.
    #[test]
    fn undescribed_image_urls_excludes_audio_and_described() {
        let message = message_with_attachments(
            vec![
                "https://cdn/a.png".to_owned(),
                "https://cdn/b.png".to_owned(),
                "https://cdn/voice.ogg".to_owned(),
            ],
            vec![DescribedAttachment {
                url: "https://cdn/a.png".to_owned(),
                description: "beskriven".to_owned(),
            }],
        );
        assert_eq!(
            message.undescribed_image_urls(),
            vec!["https://cdn/b.png".to_owned()],
            "only undescribed, non-voice attachments need describing"
        );
    }

    /// `add_descriptions` merges descriptions for owned URLs once, ignoring unrelated URLs.
    #[test]
    fn add_descriptions_merges_matching_urls_without_duplicates() {
        let mut message =
            message_with_attachments(vec!["https://cdn/a.png".to_owned()], Vec::new());
        let descriptions = vec![
            DescribedAttachment {
                url: "https://cdn/a.png".to_owned(),
                description: "katt".to_owned(),
            },
            DescribedAttachment {
                url: "https://cdn/other.png".to_owned(),
                description: "ovidkommande".to_owned(),
            },
        ];
        message.add_descriptions(&descriptions);
        message.add_descriptions(&descriptions);
        assert_eq!(
            message.described_count(),
            1,
            "only the matching url merges, and a second merge adds no duplicate"
        );
        assert_eq!(
            message.description_for("https://cdn/a.png"),
            Some("katt"),
            "the matching url is described"
        );
    }

    /// Recognized audio extensions are classified as audio, everything else as images.
    #[test]
    fn audio_and_image_attachments_are_classified_by_extension() {
        let message = message_with_attachments(
            vec![
                "https://cdn/img.png".to_owned(),
                "https://cdn/voice.ogg".to_owned(),
                "https://cdn/clip.mp3".to_owned(),
            ],
            Vec::new(),
        );
        assert!(
            message.has_audio_attachment(),
            "an .ogg or .mp3 attachment is an audio attachment"
        );
        assert!(
            message.has_image_attachment(),
            "a non-audio attachment is an image attachment"
        );
        assert_eq!(
            message.audio_attachment_urls(),
            vec![
                "https://cdn/voice.ogg".to_owned(),
                "https://cdn/clip.mp3".to_owned()
            ],
            "both the .ogg and the .mp3 are listed as audio, the .png is not"
        );
    }

    /// Audio formats are detected from common extensions; non-audio extensions yield `None`.
    #[test]
    fn audio_format_is_detected_from_common_extensions() {
        assert_eq!(
            audio_format_from_url("https://cdn/voice.ogg"),
            Some(AudioMediaType::OGG),
            "an .ogg attachment is OGG audio"
        );
        assert_eq!(
            audio_format_from_url("https://cdn/clip.MP3?ex=123"),
            Some(AudioMediaType::MP3),
            "an .mp3 is MP3 audio, case-insensitively and ignoring the query string"
        );
        assert_eq!(
            audio_format_from_url("https://cdn/song.wav"),
            Some(AudioMediaType::WAV),
            "a .wav is WAV audio"
        );
        assert_eq!(
            audio_format_from_url("https://cdn/img.png"),
            None,
            "an image extension is not audio"
        );
        assert_eq!(
            audio_format_from_url("https://cdn/noext"),
            None,
            "a URL without an extension is not audio"
        );
    }

    /// `undescribed_audio_urls` lists only voice attachments without a cached transcription.
    #[test]
    fn undescribed_audio_urls_excludes_images_and_transcribed() {
        let message = message_with_attachments(
            vec![
                "https://cdn/a.ogg".to_owned(),
                "https://cdn/b.ogg".to_owned(),
                "https://cdn/img.png".to_owned(),
            ],
            vec![DescribedAttachment {
                url: "https://cdn/a.ogg".to_owned(),
                description: "transkriberad".to_owned(),
            }],
        );
        assert_eq!(
            message.undescribed_audio_urls(),
            vec!["https://cdn/b.ogg".to_owned()],
            "only untranscribed voice attachments need transcribing"
        );
    }

    /// `add_encoded_audio` merges encodings for owned URLs once, ignoring unrelated URLs.
    #[test]
    fn add_encoded_audio_merges_matching_urls_without_duplicates() {
        let mut message =
            message_with_attachments(vec!["https://cdn/voice.ogg".to_owned()], Vec::new());
        let encoded = vec![
            EncodedAudio {
                url: "https://cdn/voice.ogg".to_owned(),
                data: "AAAA".to_owned(),
                format: AudioMediaType::OGG,
            },
            EncodedAudio {
                url: "https://cdn/other.ogg".to_owned(),
                data: "BBBB".to_owned(),
                format: AudioMediaType::OGG,
            },
        ];
        message.add_encoded_audio(&encoded);
        message.add_encoded_audio(&encoded);
        assert_eq!(
            message
                .encoded_audio_for("https://cdn/voice.ogg")
                .map(|audio| audio.data.as_str()),
            Some("AAAA"),
            "the matching url is encoded"
        );
        assert!(
            message.encoded_audio_for("https://cdn/other.ogg").is_none(),
            "an unrelated url is not encoded onto this message"
        );
    }

    /// Native audio sends an encoded voice note as its own base64 audio content part.
    #[test]
    fn native_audio_becomes_a_separate_audio_content_part() {
        let message = message_with_encoded_audio(
            vec!["https://cdn/voice.ogg".to_owned()],
            vec![EncodedAudio {
                url: "https://cdn/voice.ogg".to_owned(),
                data: "AAAA".to_owned(),
                format: AudioMediaType::OGG,
            }],
        );
        let voiced = message.to_rig_messages(AttachmentMode::default()).pop();
        assert!(
            matches!(voiced, Some(RigMessage::User { .. })),
            "the message maps to a user message"
        );
        let Some(RigMessage::User { content }) = voiced else {
            return;
        };
        assert_eq!(
            content.iter().count(),
            2,
            "native audio sends the text plus the audio as two content parts"
        );
        assert!(
            content
                .iter()
                .any(|part| matches!(part, UserContent::Text(_))),
            "one part carries the text"
        );
        assert!(
            content
                .iter()
                .any(|part| matches!(part, UserContent::Audio(_))),
            "the other part carries the audio natively"
        );
    }

    /// Describe mode folds the description into one text content, so a text-only model receives a
    /// plain string rather than a multi-part content array (which such models reject).
    #[test]
    fn describe_mode_folds_descriptions_into_a_single_text_content() {
        let message = message_with_attachments(
            vec!["https://cdn/img.png".to_owned()],
            vec![DescribedAttachment {
                url: "https://cdn/img.png".to_owned(),
                description: "en katt".to_owned(),
            }],
        );
        let described = message.to_rig_messages(both(MediaMode::Describe)).pop();
        assert!(
            matches!(described, Some(RigMessage::User { .. })),
            "the message maps to a user message"
        );
        let Some(RigMessage::User { content }) = described else {
            return;
        };
        assert_eq!(
            content.iter().count(),
            1,
            "describe mode keeps the user message as a single text content"
        );
        assert!(
            matches!(content.iter().next(), Some(UserContent::Text(_))),
            "the single content is text, with the description folded in"
        );
    }

    /// Image mode keeps the image as its own content part alongside the text.
    #[test]
    fn image_mode_keeps_the_image_as_a_separate_content_part() {
        let message = message_with_attachments(vec!["https://cdn/img.png".to_owned()], Vec::new());
        let imaged = message.to_rig_messages(AttachmentMode::default()).pop();
        assert!(
            matches!(imaged, Some(RigMessage::User { .. })),
            "the message maps to a user message"
        );
        let Some(RigMessage::User { content }) = imaged else {
            return;
        };
        assert_eq!(
            content.iter().count(),
            2,
            "image mode sends the text plus the image as two content parts"
        );
        assert!(
            content
                .iter()
                .any(|part| matches!(part, UserContent::Image(_))),
            "one part carries the image"
        );
    }

    /// An attachment survives even when every line is role-prefixed, so the message has no user
    /// part to carry it: it is appended as a trailing user message instead of vanishing.
    #[test]
    fn attachments_without_a_user_part_become_a_trailing_user_message() {
        let message = Message::builder()
            .parts(("system: do this".to_owned(), Role::System))
            .attachments(vec!["https://cdn/img.png".to_owned()])
            .build();
        let messages = message.to_rig_messages(AttachmentMode::default());
        assert!(
            matches!(messages.first(), Some(RigMessage::System { .. })),
            "the prefixed line stays a system message"
        );
        let last = messages.last();
        assert!(
            matches!(last, Some(RigMessage::User { .. })),
            "the attachment is carried by a trailing user message"
        );
        let Some(RigMessage::User { content }) = last else {
            return;
        };
        assert_eq!(
            content.iter().count(),
            2,
            "the trailing user message holds the empty text plus the image"
        );
        assert!(
            content
                .iter()
                .any(|part| matches!(part, UserContent::Image(_))),
            "the trailing user message carries the image"
        );
    }
}
