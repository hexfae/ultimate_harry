//! Custom traits for extending poise contexts and interactions.

use alloc::borrow::Cow;
use poise::{ApplicationContext, Context, CreateReply, ReplyHandle};

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
