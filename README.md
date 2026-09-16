# Ultimate Harry

Ulimate Harry is a Discord bot for talking with characters.

## Features

- Message "swiping"
- Message editing (with history and undo/redo per swipe)
- Message pinning (to a different channel, because of Discord's 50-pin channel
limit)
- Everything configured via slash commands
- Multiple users in a conversation
- Multiple characters in a conversation
- ElevenLabs V2/V3 TTS support (either a global default voice, a character
default voice, or from the dropdown, pick a specific voice or automatic (which
supports multi-voice!))
- Automatically adds ElevenLabs V3 audio tags (e.g. `[laughs]`, `[angry]`)
- User-installable TTS command, for doing TTS anywhere
- Text streaming (1s intervals, due to Discord's rate limits)
- Uses OpenRouter for model support
- Statistics, e.g. tokens generated, most used characters (globally/per user)
- Swedish user-facing text (sorry, non-speakers!)
- On models without vision/hearing, hand off to a capable model that describes
the image/audio, then passes the description back
- React with a configured emoji when a user is mentioned/replied to (special
request from a user, "feature" from the now-deleted Ryybot)
- Probably more that I'm forgetting…

## Building

The only officially supported method of building is with
[Nix](https://nixos.org/), however there aren't really any dependencies
(besides the Rust toolchain). Linking does use the
[mold](https://github.com/rui314/mold) linker by default, though, so either use
that or edit the [Cargo config](.cargo/config.toml).

## Running

The officially supported method of running is with the
[flake's NixOS module](flake.nix).

Set the `TOKEN_FILE` environment variable to point to a file containing (only)
the bot's Discord token. Besides that, everything is configured through slash
commands in a Discord server the bot is in. State is found in
`./harry_database/`.

Before you can chat with characters, you have to set a global `api-nyckel`
through `/modell`.

## Usage

Configure text model, OpenRouter API key, temperature, vision model, and audio
model globally with `/modell`. Configure model and temperature per character
with `/gubbe modell`. Configure ElevenLabs API key, global default voice,
global default ElevenLabs model, and V3 audio tag model with `/tal`.

Create, view, and delete voices with `/röst {skapa,visa,döda}` (create them on
the ElevenLabs website first). Set a character's default voice with `/gubbe
röst`.

Create, view, update, and delete characters with  `/gubbe
{skapa,visa,ändra,döda}`. Start a new chat with `/prata`. Restore deleted
characters with `/gubbe återuppliva`. Duplicate a character with `/gubbe klona`.
Set a character's color (the strip of color on the left of the embed) with
`/gubbe färg`. 

Read some text via an ElevenLabs TTS voice with `/säg`. View fun statistics
with `/statistik`. Set which name the bot prepends to your messages with `/namn`
(otherwise it uses `User`. Set which emoji the bot reacts with when you're
pinged/replied to using `emoji`. Set which channel pinned messages go into
using `/fästkanal`.

On a bot's message, the previous/next buttons let you "swipe" through different
responses (pressing next on the last response generates a new one). The
undo/redo buttons let you go through the edit history of a swipe. The edit
button lets you edit the bot's message. The pin button sends the current
version of the current swipe into the configured channel. The audio button
sends a TTS voice message using the character's default voice or the global
default voice. The continue button makes the character continue writing its
message. While streaming the response, the continue button is instead a stop
button, which interrupts the bot (e.g. if it's stuck, is repeating itself). The
"Svara som…"" dropdown lets you pick a character to respond to the character's
message. The "Läs upp som…" dropdown lets you pick a custom voice for TTS, or
choose automatic to have it automatically pick one (or multiple, if applicable).

## Security

On the Discord side, there is no form of authentication or permission checks,
so everyone can run every command, and can view all settings (besides API keys,
which are redacted), EXCEPT users must share a server with the bot to run
`/säg`.

On the server side, everything is stored in plain text (however not
world-readable, i.e. `600`). Characters, chats, configuration, and users are
stored in `.json`s under
`./harry_database/{characters,chats,configuration,users}`.

## License

`AGPL-3.0-or-later`

## What's with the name?

- Ultimate: This is like the 6th rewrite of this concept, the original being
over 3 years old (2023-06-01).
- Harry: One of our first characters was a daycare worker, who was told to act
condescendingly and to infantalize the user, and generally treat them like a
dumb baby. On the fifth day of the first version, instead of writing a normal
response, it suddenly started its own long roleplay, writing as both the user
and as "Harry" in its message. After asking it 4 times who Harry was, it
introduced itself. We didn't have a (good) name for either the character or the
bot itself, so we started calling both Harry. So yes, it was a hallucination.

## Is it any good?

Yes.

## Contributing

Please contribute
