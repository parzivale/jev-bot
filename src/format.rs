//! Turning a 0–1 probability into something readable in Discord.

use poise::serenity_prelude as serenity;

const BAR_WIDTH: usize = 20;

/// Message text is quoted back in the embed, which Discord caps at 4096 chars.
const MAX_QUOTED: usize = 900;

/// Ordered high → low; the first threshold the probability clears wins.
const VERDICTS: &[(f64, &str)] = &[
    (0.95, "Almost certainly true"),
    (0.80, "Likely true"),
    (0.60, "Leaning true"),
    (0.40, "Toss-up"),
    (0.20, "Leaning false"),
    (0.05, "Likely false"),
    (0.00, "Almost certainly false"),
];

pub fn verdict(p: f64) -> &'static str {
    VERDICTS
        .iter()
        .find(|(floor, _)| p >= *floor)
        .map(|(_, label)| *label)
        .unwrap_or("Almost certainly false")
}

pub fn bar(p: f64) -> String {
    let filled = (p.clamp(0.0, 1.0) * BAR_WIDTH as f64).round() as usize;
    "█".repeat(filled) + &"░".repeat(BAR_WIDTH - filled)
}

/// Red at 0, amber at 0.5, green at 1.
pub fn colour(p: f64) -> serenity::Colour {
    let p = p.clamp(0.0, 1.0);
    let (r, g) = if p < 0.5 {
        (255.0, p * 2.0 * 200.0)
    } else {
        ((1.0 - p) * 2.0 * 255.0, 200.0)
    };
    serenity::Colour::from_rgb(r.round() as u8, g.round() as u8, 0)
}

pub fn percent(p: f64) -> String {
    format!("{:.1}%", p * 100.0)
}

/// Truncate on a character boundary, never mid-codepoint.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{}…", kept.trim_end())
}

/// Blockquote every line, so a multi-line message stays visually one quote.
fn quote(text: &str) -> String {
    truncate(text, MAX_QUOTED)
        .lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The verdict embed. `author` credits whoever wrote the checked message.
pub fn truth_embed(
    statement: &str,
    probability: f64,
    model: &str,
    author: Option<&serenity::User>,
) -> serenity::CreateEmbed {
    let embed = serenity::CreateEmbed::new()
        .colour(colour(probability))
        .title(verdict(probability))
        .description(quote(statement))
        .field(
            "Probability true",
            format!("`{}` **{}**", bar(probability), percent(probability)),
            false,
        )
        .footer(serenity::CreateEmbedFooter::new(format!(
            "{model} · a value near 50% means jev can't separate the two"
        )));

    match author {
        Some(user) => {
            embed.author(serenity::CreateEmbedAuthor::new(user.tag()).icon_url(user.face()))
        }
        None => embed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_covers_the_whole_range() {
        assert_eq!(verdict(1.0), "Almost certainly true");
        assert_eq!(verdict(0.97), "Almost certainly true");
        assert_eq!(verdict(0.87), "Likely true");
        assert_eq!(verdict(0.72), "Leaning true");
        assert_eq!(verdict(0.50), "Toss-up");
        assert_eq!(verdict(0.30), "Leaning false");
        assert_eq!(verdict(0.15), "Likely false");
        assert_eq!(verdict(0.0), "Almost certainly false");
    }

    #[test]
    fn bar_is_always_full_width() {
        for step in 0..=100 {
            let p = step as f64 / 100.0;
            assert_eq!(bar(p).chars().count(), BAR_WIDTH, "at p={p}");
        }
    }

    #[test]
    fn bar_ends_are_empty_and_full() {
        assert_eq!(bar(0.0), "░".repeat(BAR_WIDTH));
        assert_eq!(bar(1.0), "█".repeat(BAR_WIDTH));
    }

    #[test]
    fn colour_runs_red_to_green() {
        assert_eq!(colour(0.0).tuple(), (255, 0, 0));
        assert_eq!(colour(1.0).tuple(), (0, 200, 0));
        let mid = colour(0.5).tuple();
        assert!(mid.0 > 200 && mid.1 > 150, "amber-ish at 0.5, got {mid:?}");
    }

    /// Out-of-range input would otherwise panic in `repeat`.
    #[test]
    fn out_of_range_probabilities_are_clamped() {
        assert_eq!(bar(1.5).chars().count(), BAR_WIDTH);
        assert_eq!(bar(-0.5).chars().count(), BAR_WIDTH);
    }

    #[test]
    fn percent_has_one_decimal() {
        assert_eq!(percent(0.931), "93.1%");
        assert_eq!(percent(0.0), "0.0%");
    }

    #[test]
    fn quote_prefixes_every_line() {
        assert_eq!(quote("one\ntwo"), "> one\n> two");
        assert_eq!(quote("solo"), "> solo");
    }

    #[test]
    fn long_messages_are_truncated() {
        let long = "a".repeat(MAX_QUOTED * 2);
        let quoted = quote(&long);
        assert!(quoted.ends_with('…'));
        assert!(
            quoted.chars().count() <= MAX_QUOTED + 4,
            "got {}",
            quoted.chars().count()
        );
    }

    /// Truncating a multi-byte string mid-codepoint would panic on a slice.
    #[test]
    fn truncation_is_codepoint_safe() {
        let emoji = "🎉".repeat(MAX_QUOTED * 2);
        let quoted = quote(&emoji);
        assert!(quoted.ends_with('…'));
        let accented = "é".repeat(MAX_QUOTED + 10);
        assert!(quote(&accented).ends_with('…'));
    }

    #[test]
    fn short_text_is_untouched() {
        assert_eq!(truncate("hello", MAX_QUOTED), "hello");
        assert_eq!(truncate("🎉🎉", 5), "🎉🎉");
    }
}
