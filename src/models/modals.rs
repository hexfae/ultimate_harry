use poise::Modal;

#[derive(Modal)]
#[name = "Skapa gubbe"]
pub struct CreateCharacterModal {
    #[paragraph]
    #[name = "Namn"]
    #[placeholder = "Vad heter din nya skapelse?"]
    pub name: String,
    #[paragraph]
    #[name = "Hälsning"]
    #[placeholder = "Hur ska din vackra varelse hälsa på folk?"]
    pub greeting: String,
    #[paragraph]
    #[name = "Smeknamn"]
    #[placeholder = "Vad kallar du din gubbe kort och gott?"]
    pub nickname: Option<String>,
    #[paragraph]
    #[name = "Beskrivning"]
    #[placeholder = "En kort beskrivning för dig att komma ihåg gubben enklare. Läses inte av modellen."]
    pub description: Option<String>,
    #[paragraph]
    #[name = "Personlighet"]
    #[placeholder = "Hur ska gubben bete sig? Detta ska vara i första person, alltså \"Jag heter, jag skriver, jag gör…\""]
    pub personality: Option<String>,
}

#[derive(Modal)]
#[name = "Skapa gubbe"]
pub struct SecondCreateCharacterModal {
    #[paragraph]
    #[name = "Profilbild"]
    #[placeholder = "En länk till en bild."]
    pub avatar: Option<String>,
    #[paragraph]
    #[name = "Emoji"]
    #[placeholder = "Till exempel 🤩, :robot:, eller :cholol:, syns oftast bredvid gubbens namn."]
    pub emoji: Option<String>,
    #[paragraph]
    #[name = "Systemprompt"]
    #[placeholder = "Placeras alltid i slutet som systeminstruktioner. Kan kanske ha väldigt stort inflytande på gubben?"]
    pub system_prompt: Option<String>,
    #[paragraph]
    #[name = "Prompt"]
    #[placeholder = "En lista av instruktioner för gubben, till exempel \"Skriv korta meningar, skriv 2-4 stycken.\""]
    pub prompt: Option<String>,
    #[paragraph]
    #[name = "Scenario"]
    #[placeholder = "Scenariot som gubben har funnit sig själv i. Till exempel \"En bensinmack mitt ute i ingenstans.\""]
    pub scenario: Option<String>,
}

#[derive(Modal)]
#[name = "Ändra gubbe"]
pub struct EditCharacterModal {
    #[paragraph]
    #[name = "Namn"]
    #[placeholder = "Vad ska din perfekta skapelse EGENTLIGEN heta?"]
    pub name: Option<String>,
    #[paragraph]
    #[name = "Hälsning"]
    #[placeholder = "Men vad skulle gubben faktiskt säga som hälsning då?"]
    pub greeting: Option<String>,
    #[paragraph]
    #[name = "Smeknamn"]
    #[placeholder = "Dens smeknamn då, det skulle ju vara…?"]
    pub nickname: Option<String>,
    #[paragraph]
    #[name = "Beskrivning"]
    #[placeholder = "Sedan var det beskrivningen, det här lilla korta för dig, som inte läses av modellen alltså."]
    pub description: Option<String>,
    #[paragraph]
    #[name = "Personlighet"]
    #[placeholder = "Gubbens personlighet, typ beteende. I första person, alltså \"Jag heter, jag skriver, jag gör…\"."]
    pub personality: Option<String>,
}

#[derive(Modal)]
#[name = "Ändra gubbe"]
pub struct SecondEditCharacterModal {
    #[paragraph]
    #[name = "Profilbild"]
    #[placeholder = "En länk till rätt bild."]
    pub avatar: Option<String>,
    #[name = "Emoji"]
    #[placeholder = "Till exempel 🤩, :robot:, eller :chosad:, syns oftast bredvid gubbens namn."]
    pub emoji: Option<String>,
    #[paragraph]
    #[name = "Systemprompt"]
    #[placeholder = "Placeras alltid i slutet som systeminstruktioner. Kan kanske ha väldigt stort inflytande på gubben?"]
    pub system_prompt: Option<String>,
    #[paragraph]
    #[name = "Prompt"]
    #[placeholder = "En lista av instruktioner för gubben, till exempel \"Skriv korta meningar, skriv 2-4 stycken.\""]
    pub prompt: Option<String>,
    #[paragraph]
    #[name = "Scenario"]
    #[placeholder = "Scenariot som gubben har funnit sig själv i. Till exempel \"En bensinmack mitt ute i ingenstans.\""]
    pub scenario: Option<String>,
}

#[derive(Modal)]
#[name = "Ändra meddelande"]
pub struct EditMessageModal {
    #[paragraph]
    #[name = "Meddelande"]
    #[placeholder = "Vad ska egentligen stå här?"]
    pub content: String,
}
