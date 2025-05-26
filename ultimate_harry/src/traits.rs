use crate::{
    Context, DeferSnafu, DeleteMessageSnafu, DeleteResponseSnafu, EditMessageSnafu,
    SendMessageSnafu, SendResponseSnafu, ShowModalSnafu, TEN_MINUTES,
};
use poise::{
    CreateReply, Modal, ReplyHandle, execute_modal_on_component_interaction,
    serenity_prelude::{
        ComponentInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
    },
};
use snafu::ResultExt;

type Result<T = ()> = std::result::Result<T, crate::Error>;

/// If this is an application command,
/// `poise::structs::context::ApplicationContext::defer_ephemeral` is called.
///
/// If this is a prefix command, a typing broadcast is started
/// until the return value is dropped.
pub trait DeferEphemeralOrBroadcast {
    // TODO: remove eventually?
    #[allow(async_fn_in_trait)] // i'm only using this in my code
    async fn defer_ephemeral_or_broadcast(&self) -> Result;
}

impl DeferEphemeralOrBroadcast for Context<'_> {
    /// If this is an application command,
    /// `poise::structs::context::ApplicationContext::defer_ephemeral` is called.
    ///
    /// If this is a prefix command, a typing broadcast is started
    /// until the return value is dropped.
    async fn defer_ephemeral_or_broadcast(&self) -> Result {
        match self {
            Context::Application(_) => self.defer_ephemeral().await,
            Context::Prefix(_) => self.defer_or_broadcast().await.map(|_| ()),
        }
        .context(DeferSnafu)
    }
}

pub trait DeleteInvokingMessageIfPrefix {
    // TODO: remove eventually?
    #[allow(async_fn_in_trait)] // i'm only using this in my code
    async fn delete_invoking_message_if_prefix(&self) -> Result;
}

impl DeleteInvokingMessageIfPrefix for Context<'_> {
    async fn delete_invoking_message_if_prefix(&self) -> Result {
        if let Context::Prefix(ctx) = self {
            ctx.msg.delete(ctx).await.context(DeleteMessageSnafu)?;
        }
        Ok(())
    }
}

pub trait DeleteResponse {
    async fn delete_response(&self, interaction: ComponentInteraction) -> Result;
}

impl DeleteResponse for Context<'_> {
    async fn delete_response(&self, interaction: ComponentInteraction) -> Result {
        interaction
            .delete_response(self)
            .await
            .context(DeleteResponseSnafu)
    }
}

pub trait DeleteSelfAndInvokingMessageIfPrefix {
    // TODO: remove eventually?
    #[allow(async_fn_in_trait)] // i'm only using this in my code
    async fn delete_self_and_invoking_message_if_prefix(&self, ctx: Context<'_>) -> Result;
}

impl DeleteSelfAndInvokingMessageIfPrefix for ReplyHandle<'_> {
    async fn delete_self_and_invoking_message_if_prefix(&self, ctx: Context<'_>) -> Result {
        self.delete(ctx).await.context(DeleteMessageSnafu)?;
        if let Context::Prefix(ctx) = ctx {
            ctx.msg.delete(ctx).await.context(DeleteMessageSnafu)?;
        }
        Ok(())
    }
}

pub trait EditWith {
    async fn edit_with(&self, ctx: Context<'_>, content: impl AsRef<str>) -> Result;
}

impl EditWith for ReplyHandle<'_> {
    async fn edit_with(&self, ctx: Context<'_>, content: impl AsRef<str>) -> Result {
        self.edit(
            ctx,
            CreateReply::default()
                .content(content.as_ref())
                .components(vec![]),
        )
        .await
        .context(EditMessageSnafu)
    }
}

pub trait RespondToWith {
    // TODO: remove eventually?
    #[allow(async_fn_in_trait)] // i'm only using this in my code
    async fn respond_to_with(
        &self,
        interaction: &ComponentInteraction,
        message: impl AsRef<str>,
    ) -> Result;
}

impl RespondToWith for Context<'_> {
    async fn respond_to_with(
        &self,
        interaction: &ComponentInteraction,
        message: impl AsRef<str>,
    ) -> Result {
        interaction
            .create_response(
                self,
                CreateInteractionResponse::UpdateMessage(
                    CreateInteractionResponseMessage::new()
                        .content(message.as_ref())
                        .embeds(vec![])
                        .components(vec![]),
                ),
            )
            .await
            .context(SendResponseSnafu)
    }
}

pub trait SayWith {
    async fn say_with(&self, message: impl AsRef<str>) -> Result<ReplyHandle<'_>>;
}

impl SayWith for Context<'_> {
    async fn say_with(&self, message: impl AsRef<str>) -> Result<ReplyHandle<'_>> {
        self.say(message.as_ref()).await.context(SendMessageSnafu)
    }
}

pub trait ShowModal<M: Modal> {
    async fn show_modal(&self, interaction: ComponentInteraction) -> Result<Option<M>>;
}

impl<M: Modal> ShowModal<M> for poise::serenity_prelude::Context {
    async fn show_modal(&self, interaction: ComponentInteraction) -> Result<Option<M>> {
        execute_modal_on_component_interaction::<M>(self, interaction, None, Some(TEN_MINUTES))
            .await
            .context(ShowModalSnafu)
    }
}
