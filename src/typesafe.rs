//! Client for the TypeSafe System One API.
//!
//! There's no official Rust SDK, so this speaks the HTTP API directly:
//! `POST /v1/systemone` with the statement as `state` and one `noul` question.
//! See <https://docs.typesafe.ai/primitives/noul>.

use std::time::Duration;

use serde::{Deserialize, Serialize};

const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
const DEFAULT_MODEL: &str = "jev-latest";
const TIMEOUT: Duration = Duration::from_secs(20);

/// Retries for the two statuses the docs say to back off on (429, 529).
const MAX_ATTEMPTS: u32 = 3;
const BACKOFF_BASE: Duration = Duration::from_millis(500);

// The docs suggest phrasing a noul so that higher always means "yes", and
// spelling out the boundary when it's subtle. Here "yes" == the statement holds.
const INSTRUCTIONS: &str = "Is the statement in the state true as a matter of fact?";
const CRITERIA_TRUE: &str = "The statement asserts something that is accurate and verifiable, \
     or overwhelmingly supported by what is generally known.";
const CRITERIA_FALSE: &str = "The statement asserts something inaccurate, contradicted by what \
     is generally known, or fabricated.";

/// Errors are phrased for display in Discord; the full detail goes to the log.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("TypeSafe rejected the API key. Check `TYPESAFE_API_KEY`.")]
    Auth,
    #[error("TypeSafe couldn't process that request.")]
    Unprocessable,
    #[error("Rate limited by TypeSafe. Try again shortly.")]
    RateLimited,
    #[error("TypeSafe is overloaded right now. Try again shortly.")]
    Overloaded,
    #[error("TypeSafe took too long to answer.")]
    Timeout,
    #[error("Couldn't reach TypeSafe.")]
    Connection,
    #[error("TypeSafe returned an unexpected response.")]
    Malformed,
    #[error("TypeSafe returned HTTP {0}.")]
    Unexpected(u16),
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            Error::Timeout
        } else if err.is_decode() {
            Error::Malformed
        } else {
            Error::Connection
        }
    }
}

// --- wire types -------------------------------------------------------------
// Keying `questions`/`answers` by a named field rather than a map means the
// request and the response agree on the question id at compile time.

#[derive(Serialize)]
struct Request<'a> {
    state: &'a str,
    model: &'a str,
    questions: Questions,
}

#[derive(Serialize)]
struct Questions {
    is_true: NoulQuestion,
}

#[derive(Serialize)]
struct NoulQuestion {
    #[serde(rename = "type")]
    kind: &'static str,
    instructions: &'static str,
    criteria: Criteria,
}

#[derive(Serialize)]
struct Criteria {
    #[serde(rename = "true")]
    yes: &'static str,
    #[serde(rename = "false")]
    no: &'static str,
}

#[derive(Deserialize)]
struct Response {
    model: String,
    answers: Answers,
    usage: Usage,
}

#[derive(Deserialize)]
struct Answers {
    is_true: NoulAnswer,
}

