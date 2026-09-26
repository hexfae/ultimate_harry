//! Modal forms for creating and editing characters and messages.

use crate::shortcodes::resolve;
use poise::Modal;
use poise::serenity_prelude::{Emoji, small_fixed_array::FixedString};

/// A modal text field's value: Discord caps modal input at 4000 characters, so
/// a fixed-capacity string carries it without an allocation.
type Field = FixedString<u16>;

/// The first modal form for creating a new character.
///
/// Contains basic information like name, greeting, nickname,
/// description, and personality.
#[derive(Debug, Modal)]
#[name = "Skapa gubbe"]
pub struct CreateCharacterModal {
    /// The character's name.
    #[paragraph]
    #[name = "Namn"]
    #[placeholder = "Vad heter din nya skapelse?"]
    pub name: Field,
    /// The character's greeting.
    #[paragraph]
    #[name = "Hälsning"]
    #[placeholder = "Hur ska din vackra varelse hälsa på folk?"]
    pub greeting: Field,
    /// The character's nickname.
    #[paragraph]
    #[name = "Smeknamn"]
    #[placeholder = "Vad kallar du din gubbe kort och gott?"]
    pub nickname: Option<Field>,
    /// The character's description.
    #[paragraph]
    #[name = "Beskrivning"]
    #[placeholder = "En kort beskrivning för dig att komma ihåg gubben enklare. Läses inte av modellen."]
    pub description: Option<Field>,
    /// The character's personality.
    #[paragraph]
    #[name = "Personlighet"]
    #[placeholder = "Hur ska gubben bete sig? Detta ska vara i första person, alltså \"Jag heter, jag skriver, jag gör…\""]
    pub personality: Option<Field>,
}

/// The second modal form for creating a new character.
///
/// Contains additional information like avatar, emoji,
/// system prompt, prompt, and scenario.
#[derive(Debug, Modal)]
#[name = "Skapa gubbe"]
pub struct SecondCreateCharacterModal {
    /// The character's avatar.
    #[paragraph]
    #[name = "Profilbild"]
    #[placeholder = "En länk till en bild."]
    pub avatar: Option<Field>,
    /// The character's emoji.
    #[paragraph]
    #[name = "Emoji"]
    #[placeholder = "Till exempel 🤩, :robot:, eller :cholol:, syns oftast bredvid gubbens namn."]
    pub emoji: Option<Field>,
    /// The character's system prompt.
    #[paragraph]
    #[name = "Systemprompt"]
    #[placeholder = "Placeras alltid i slutet som systeminstruktioner. Kan kanske ha väldigt stort inflytande på gubben?"]
    pub system_prompt: Option<Field>,
    /// The character's prompt.
    #[paragraph]
    #[name = "Prompt"]
    #[placeholder = "En lista av instruktioner för gubben, till exempel \"Skriv korta meningar, skriv 2-4 stycken.\""]
    pub prompt: Option<Field>,
    /// The character's scenario.
    #[paragraph]
    #[name = "Scenario"]
    #[placeholder = "Scenariot som gubben har funnit sig själv i. Till exempel \"En bensinmack mitt ute i ingenstans.\""]
    pub scenario: Option<Field>,
}

/// The first modal form for editing an existing character.
///
/// Contains editable basic information like name, greeting,
/// nickname, description, and personality.
#[derive(Debug, Modal)]
#[name = "Ändra gubbe"]
pub struct EditCharacterModal {
    /// The character's new name.
    #[paragraph]
    #[name = "Namn"]
    #[placeholder = "Vad ska din perfekta skapelse EGENTLIGEN heta?"]
    pub name: Option<Field>,
    /// The character's new greeting.
    #[paragraph]
    #[name = "Hälsning"]
    #[placeholder = "Men vad skulle gubben faktiskt säga som hälsning då?"]
    pub greeting: Option<Field>,
    /// The character's new nickname.
    #[paragraph]
    #[name = "Smeknamn"]
    #[placeholder = "Dens smeknamn då, det skulle ju vara…?"]
    pub nickname: Option<Field>,
    /// The character's new description.
    #[paragraph]
    #[name = "Beskrivning"]
    #[placeholder = "Sedan var det beskrivningen, det här lilla korta för dig, som inte läses av modellen alltså."]
    pub description: Option<Field>,
    /// The character's new personality.
    #[paragraph]
    #[name = "Personlighet"]
    #[placeholder = "Gubbens personlighet, typ beteende. I första person, alltså \"Jag heter, jag skriver, jag gör…\"."]
    pub personality: Option<Field>,
}

