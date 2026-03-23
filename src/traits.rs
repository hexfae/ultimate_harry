use crate::{
    Context, DeleteResponseSnafu, EditMessageSnafu, SendMessageSnafu, SendResponseSnafu,
    ShowModalSnafu,
};
use poise::{
    CreateReply, Modal, ReplyHandle, execute_modal_on_component_interaction,
    serenity_prelude::{
        ComponentInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
    },
};
use snafu::ResultExt;

type Result<T = ()> = std::result::Result<T, crate::Error>;

pub trait DeleteResponse {
    #[expect(async_fn_in_trait)] // i'm only using this in my code
    async fn delete_response(&self, interaction: ComponentInteraction) -> Result;
}

impl DeleteResponse for Context<'_> {
    async fn delete_response(&self, interaction: ComponentInteraction) -> Result {
        interaction
            .delete_response(self.http())
            .await
            .context(DeleteResponseSnafu)
    }
}

pub trait EditWith {
    #[expect(async_fn_in_trait)] // i'm only using this in my code
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
    #[expect(async_fn_in_trait)] // i'm only using this in my code
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
                self.http(),
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
    #[expect(async_fn_in_trait)] // i'm only using this in my code
    async fn say_with(&self, message: impl AsRef<str>) -> Result<ReplyHandle<'_>>;
}

impl SayWith for Context<'_> {
    async fn say_with(&self, message: impl AsRef<str>) -> Result<ReplyHandle<'_>> {
        self.say(message.as_ref()).await.context(SendMessageSnafu)
    }
}

pub trait ShowModal<M: Modal> {
    #[expect(async_fn_in_trait)] // i'm only using this in my code
    async fn show_modal(&self, interaction: ComponentInteraction) -> Result<Option<M>>;
}

impl<M: Modal> ShowModal<M> for poise::serenity_prelude::Context {
    async fn show_modal(&self, interaction: ComponentInteraction) -> Result<Option<M>> {
        execute_modal_on_component_interaction::<M>(self, interaction, None, None)
            .await
            .context(ShowModalSnafu)
    }
}