#[derive(Deserialize)]
struct NoulAnswer {
    noul: f64,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// What jev thinks of a statement.
#[derive(Debug)]
pub struct Verdict {
    /// 0–1. Near 0.5 means jev genuinely can't separate true from false.
    pub probability: f64,
    pub model: String,
    pub usage: Usage,
}

/// The System One endpoint for a base URL, tolerating a trailing slash.
fn endpoint(base_url: &str) -> String {
    format!("{}/v1/systemone", base_url.trim_end_matches('/'))
}

pub struct Client {
    http: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl Client {
    /// Reads `TYPESAFE_API_KEY`, and `TYPESAFE_MODEL` / `TYPESAFE_BASE_URL` if set.
    pub fn from_env() -> Result<Self, String> {
        let api_key = crate::secret("TYPESAFE_API_KEY")?.ok_or_else(|| {
            "TYPESAFE_API_KEY or TYPESAFE_API_KEY_FILE must be set (see .env.example).".to_string()
        })?;

        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| format!("could not build HTTP client: {e}"))?;

        Ok(Self {
            http,
            api_key,
            model: crate::env_opt("TYPESAFE_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into()),
            base_url: crate::env_opt("TYPESAFE_BASE_URL")
                .unwrap_or_else(|| DEFAULT_BASE_URL.into()),
        })
    }

    /// Ask jev how likely `statement` is to be true.
    pub async fn score(&self, statement: &str) -> Result<Verdict, Error> {
        let body = Request {
            state: statement,
            model: &self.model,
            questions: Questions {
                is_true: NoulQuestion {
                    kind: "noul",
                    instructions: INSTRUCTIONS,
                    criteria: Criteria {
                        yes: CRITERIA_TRUE,
                        no: CRITERIA_FALSE,
                    },
                },
            },
        };

        let url = endpoint(&self.base_url);

        for attempt in 0..MAX_ATTEMPTS {
            let response = self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await?;

            let status = response.status();
            if status.is_success() {
                let parsed: Response = response.json().await?;
                return Ok(Verdict {
                    probability: parsed.answers.is_true.noul,
                    model: parsed.model,
                    usage: parsed.usage,
                });
            }

            let err = match status.as_u16() {
                401 | 403 => Error::Auth,
                422 => Error::Unprocessable,
                429 => Error::RateLimited,
                529 => Error::Overloaded,
                other => Error::Unexpected(other),
            };

            // Only 429 and 529 are worth retrying; everything else fails now.
            let retryable = matches!(err, Error::RateLimited | Error::Overloaded);
            if !retryable || attempt + 1 == MAX_ATTEMPTS {
                return Err(err);
            }

            tokio::time::sleep(BACKOFF_BASE * 2u32.pow(attempt)).await;
        }

        unreachable!("loop returns on the final attempt")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const OK_BODY: &str = r#"{"model":"jev-1.13.0","answers":{"is_true":{"type":"noul","noul":0.87}},"usage":{"input_tokens":312,"output_tokens":48}}"#;

    fn headers_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
    }

    fn content_length(head: &[u8]) -> usize {
        String::from_utf8_lossy(head)
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())?
            })
            .unwrap_or(0)
    }

    /// One recorded request: the start line, the headers, and the body.
    struct Recorded {
        head: String,
        body: String,
    }

    /// Serves a scripted sequence of responses, recording each request.
    async fn serve(responses: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<Recorded>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);

        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut sock, _) = listener.accept().await.unwrap();

                let mut buf = Vec::new();
                loop {
                    let mut chunk = [0u8; 2048];
                    let n = sock.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(start) = headers_end(&buf)
                        && buf.len() >= start + content_length(&buf[..start])
                    {
                        recorder.lock().unwrap().push(Recorded {
                            head: String::from_utf8_lossy(&buf[..start]).into_owned(),
                            body: String::from_utf8_lossy(&buf[start..]).into_owned(),
                        });
                        break;
                    }
                }

                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(response.as_bytes()).await.unwrap();
                sock.shutdown().await.ok();
            }
        });

        (format!("http://{addr}"), seen)
    }

    fn client(base_url: String) -> Client {
        Client {
            http: reqwest::Client::builder().timeout(TIMEOUT).build().unwrap(),
            api_key: "sk-test".into(),
            model: "jev-latest".into(),
            base_url,
        }
    }

    #[test]
    fn endpoint_tolerates_a_trailing_slash() {
        let want = "https://api.typesafe.ai/v1/systemone";
        assert_eq!(endpoint("https://api.typesafe.ai"), want);
        assert_eq!(endpoint("https://api.typesafe.ai/"), want);
        assert_eq!(endpoint("https://api.typesafe.ai///"), want);
    }

    #[tokio::test]
    async fn sends_the_documented_request_shape() {
        let (url, seen) = serve(vec![(200, OK_BODY)]).await;
        client(url)
            .score("The Eiffel Tower is in Paris.")
            .await
            .unwrap();

        let recorded = seen.lock().unwrap();
        let body: serde_json::Value =
            serde_json::from_str(&recorded[0].body).expect("body is JSON");

        assert_eq!(body["state"], "The Eiffel Tower is in Paris.");
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["questions"]["is_true"]["type"], "noul");
        assert!(body["questions"]["is_true"]["instructions"].is_string());
        assert!(body["questions"]["is_true"]["criteria"]["true"].is_string());
        assert!(body["questions"]["is_true"]["criteria"]["false"].is_string());
    }

    #[tokio::test]
    async fn sends_auth_and_path() {
        let (url, seen) = serve(vec![(200, OK_BODY)]).await;
        client(url).score("anything").await.unwrap();

        let recorded = seen.lock().unwrap();
        let head = &recorded[0].head;
        assert!(head.starts_with("POST /v1/systemone "), "got {head:?}");
        assert!(
            head.to_lowercase()
                .contains("authorization: bearer sk-test"),
            "missing bearer auth in {head:?}"
        );
    }

    #[tokio::test]
    async fn parses_the_documented_response_shape() {
        let (url, _) = serve(vec![(200, OK_BODY)]).await;
        let verdict = client(url).score("anything").await.unwrap();

        assert_eq!(verdict.probability, 0.87);
        assert_eq!(verdict.model, "jev-1.13.0");
        assert_eq!(verdict.usage.input_tokens, 312);
        assert_eq!(verdict.usage.output_tokens, 48);
    }

    #[tokio::test]
    async fn maps_auth_failure() {
        let (url, _) = serve(vec![(401, "{}")]).await;
        let err = client(url).score("anything").await.unwrap_err();
        assert!(matches!(err, Error::Auth), "got {err:?}");
    }

    #[tokio::test]
    async fn reports_malformed_responses() {
        let (url, _) = serve(vec![(200, r#"{"model":"jev-1.13.0"}"#)]).await;
        let err = client(url).score("anything").await.unwrap_err();
        assert!(matches!(err, Error::Malformed), "got {err:?}");
    }

    #[tokio::test]
    async fn does_not_retry_unprocessable() {
        let (url, seen) = serve(vec![(422, "{}")]).await;
        let err = client(url).score("anything").await.unwrap_err();

        assert!(matches!(err, Error::Unprocessable), "got {err:?}");
        assert_eq!(seen.lock().unwrap().len(), 1, "422 must not be retried");
    }

    #[tokio::test]
    async fn retries_overload_then_succeeds() {
        let (url, seen) = serve(vec![(529, "{}"), (200, OK_BODY)]).await;
        let verdict = client(url).score("anything").await.unwrap();

        assert_eq!(verdict.probability, 0.87);
        assert_eq!(seen.lock().unwrap().len(), 2, "should have retried once");
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts() {
        let (url, seen) = serve(vec![(429, "{}"), (429, "{}"), (429, "{}")]).await;
        let err = client(url).score("anything").await.unwrap_err();

        assert!(matches!(err, Error::RateLimited), "got {err:?}");
        assert_eq!(seen.lock().unwrap().len(), MAX_ATTEMPTS as usize);
    }
}

/// Opt-in checks against the real API. Need a working `.env`; run with:
///   cargo test -- --ignored --nocapture
#[cfg(test)]
mod live {
    use super::*;

    async fn score(statement: &str) -> Verdict {
        let _ = dotenvy::dotenv();
        Client::from_env()
            .expect("credentials")
            .score(statement)
            .await
            .expect("live call")
    }

    #[tokio::test]
    #[ignore = "hits the real API"]
    async fn scores_a_true_statement_high() {
        let v = score("The Eiffel Tower is in Paris.").await;
        println!("  true statement  -> {:.4} ({})", v.probability, v.model);
        assert!(v.probability > 0.8, "expected high, got {}", v.probability);
    }

    #[tokio::test]
    #[ignore = "hits the real API"]
    async fn scores_a_false_statement_low() {
        let v = score("The Eiffel Tower is in Berlin.").await;
        println!("  false statement -> {:.4} ({})", v.probability, v.model);
        assert!(v.probability < 0.2, "expected low, got {}", v.probability);
    }

    #[tokio::test]
    #[ignore = "hits the real API"]
    async fn reports_usage() {
        let v = score("Water boils at 100 degrees Celsius at sea level.").await;
        println!(
            "  usage -> {} in / {} out tokens",
            v.usage.input_tokens, v.usage.output_tokens
        );
        assert!(v.usage.input_tokens > 0);
    }
}