/// The second modal form for editing an existing character.
///
/// Contains editable additional information like avatar, emoji,
/// system prompt, prompt, and scenario.
#[derive(Debug, Modal)]
#[name = "Ändra gubbe"]
pub struct SecondEditCharacterModal {
    /// The character's new avatar.
    #[paragraph]
    #[name = "Profilbild"]
    #[placeholder = "En länk till rätt bild."]
    pub avatar: Option<Field>,
    /// The character's new emoji.
    #[paragraph]
    #[name = "Emoji"]
    #[placeholder = "Till exempel 🤩, :robot:, eller :chosad:, syns oftast bredvid gubbens namn."]
    pub emoji: Option<Field>,
    /// The character's new system prompt.
    #[paragraph]
    #[name = "Systemprompt"]
    #[placeholder = "Placeras alltid i slutet som systeminstruktioner. Kan kanske ha väldigt stort inflytande på gubben?"]
    pub system_prompt: Option<Field>,
    /// The character's new prompt.
    #[paragraph]
    #[name = "Prompt"]
    #[placeholder = "En lista av instruktioner för gubben, till exempel \"Skriv korta meningar, skriv 2-4 stycken.\""]
    pub prompt: Option<Field>,
    /// The character's new scenario.
    #[paragraph]
    #[name = "Scenario"]
    #[placeholder = "Scenariot som gubben har funnit sig själv i. Till exempel \"En bensinmack mitt ute i ingenstans.\""]
    pub scenario: Option<Field>,
}

/// The modal form for adding an example-message pair to a character.
///
/// Two paragraph fields: an optional user line and the character's required
/// response to it. Kept separate from the create/edit forms, which are already at
/// Discord's five-field modal limit, so a pair could not be squeezed in there.
#[derive(Debug, Modal)]
#[name = "Nytt exempelmeddelande"]
pub struct ExampleMessageModal {
    /// The user's line the character responds to, if any. Left blank for an
    /// assistant-only example (a bare sample of how the character talks).
    #[paragraph]
    #[name = "Användare"]
    #[placeholder = "Vad användaren säger (lämna tomt för ett exempel utan användarrad)."]
    pub user: Option<Field>,
    /// The character's response, the actual example of how the character speaks.
    #[paragraph]
    #[name = "Svar"]
    #[placeholder = "Hur gubben svarar. Detta är exemplet på hur gubben ska prata."]
    pub response: Field,
}

/// The modal form for editing a message.
///
/// Contains a single text field for the new message content.
#[derive(Debug, Modal)]
#[name = "Ändra meddelande"]
pub struct EditMessageModal {
    /// The message's content.
    #[paragraph]
    #[name = "Meddelande"]
    #[placeholder = "Vad ska egentligen stå här?"]
    pub content: Field,
}

impl EditMessageModal {
    /// Builds the modal pre-filled with `content`, so the edit form opens
    /// populated with the existing text instead of blank.
    #[must_use]
    pub fn from_content(content: &str) -> Self {
        Self {
            content: Field::from_str_trunc(content),
        }
    }
}

/// Resolves shortcodes in an optional field in place, leaving `None` untouched.
fn resolve_field(field: &mut Option<Field>, guild_emojis: &[Emoji]) {
    if let Some(value) = field {
        *value = Field::from_string_trunc(resolve(value, guild_emojis));
    }
}

/// Resolves shortcodes in a required field in place.
fn resolve_required(field: &mut Field, guild_emojis: &[Emoji]) {
    *field = Field::from_string_trunc(resolve(field, guild_emojis));
}

/// Expands shortcodes in every text field of the character creation modals. The
/// avatar is a URL and so is left untouched.
#[expect(
    clippy::module_name_repetitions,
    reason = "resolves the create modals, which live in this module; the suffix names what it acts on"
)]
pub fn resolve_create_modals(
    first: &mut CreateCharacterModal,
    second: &mut SecondCreateCharacterModal,
    guild_emojis: &[Emoji],
) {
    resolve_required(&mut first.name, guild_emojis);
    resolve_required(&mut first.greeting, guild_emojis);
    resolve_field(&mut first.nickname, guild_emojis);
    resolve_field(&mut first.description, guild_emojis);
    resolve_field(&mut first.personality, guild_emojis);
    resolve_field(&mut second.emoji, guild_emojis);
    resolve_field(&mut second.system_prompt, guild_emojis);
    resolve_field(&mut second.prompt, guild_emojis);
    resolve_field(&mut second.scenario, guild_emojis);
}

/// Expands shortcodes in every text field of the character edit modals. The
/// avatar is a URL and so is left untouched.
#[expect(
    clippy::module_name_repetitions,
    reason = "resolves the edit modals, which live in this module; the suffix names what it acts on"
)]
pub fn resolve_edit_modals(
    first: &mut EditCharacterModal,
    second: &mut SecondEditCharacterModal,
    guild_emojis: &[Emoji],
) {
    resolve_field(&mut first.name, guild_emojis);
    resolve_field(&mut first.greeting, guild_emojis);
    resolve_field(&mut first.nickname, guild_emojis);
    resolve_field(&mut first.description, guild_emojis);
    resolve_field(&mut first.personality, guild_emojis);
    resolve_field(&mut second.emoji, guild_emojis);
    resolve_field(&mut second.system_prompt, guild_emojis);
    resolve_field(&mut second.prompt, guild_emojis);
    resolve_field(&mut second.scenario, guild_emojis);
}
