//! Utility functions for generating random Swedish phrases used in bot responses.
//!
//! This module contains various phrases used throughout the bot for different
//! scenarios, such as confirming actions, asking for deletion, showing cancellation
//! messages, and more. Each set of phrases is randomly sampled to provide variety.

use core::fmt::Display;
use nanorand::Rng as _;

/// Returns a randomly sampled item from the given list.
fn sample<T: Clone + Default>(list: &[T]) -> T {
    let mut rng = nanorand::tls_rng();
    let index = rng.generate_range(0..list.len());
    list.get(index).cloned().unwrap_or_default()
}

/// Returns a randomly sampled phrase with `{character}` replaced by the given name.
fn sample_name<N: Display>(list: &[&str], name: N) -> String {
    sample(list).replace("{character}", &name.to_string())
}

/// Returns an affirmative phrase.
pub fn yes() -> &'static str {
    sample(YES_PHRASES)
}

/// Returns a negative phrase.
pub fn no() -> &'static str {
    sample(NO_PHRASES)
}

/// Returns a phrase indicating no character was found.
pub fn no_character() -> &'static str {
    sample(NO_CHARACTER_PHRASES)
}

/// Returns a phrase asking for delete confirmation.
pub fn ask_delete() -> &'static str {
    sample(ASK_DELETE_PHRASES)
}

/// Returns a cancellation phrase.
pub fn cancelled() -> &'static str {
    sample(CANCELLED_PHRASES)
}

/// Returns a character creation phrase with the character name inserted.
pub fn created<N: Display>(name: N) -> String {
    sample_name(CREATED_PHRASES, name)
}

/// Returns a character edit phrase with the character name inserted.
pub fn edited<N: Display>(name: N) -> String {
    sample_name(EDITED_PHRASES, name)
}

/// Returns a deletion phrase.
pub fn deleted() -> &'static str {
    sample(DELETED_PHRASES)
}

/// Returns a phrase asking for restore confirmation with the character name inserted.
pub fn ask_restore<N: Display>(name: N) -> String {
    sample_name(ASK_RESTORE_PHRASES, name)
}

/// Returns a restoration phrase with the character name inserted.
pub fn restored<N: Display>(name: N) -> String {
    sample_name(RESTORED_PHRASES, name)
}

/// Returns a rollback phrase.
pub fn rolled_back() -> &'static str {
    sample(ROLLED_BACK_PHRASES)
}

/// Returns a phrase instructing to click the button below.
pub fn click_below() -> &'static str {
    sample(CLICK_BELOW_PHRASES)
}

/// Returns a phrase instructing to click.
pub fn click_me() -> &'static str {
    sample(CLICK_ME_PHRASES)
}

/// Phrases for affirmative responses.
const YES_PHRASES: &[&str] = &[
    "Ja",
    "Japp",
    "Visst",
    "Varför inte",
    "Absolut",
    "Helst",
    "Ja tack",
    "Jo tack",
    "Okej",
    "Kör på",
    "Bekräfta",
];

/// Phrases for negative responses.
const NO_PHRASES: &[&str] = &[
    "Nej",
    "Näpp",
    "Absolut inte",
    "Helst inte",
    "Nej tack",
    "Lägg av",
    "Avbryt",
    "Sluta",
    "Stopp",
    "Som om",
];

/// Phrases for when no character is found.
const NO_CHARACTER_PHRASES: &[&str] = &[
    "Ursäkta, vad sade du nyss? (ingen gubbe hittades)",
    "Det finns ingen som heter så. (ingen gubbe hittades)",
    "Skojar du med mig eller? (ingen gubbe hittades)",
    "Jag har ingen aning vad du menar. (ingen gubbe hittades)",
    "Prata lite tydligare kanske. (ingen gubbe hittades)",
    "Jag ber så hemskt mycket om ursäkt, men va? (ingen gubbe hittades)",
    "Pröva ge ett riktigt namn nästa gång. (ingen gubbe hittades)",
    "Jag har aldrig träffat den här mannen i hela mitt liv. (ingen gubbe hittades)",
];

