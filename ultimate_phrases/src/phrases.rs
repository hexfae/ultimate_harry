use nanorand::Rng;

pub fn sample<T: Clone>(list: &[T]) -> T {
    let mut rng = nanorand::tls_rng();
    let index = rng.generate_range(0..list.len());
    list[index].clone()
}

pub const NO_CHARACTER_PHRASES: &[&str] = &[
    "Ursäkta, vad sade du nyss?",
    "Det finns ingen som heter så.",
    "Skojar du med mig eller?",
    "Jag har ingen aning vad du menar.",
    "Prata lite tydligare kanske.",
    "Jag ber så hemskt mycket om ursäkt, men va?",
    "Pröva ge ett riktigt namn nästa gång.",
];

pub const ASK_DELETE_PHRASES: &[&str] = &[
    "Vill du verkligen döda den här gubben?",
    "Vet du verkligen vad du gör just nu?",
    "Be en vuxen som du litar på att hjälpa dig innan du fortsätter.",
    "Åh gud, är du säker på detta",
    "Jag har inte tänkt att stoppa dig, men se upp nu.",
    "Stänger av säkerhetsprotokoller…",
];

pub const CREATED_PHRASES: &[&str] = &[
    "Fröja! {character} har fötts.",
    "Du är nu en stolt förälder av {character}.",
    "Skapade du nyss liv ur ingenting…?",
    "Toppen, ännu en gubbe, alltså?",
    "Haha, den var bra ändå!",
    "Gud vad kreativ du är, bra jobbat!",
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

pub const CANCELLED_PHRASES: &[&str] = &[
    "Trodde väl det.",
    "Ännu ett misslyckande.",
    "Du gjorde rätt sak i dag.",
    "Jag hade inte heller kunnat.",
    "Kanske du ändå skulle ha gjort det?",
    "Har du druckit, eller?",
];
