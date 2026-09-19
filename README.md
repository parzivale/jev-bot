# jev-bot

A Discord bot that asks the TypeSafe **jev** model how likely a statement is to
be true, and answers with a calibrated probability.

```
Almost certainly false
> The Eiffel Tower is in Berlin.

Probability true  █░░░░░░░░░░░░░░░░░░░  4.0%
jev-1.13.0 · a value near 50% means jev can't separate the two
```

## Two ways to use it

| | |
| --- | --- |
| `/truth <statement>` | Check a statement you type. `private: true` keeps the result to yourself. |
| Right-click a message → **Apps** → **Fact check** | Check something already posted. Desktop: right-click. Mobile: long-press. |

Both answer in the channel. Fact check credits the checked message's author in
the embed header, but posts as a plain interaction response — it does not reply
to or ping them.

## Reading the number

One `POST /v1/systemone` per invocation, with the text as the `state` and a single
[`noul`](https://docs.typesafe.ai/primitives/noul) question. A noul returns one
0–1 probability that already encodes the model's certainty; there is no separate
confidence field.

**Near 50% means jev cannot separate true from false — not "half true".** noul is
binary, so it will not grade a partly-correct claim, and it has nothing useful to
say about statements that aren't checkable at all. If you want a gradient, the
`score` primitive is the right tool.

TypeSafe ships Python and JavaScript SDKs but no Rust one, so `src/typesafe.rs`
speaks the HTTP API directly. The request and response shapes are pinned by tests.

## Setup

Rust comes from the flake:

```sh
nix develop
cp .env.example .env
```

| Variable | | |
| --- | --- | --- |
| `DISCORD_TOKEN` | required | Your app → Bot → Reset Token |
| `TYPESAFE_API_KEY` | required | [docs.typesafe.ai](https://docs.typesafe.ai) |
| `DISCORD_GUILD_ID` | optional | A server id registers commands there instantly; blank registers globally, which takes up to an hour to propagate |
| `TYPESAFE_MODEL` | optional | Defaults to `jev-latest` |
| `TYPESAFE_BASE_URL` | optional | Defaults to `https://api.typesafe.ai`; mainly for tests |

Blank optional values are treated as unset, so a freshly copied `.env.example`
works with only the two required fields filled in.

## Inviting it

```
https://discord.com/oauth2/authorize?client_id=<APPLICATION_ID>&scope=bot+applications.commands&permissions=0
```

`permissions=0` is enough: both commands answer as interaction responses, which
need no channel permissions.

## Run

```sh
cargo run --release
```

Commands register themselves on connect — there is no separate deploy step.

## Test

```sh
cargo test                        # 19 tests, no network
cargo test -- --ignored --nocapture   # 3 live tests, needs a real API key
```

The offline client tests run against a scripted local HTTP server that records
what was sent, covering the request shape, the response shape, auth header and
path, status→error mapping, malformed responses, and that 429/529 retry with
backoff while 422 does not.

## Layout

| | |
| --- | --- |
| `src/main.rs` | Framework setup, both commands |
| `src/typesafe.rs` | System One client, wire types, retries, errors |
| `src/format.rs` | Verdict labels, bar, colour, quoting, embed |

## Notes

- `/truth` caps input at 1000 characters; jev accepts far more (32k tokens for
  state plus question). Fact check reads whatever the message contains, and
  truncates only what it quotes back.
- Input costs $0.042/M tokens and output is free, so a check runs roughly
  350 tokens — about $0.000015. Each call logs its usage.
- Consider who can invoke Fact check. Pointed at a personal statement it will
  publicly score something unverifiable, and a 50% result reads as an accusation
  while actually meaning "no information".