/// Phrases for asking delete confirmation.
const ASK_DELETE_PHRASES: &[&str] = &[
    "Vill du verkligen döda den här gubben?",
    "Vet du verkligen vad du gör just nu?",
    "Be en vuxen som du litar på att hjälpa dig innan du fortsätter.",
    "Åh gud, är du säker på detta?",
    "Jag har inte tänkt att stoppa dig, men se upp nu.",
    "Stänger av säkerhetsprotokoll…",
    "Är detta din gubbe?",
];

/// Phrases for cancellation.
const CANCELLED_PHRASES: &[&str] = &[
    "Trodde väl det.",
    "Ännu ett misslyckande.",
    "Du gjorde rätt sak i dag.",
    "Jag hade inte heller kunnat.",
    "Kanske du ändå skulle ha gjort det?",
    "Har du druckit, eller?",
    "Vi var så här nära storhet!",
    "Jag skulle ha gjort det om jag var du.",
];

/// Phrases for character creation, contains `{character}` placeholder.
const CREATED_PHRASES: &[&str] = &[
    "Fröjdas! {character} har fötts.",
    "Du är nu en stolt förälder av {character}.",
    "Skapade du nyss liv ur ingenting…? {character} skrämmer mig…",
    "Toppen, ännu en gubbe, alltså? {character}, vilket perfekt namn, verkligen.",
    "Haha, den var bra ändå! Jag älskar {character}!",
    "Gud vad kreativ du är, ändå bra jobbat med {character}!",
    "Jag hade inte kunnat skapa en bättre {character} själv.",
];

/// Phrases for character edit, contains `{character}` placeholder.
const EDITED_PHRASES: &[&str] = &[
    "Din perfekta varelse, {character}, är nu ännu mer perfekt.",
    "Wow, {character} är bättre än någonsin.",
    "Haha, den här nya {character} var bättre ändå!",
    "Gud vad kreativ du är, snyggt jobbat med {character}!",
    "Betyder det här att du gjorde ett misstag med {character} tidigare…?",
];

/// Phrases for character deletion.
const DELETED_PHRASES: &[&str] = &[
    "Du gjorde det du behövde.",
    "Den fick vad den förtjänade.",
    "Ändamålet helgar medlen.",
    "Skyll inte på dig själv.",
    "Du gjorde det för ditt lands skull.",
    "Du vet inte vad du nyss gjorde.",
    "Bra jobbat, soldat.",
    "Stå för rättvisa.",
    "En ängel gråter.",
];

/// Phrases for asking restore confirmation.
const ASK_RESTORE_PHRASES: &[&str] = &[
    "Vill du verkligen återuppliva {character}?",
    "Är du säker på att du vill väcka {character} till liv igen?",
    "Ska vi ge {character} en andra chans?",
    "Tillbaka från döden, alltså?",
    "Ångrar du dig redan?",
    "Är detta gubben du vill ha tillbaka?",
];

/// Phrases for character restoration.
const RESTORED_PHRASES: &[&str] = &[
    "{character} lever igen!",
    "Välkommen tillbaka, {character}.",
    "Återupplivad och redo igen.",
    "Döden var visst inte slutet ändå.",
    "Tillbaka som om ingenting hänt.",
    "En ängel ler.",
];

/// Phrases for rolling a character back to an older version.
const ROLLED_BACK_PHRASES: &[&str] = &[
    "Tillbaka till det förflutna.",
    "Som den en gång var.",
    "Rullade tillbaka tiden åt dig.",
    "Den gamla goda versionen är tillbaka.",
    "Ånej, var den nya versionen så dålig?",
    "Bättre förr, eller hur?",
];

/// Phrases for clicking the button below.
const CLICK_BELOW_PHRASES: &[&str] = &[
    "Klicka på nedanstående knapp för att fortsätta.",
    "Klicka på nedanstående knapp om du vågar.",
    "Klicka på nedanstående knapp.",
    "Klicka inte på nedanstående knapp.",
    "Det gör ont för nedanstående knapp när du klickar på den. Gör det.",
    "Är du säker att du vågar?",
];

/// Phrases for clicking me.
const CLICK_ME_PHRASES: &[&str] = &[
    "Klicka på mig.",
    "Klicka på mig!",
    "Snälla klicka på mig!",
    "Klicka på mig nu!",
    "Det gör ont när du klickar på mig!",
    "Klicka inte på mig.",
    "Är du säker att du vågar?",
];
