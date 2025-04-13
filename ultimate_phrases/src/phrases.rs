use nanorand::Rng;

pub fn sample<T: Clone>(list: &[T]) -> T {
    let mut rng = nanorand::tls_rng();
    let index = rng.generate_range(0..list.len());
    list[index].clone()
}

pub fn sample_name(list: &[&str], name: impl AsRef<str>) -> String {
    let mut rng = nanorand::tls_rng();
    let index = rng.generate_range(0..list.len());
    list[index].replace("{character}", name.as_ref())
}

pub const YES_PHRASES: &[&str] = &[
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

pub const NO_PHRASES: &[&str] = &[
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

pub const NO_CHARACTER_PHRASES: &[&str] = &[
    "Ursäkta, vad sade du nyss? (ingen gubbe hittades)",
    "Det finns ingen som heter så. (ingen gubbe hittades)",
    "Skojar du med mig eller? (ingen gubbe hittades)",
    "Jag har ingen aning vad du menar. (ingen gubbe hittades)",
    "Prata lite tydligare kanske. (ingen gubbe hittades)",
    "Jag ber så hemskt mycket om ursäkt, men va? (ingen gubbe hittades)",
    "Pröva ge ett riktigt namn nästa gång. (ingen gubbe hittades)",
];

pub const ASK_EDIT_PHRASES: &[&str] = &[
    "Vill du verkligen redigera den här gubben?",
    "Vet du verkligen vad du gör just nu?",
    "Be en vuxen som du litar på att hjälpa dig innan du fortsätter.",
    "Åh gud, är du säker på detta?",
    "Jag har inte tänkt att stoppa dig, men se upp nu.",
    "Stänger av säkerhetsprotokoller…",
    "Är detta din gubbe?",
];

pub const ASK_DELETE_PHRASES: &[&str] = &[
    "Vill du verkligen döda den här gubben?",
    "Vet du verkligen vad du gör just nu?",
    "Be en vuxen som du litar på att hjälpa dig innan du fortsätter.",
    "Åh gud, är du säker på detta?",
    "Jag har inte tänkt att stoppa dig, men se upp nu.",
    "Stänger av säkerhetsprotokoller…",
    "Är detta din gubbe?",
];

pub const CANCELLED_PHRASES: &[&str] = &[
    "Trodde väl det.",
    "Ännu ett misslyckande.",
    "Du gjorde rätt sak i dag.",
    "Jag hade inte heller kunnat.",
    "Kanske du ändå skulle ha gjort det?",
    "Har du druckit, eller?",
    "Vi var så här nära storhet!",
    "Jag skulle ha gjort det om jag var du.",
];

pub const CREATED_PHRASES: &[&str] = &[
    "Fröja! {character} har fötts.",
    "Du är nu en stolt förälder av {character}.",
    "Skapade du nyss liv ur ingenting…? {character} skrämmer mig…",
    "Toppen, ännu en gubbe, alltså? {character}, vilket perfekt namn, verkligen.",
    "Haha, den var bra ändå! Jag älskar {character}!",
    "Gud vad kreativ du är, ändå bra jobbat med {character}!",
    "Jag hade inte kunnat skapa en bättre {character} själv.",
];

pub const EDITED_PHRASES: &[&str] = &[
    "Din perfekta varelse, {character}, är nu ännu mer perfekt.",
    "Wow, {character} är bättre än någonsin.",
    "Haha, den här nya {character} var bättre ändå!",
    "Gud vad kreativ du är, snyggt jobbat med {character}!",
    "Betyder det här att du gjorde ett misstag med {character} tidigare…?",
];

pub const DELETED_PHRASES: &[&str] = &[
    "Du gjorde det du behövde.",
    "Den hade det kommande.",
    "Ändamålet helgar medlen.",
    "Skyll inte på dig själv.",
    "Du gjorde det för ditt lands skull.",
    "Du vet inte vad du nyss gjorde.",
    "Bra jobbat, soldat.",
    "Stå för rättvisa.",
    "En ängel gråter.",
];

pub const EDITING_PHRASES: &[&str] = &["Konst håller på att skapas…"];

pub const TIMEOUT_PHRASES: &[&str] = &[
    "Tror du att jag har hela dagen på mig att vänta?",
    "Jag är en upptagen robot. Jag kommer inte att vänta på dig längre.",
    "Oj då, där tog du lite för lång tid på dig. Synd!",
];

pub const CLICK_BELOW_PHRASES: &[&str] = &[
    "Klicka på nedanstående knapp för att fortsätta.",
    "Klicka på nedanstående knapp om du vågar.",
    "Klicka på nedanstående knapp.",
    "Klicka inte på nedanstående knapp.",
    "Det gör ont för nedanstående knapp när du klickar på den. Gör det.",
    "Är du säker att du vågar?",
];

pub const CLICK_ME_PHRASES: &[&str] = &[
    "Klicka på mig.",
    "Klicka på mig!",
    "Snälla klicka på mig!",
    "Klicka på mig nu!",
    "Det gör ont när du klickar på mig!",
    "Klicka inte på mig.",
    "Är du säker att du vågar?",
];
