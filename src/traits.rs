use crate::Context;
use miette::Diagnostic;
use poise::{
    CreateReply, Modal, ReplyHandle, execute_modal_on_component_interaction,
    serenity_prelude::{
        ComponentInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
    },
};
use snafu::{ResultExt, Snafu};

pub trait EditWith {
    #[expect(async_fn_in_trait)] // i'm only using this in my code
    async fn edit_with(
        &self,
        ctx: Context<'_>,
        content: impl AsRef<str>,
    ) -> Result<(), serenity::Error>;
}

impl EditWith for ReplyHandle<'_> {
    async fn edit_with(
        &self,
        ctx: Context<'_>,
        content: impl AsRef<str>,
    ) -> Result<(), serenity::Error> {
        self.edit(
            ctx,
            CreateReply::default()
                .content(content.as_ref())
                .components(vec![]),
        )
        .await
    }
}

pub trait RespondToWith {
    #[expect(async_fn_in_trait)] // i'm only using this in my code
    async fn respond_to_with(
        &self,
        interaction: &ComponentInteraction,
        message: impl AsRef<str>,
    ) -> Result<(), serenity::Error>;
}

impl RespondToWith for Context<'_> {
    async fn respond_to_with(
        &self,
        interaction: &ComponentInteraction,
        message: impl AsRef<str>,
    ) -> Result<(), serenity::Error> {
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
    }
}

#[derive(Debug, Snafu, Diagnostic)]
#[snafu(display("Kunde inte visa modal: {source}"))]
#[snafu(visibility(pub))]
#[diagnostic(
    code(traits::show_modal),
    help("Försök igen eller starta om interaktionen")
)]
pub struct ShowModalError {
    #[snafu(source)]
    pub source: serenity::Error,
}

impl From<serenity::Error> for ShowModalError {
    fn from(source: serenity::Error) -> Self {
        ShowModalError { source }
    }
}

pub trait ShowModal<M: Modal> {
    #[expect(async_fn_in_trait)] // i'm only using this in my code
    async fn show_modal(
        &self,
        interaction: ComponentInteraction,
    ) -> Result<Option<M>, ShowModalError>;
}

impl<M: Modal> ShowModal<M> for poise::serenity_prelude::Context {
    async fn show_modal(
        &self,
        interaction: ComponentInteraction,
    ) -> Result<Option<M>, ShowModalError> {
        execute_modal_on_component_interaction::<M>(self, interaction, None, None)
            .await
            .context(ShowModalSnafu)
    }
}
