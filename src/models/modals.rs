//! Modal forms for creating and editing characters and messages.

use poise::Modal;

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
    pub name: String,
    /// The character's greeting.
    #[paragraph]
    #[name = "Hälsning"]
    #[placeholder = "Hur ska din vackra varelse hälsa på folk?"]
    pub greeting: String,
    /// The character's nickname.
    #[paragraph]
    #[name = "Smeknamn"]
    #[placeholder = "Vad kallar du din gubbe kort och gott?"]
    pub nickname: Option<String>,
    /// The character's description.
    #[paragraph]
    #[name = "Beskrivning"]
    #[placeholder = "En kort beskrivning för dig att komma ihåg gubben enklare. Läses inte av modellen."]
    pub description: Option<String>,
    /// The character's personality.
    #[paragraph]
    #[name = "Personlighet"]
    #[placeholder = "Hur ska gubben bete sig? Detta ska vara i första person, alltså \"Jag heter, jag skriver, jag gör…\""]
    pub personality: Option<String>,
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
    pub avatar: Option<String>,
    /// The character's emoji.
    #[paragraph]
    #[name = "Emoji"]
    #[placeholder = "Till exempel 🤩, :robot:, eller :cholol:, syns oftast bredvid gubbens namn."]
    pub emoji: Option<String>,
    /// The character's system prompt.
    #[paragraph]
    #[name = "Systemprompt"]
    #[placeholder = "Placeras alltid i slutet som systeminstruktioner. Kan kanske ha väldigt stort inflytande på gubben?"]
    pub system_prompt: Option<String>,
    /// The character's prompt.
    #[paragraph]
    #[name = "Prompt"]
    #[placeholder = "En lista av instruktioner för gubben, till exempel \"Skriv korta meningar, skriv 2-4 stycken.\""]
    pub prompt: Option<String>,
    /// The character's scenario.
    #[paragraph]
    #[name = "Scenario"]
    #[placeholder = "Scenariot som gubben har funnit sig själv i. Till exempel \"En bensinmack mitt ute i ingenstans.\""]
    pub scenario: Option<String>,
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
    pub name: Option<String>,
    /// The character's new greeting.
    #[paragraph]
    #[name = "Hälsning"]
    #[placeholder = "Men vad skulle gubben faktiskt säga som hälsning då?"]
    pub greeting: Option<String>,
    /// The character's new nickname.
    #[paragraph]
    #[name = "Smeknamn"]
    #[placeholder = "Dens smeknamn då, det skulle ju vara…?"]
    pub nickname: Option<String>,
    /// The character's new description.
    #[paragraph]
    #[name = "Beskrivning"]
    #[placeholder = "Sedan var det beskrivningen, det här lilla korta för dig, som inte läses av modellen alltså."]
    pub description: Option<String>,
    /// The character's new personality.
    #[paragraph]
    #[name = "Personlighet"]
    #[placeholder = "Gubbens personlighet, typ beteende. I första person, alltså \"Jag heter, jag skriver, jag gör…\"."]
    pub personality: Option<String>,
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
    pub avatar: Option<String>,
    /// The character's new emoji.
    #[name = "Emoji"]
    #[placeholder = "Till exempel 🤩, :robot:, eller :chosad:, syns oftast bredvid gubbens namn."]
    pub emoji: Option<String>,
    /// The character's new system prompt.
    #[paragraph]
    #[name = "Systemprompt"]
    #[placeholder = "Placeras alltid i slutet som systeminstruktioner. Kan kanske ha väldigt stort inflytande på gubben?"]
    pub system_prompt: Option<String>,
    /// The character's new prompt.
    #[paragraph]
    #[name = "Prompt"]
    #[placeholder = "En lista av instruktioner för gubben, till exempel \"Skriv korta meningar, skriv 2-4 stycken.\""]
    pub prompt: Option<String>,
    /// The character's new scenario.
    #[paragraph]
    #[name = "Scenario"]
    #[placeholder = "Scenariot som gubben har funnit sig själv i. Till exempel \"En bensinmack mitt ute i ingenstans.\""]
    pub scenario: Option<String>,
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
    pub content: String,
}
