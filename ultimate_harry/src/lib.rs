use poise::serenity_prelude::{
    ComponentInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
};
use std::time::Duration;

pub type Context<'a> = poise::Context<'a, (), Error>;
pub type Result<T, E = Error> = std::result::Result<T, E>;
pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

pub const FIVE_SECONDS: Duration = Duration::from_secs(5);
pub const ONE_MINUTE: Duration = Duration::from_secs(60);
pub const TEN_MINUTES: Duration = Duration::from_secs(60 * 10);
pub const ONE_HOUR: Duration = Duration::from_secs(60 * 60);

/// If this is an application command,
/// `poise::structs::context::ApplicationContext::defer_ephemeral` is called.
///
/// If this is a prefix command, a typing broadcast is started
/// until the return value is dropped.
pub trait DeferEphemeralOrBroadcast {
    // TODO: remove eventually?
    #[allow(async_fn_in_trait)] // i'm only using this in my code
    async fn defer_ephemeral_or_broadcast(&self) -> Result<()>;
}

impl DeferEphemeralOrBroadcast for Context<'_> {
    /// If this is an application command,
    /// `poise::structs::context::ApplicationContext::defer_ephemeral` is called.
    ///
    /// If this is a prefix command, a typing broadcast is started
    /// until the return value is dropped.
    async fn defer_ephemeral_or_broadcast(&self) -> Result<()> {
        match self {
            Context::Application(ctx) => {
                ctx.defer_ephemeral().await?;
            }
            Context::Prefix(ctx) => {
                ctx.defer_or_broadcast().await?;
            }
        };
        Ok(())
    }
}

pub trait RespondWith {
    // TODO: remove eventually?
    #[allow(async_fn_in_trait)] // i'm only using this in my code
    async fn respond_with(&self, ctx: Context<'_>, message: impl AsRef<str>) -> Result<()>;
}

impl RespondWith for ComponentInteraction {
    async fn respond_with(&self, ctx: Context<'_>, message: impl AsRef<str>) -> Result<()> {
        self.create_response(
            ctx,
            CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new()
                    .content(message.as_ref())
                    .embeds(vec![])
                    .components(vec![]),
            ),
        )
        .await?;
        Ok(())
    }
}

pub async fn delete_invoking_message_if_prefix(ctx: Context<'_>) -> Result<()> {
    if let Context::Prefix(ctx) = ctx {
        ctx.msg.delete(ctx).await?;
    }
    Ok(())
}
