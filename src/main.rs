mod format;
mod typesafe;

use poise::serenity_prelude as serenity;

/// Treats an unset variable and a blank one the same: copying `.env.example`
/// leaves optional keys present but empty.
pub(crate) fn env_opt(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Resolve a secret from an inline value or a file path.
///
/// Split out from [`secret`] so it can be tested without mutating the process
/// environment, which races across parallel tests.
fn read_secret(
    name: &str,
    inline: Option<String>,
    path: Option<String>,
) -> Result<Option<String>, String> {
    let Some(path) = path else {
        return Ok(inline);
    };

    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read {name}_FILE at {path}: {e}"))?;

    // Trailing newlines are near-universal in secret files; a literal one in a
    // bearer token produces a baffling 401.
    match contents.trim() {
        "" => Err(format!("{name}_FILE at {path} is empty")),
        value => Ok(Some(value.to_string())),
    }
}

/// A secret from `<NAME>_FILE` if that is set, otherwise from `<NAME>`.
///
/// The file form keeps secrets out of the process environment, and out of the
/// Nix store: a value supplied at build time would be world-readable there.
pub(crate) fn secret(name: &str) -> Result<Option<String>, String> {
    read_secret(name, env_opt(name), env_opt(&format!("{name}_FILE")))
}

struct Data {
    typesafe: typesafe::Client,
}

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Ask the jev model how likely a statement is to be true.
#[poise::command(slash_command)]
async fn truth(
    ctx: Context<'_>,
    #[description = "The statement to evaluate"]
    #[max_length = 1000]
    statement: String,
    #[description = "Only show the result to you (default: false)"] private: Option<bool>,
) -> Result<(), Error> {
    let ephemeral = private.unwrap_or(false);

    // jev usually answers well inside Discord's 3s window, but defer so a slow
    // call or a retry can't kill the interaction.
    ctx.defer_or_broadcast().await?;

    match score(&ctx, &statement).await {
        Ok(verdict) => {
            let embed = format::truth_embed(&statement, verdict.probability, &verdict.model, None);
            ctx.send(
                poise::CreateReply::default()
                    .embed(embed)
                    .ephemeral(ephemeral),
            )
            .await?;
        }
        Err(message) => {
            ctx.send(
                poise::CreateReply::default()
                    .content(message)
                    .ephemeral(true),
            )
            .await?;
        }
    }

    Ok(())
}

/// Right-click a message → Apps → Fact check.
///
/// The verdict is the interaction response itself: one public message, no
/// separate reply and no ping aimed at the author. The embed credits them in
/// its header instead.
#[poise::command(context_menu_command = "Fact check")]
async fn fact_check(ctx: Context<'_>, message: serenity::Message) -> Result<(), Error> {
    let statement = message.content.trim().to_string();

    // Answered before deferring: this needs no API call, and stays private.
    if statement.is_empty() {
        ctx.send(
            poise::CreateReply::default()
                .content("That message has no text to check — attachments and embeds aren't read.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    ctx.defer().await?;

    match score(&ctx, &statement).await {
        Ok(verdict) => {
            let embed = format::truth_embed(
                &statement,
                verdict.probability,
                &verdict.model,
                Some(&message.author),
            );
            ctx.send(poise::CreateReply::default().embed(embed)).await?;
        }
        Err(text) => {
            ctx.send(poise::CreateReply::default().content(text))
                .await?;
        }
    }

    Ok(())
}

/// Score a statement, logging it and mapping any failure to a user-facing line.
async fn score(ctx: &Context<'_>, statement: &str) -> Result<typesafe::Verdict, String> {
    match ctx.data().typesafe.score(statement).await {
        Ok(verdict) => {
            println!(
                "scored {:.3} ({} in / {} out tokens): {statement:?}",
                verdict.probability, verdict.usage.input_tokens, verdict.usage.output_tokens
            );
            Ok(verdict)
        }
        Err(err) => {
            eprintln!("scoring {statement:?} failed: {err:?}");
            Err(format!("⚠️ {err}"))
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    // Absent .env is fine — the vars may come from the environment directly.
    let _ = dotenvy::dotenv();

    let token = secret("DISCORD_TOKEN")?.ok_or_else(|| {
        "DISCORD_TOKEN or DISCORD_TOKEN_FILE must be set (see .env.example).".to_string()
    })?;
    let typesafe = typesafe::Client::from_env()?;

    // Guild-scoped registration shows up immediately; global takes up to an hour.
    let guild_id = match env_opt("DISCORD_GUILD_ID") {
        Some(raw) => Some(
            raw.parse::<u64>()
                .map_err(|_| format!("DISCORD_GUILD_ID is not a valid id: {raw:?}"))?,
        ),
        None => None,
    };

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![truth(), fact_check()],
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                let commands = &framework.options().commands;
                match guild_id {
                    Some(id) => {
                        poise::builtins::register_in_guild(
                            ctx,
                            commands,
                            serenity::GuildId::new(id),
                        )
                        .await?;
                        println!("Registered /truth and Fact check in guild {id}.");
                    }
                    None => {
                        poise::builtins::register_globally(ctx, commands).await?;
                        println!(
                            "Registered /truth and Fact check globally \
                             (may take up to an hour to appear)."
                        );
                    }
                }
                println!("Listening as {}.", ready.user.tag());
                Ok(Data { typesafe })
            })
        })
        .build();

    // Slash and context-menu commands need no privileged intents.
    let mut client =
        serenity::ClientBuilder::new(token, serenity::GatewayIntents::non_privileged())
            .framework(framework)
            .await
            .map_err(|e| format!("could not connect to Discord: {e}"))?;

    client
        .start()
        .await
        .map_err(|e| format!("bot stopped: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique scratch path per test; no two tests share a file.
    fn scratch(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("jev-bot-{}-{tag}", std::process::id()))
    }

    fn write(tag: &str, contents: &str) -> String {
        let path = scratch(tag);
        std::fs::write(&path, contents).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn falls_back_to_the_inline_value() {
        let got = read_secret("TOK", Some("inline".into()), None).unwrap();
        assert_eq!(got, Some("inline".into()));
    }

    #[test]
    fn absent_everywhere_is_none() {
        assert_eq!(read_secret("TOK", None, None).unwrap(), None);
    }

    #[test]
    fn a_file_path_wins_over_the_inline_value() {
        let path = write("wins", "from-file");
        let got = read_secret("TOK", Some("inline".into()), Some(path)).unwrap();
        assert_eq!(got, Some("from-file".into()));
    }

    /// Secret files almost always end in a newline; one inside a bearer token
    /// produces a 401 that is very hard to read.
    #[test]
    fn trailing_whitespace_is_stripped() {
        let path = write("trailing", "  secret-value\n\n");
        let got = read_secret("TOK", None, Some(path)).unwrap();
        assert_eq!(got, Some("secret-value".into()));
    }

    #[test]
    fn an_empty_file_is_an_error() {
        let path = write("empty", "   \n");
        let err = read_secret("TOK", None, Some(path.clone())).unwrap_err();
        assert!(err.contains("TOK_FILE"), "got {err}");
        assert!(err.contains(&path), "error should name the path, got {err}");
    }

    #[test]
    fn a_missing_file_is_an_error_naming_the_path() {
        let missing = scratch("nope").to_string_lossy().into_owned();
        let err = read_secret("TOK", Some("inline".into()), Some(missing.clone())).unwrap_err();
        assert!(err.contains(&missing), "got {err}");
        assert!(err.contains("TOK_FILE"), "got {err}");
    }
}
