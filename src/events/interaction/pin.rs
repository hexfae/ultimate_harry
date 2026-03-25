use crate::{db::Database, events::message::history_and_character_of};
use miette::{Diagnostic, Report};
use serenity::all::{
    ComponentInteraction, Context, CreateInteractionResponse, CreateInteractionResponseMessage,
    MessageId,
};
use snafu::{ResultExt as _, Snafu};

#[derive(Debug, Snafu, Diagnostic)]
enum PinReplyError {
    #[snafu(display("Kunde inte skicka meddelandet: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är tillgänglig"),
        code(events::interaction::pin::send_message)
    )]
    SendMessage { source: serenity::Error },
}

pub async fn pin(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<(), Report> {
    let Some((history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    let reply = history
        .into_bare_response(&character, interaction.message.link().to_string(), db)
        .await;

    let pins_channel_id = db.pins_channel().await;
    let pin = pins_channel_id
        .widen()
        .send_message(&ctx.http, reply.to_prefix((&*interaction.message).into()))
        .await
        .context(SendMessageSnafu)?;

    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new().content(pin.link().to_string()),
    );

    interaction
        .create_response(&ctx.http, response)
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}
