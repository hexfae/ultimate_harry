//! Custom traits for extending poise contexts and interactions.

use alloc::borrow::Cow;
use poise::{
    ApplicationContext, Context, CreateReply, Modal, ReplyHandle,
    execute_modal_on_component_interaction,
    serenity_prelude::{
        ComponentInteraction, Context as SerenityContext, CreateInteractionResponse,
        CreateInteractionResponseMessage,
    },
};

/// A trait for sending ephemeral messages that only the user can see.
pub trait SayEphemeral {
    /// Send a string as an ephemeral reply.
    async fn say_ephemeral(
        &self,
        content: impl Into<Cow<'_, str>>,
    ) -> Result<ReplyHandle<'_>, serenity::Error>;
}

impl<U: Send + Sync + 'static, E> SayEphemeral for ApplicationContext<'_, U, E> {
    async fn say_ephemeral(
        &self,
        content: impl Into<Cow<'_, str>>,
    ) -> Result<ReplyHandle<'_>, serenity::Error> {
        self.send(CreateReply::new().content(content).ephemeral(true))
            .await
    }
}

impl<U: Send + Sync + 'static, E> SayEphemeral for Context<'_, U, E> {
    async fn say_ephemeral(
        &self,
        content: impl Into<Cow<'_, str>>,
    ) -> Result<ReplyHandle<'_>, serenity::Error> {
        self.send(CreateReply::new().content(content).ephemeral(true))
            .await
    }
}

/// A trait for responding to an interaction with a message.
pub trait RespondToWith {
    /// Respond to the given interaction with the given text, by updating the
    /// original message, additionally clearing its embeds and components.
    async fn respond_to_with(
        &self,
        interaction: &ComponentInteraction,
        message: impl AsRef<str>,
    ) -> Result<(), serenity::Error>;
}

impl<U: Send + Sync + 'static, E> RespondToWith for Context<'_, U, E> {
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

/// Convenience function for showing a modal with no timeout.
pub trait ShowModal<M: Modal> {
    /// Show a modal pre-filled with the given defaults, with no timeout.
    async fn show_modal_with_defaults(
        &self,
        interaction: ComponentInteraction,
        defaults: M,
    ) -> Result<Option<M>, serenity::Error>;
}

impl<M: Modal> ShowModal<M> for SerenityContext {
    async fn show_modal_with_defaults(
        &self,
        interaction: ComponentInteraction,
        defaults: M,
    ) -> Result<Option<M>, serenity::Error> {
        execute_modal_on_component_interaction::<M>(self, interaction, Some(defaults), None).await
    }
}
