use crate::{SendMessageSnafu, db::Database, events::message::HistoryCharacter};
use miette::Report;
use serenity::all::{
    ComponentInteraction, Context, CreateInteractionResponse, CreateInteractionResponseMessage,
    MessageId,
};
use snafu::ResultExt;

pub async fn pin(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<(), Report> {
    let Some((history, character)) = id.history_character(db).await? else {
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

    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new().content(pin.link().to_string()),
            ),
        )
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}
