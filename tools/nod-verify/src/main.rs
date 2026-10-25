//! `nod-verify` — verify a Nod decision receipt offline against pinned keys.
//!
//! Exit status is the contract: 0 only on a full pass, 1 when the receipt does
//! not verify (or an `--expect-*` check fails), 2 when the inputs could not be
//! read or understood. Stdout carries exactly one JSON line either way.

use std::{fs, path::PathBuf, process::ExitCode};

use anyhow::{anyhow, bail, Context};
use chrono::{DateTime, Duration, Utc};
use nod_proto::{
    receipt::{PinnedKey, ReceiptVerificationInput, VerifiedReceipt},
    verify_decision_receipt, Decision, Request,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const USAGE: &str = "usage: nod-verify --request created.json --decision decision.json --keys keys.json \
[--expect-user USER] [--expect-option OPTION] [--strict-time] [--max-clock-skew SECONDS] [--now RFC3339]

  --request   the create response (`{\"request_id\",\"deduped\",\"request\":{...}}`) or a bare request
  --decision  the /decision or /wait response, the callback body, or a bare decision record
  --keys      `{\"keys\":[{\"user_id\":\"owner\",\"key_id\":\"...\",\"public_key\":\"...\"}]}`; values come
              from the admin devices list (`signing_key_id`, `signing_public_key`), recorded at enrollment
  --expect-user     require the verified actor to be this user id
  --expect-option   require the verified option to be this option id
  --strict-time     treat signed_at outside the clock-skew tolerance as a failure instead of a warning
  --max-clock-skew  tolerance in seconds (default 600)
  --now             verifier clock, RFC 3339 (default: system time)

exit 0 on a full pass, 1 on verification failure, 2 on input error";

const EXIT_PASS: u8 = 0;
const EXIT_FAIL: u8 = 1;
const EXIT_INPUT: u8 = 2;

#[derive(Debug, Default)]
struct Args {
    request: Option<PathBuf>,
    decision: Option<PathBuf>,
    keys: Option<PathBuf>,
    expect_user: Option<String>,
    expect_option: Option<String>,
    strict_time: bool,
    max_clock_skew: Option<i64>,
    now: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct KeysFile {
    keys: Vec<KeyEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyEntry {
    user_id: String,
    #[serde(default)]
    device_id: Option<String>,
    key_id: String,
    public_key: String,
}

#[derive(Debug, Serialize)]
struct PassReport<'a> {
    ok: bool,
    request_id: &'a str,
    user_id: &'a str,
    device_id: &'a str,
    key_id: &'a str,
    option_id: &'a str,
    option_kind: &'a str,
    request_digest: &'a str,
    digest_version: &'a str,
    signed_at: String,
    warnings: &'a [String],
}

#[derive(Debug, Serialize)]
struct FailReport<'a> {
    ok: bool,
    stage: &'a str,
    error: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

enum Outcome {
    Pass(Box<VerifiedReceipt>),
    Fail {
        error: String,
        warnings: Vec<String>,
    },
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => return input_error(error),
    };
    match run(&args) {
        Ok(Outcome::Pass(verified)) => {
            print_json(&PassReport {
                ok: true,
                request_id: &verified.request_id,
                user_id: &verified.user_id,
                device_id: &verified.device_id,
                key_id: &verified.key_id,
                option_id: &verified.option_id,
                option_kind: verified.option_kind.as_str(),
                request_digest: &verified.request_digest,
                digest_version: verified.digest_version.as_str(),
                signed_at: verified
                    .signed_at
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                warnings: &verified.warnings,
            });
            ExitCode::from(EXIT_PASS)
        }
        Ok(Outcome::Fail { error, warnings }) => {
            print_json(&FailReport {
                ok: false,
                stage: "verification",
                error,
                warnings,
            });
            ExitCode::from(EXIT_FAIL)
        }
        Err(error) => input_error(error),
    }
}

fn input_error(error: anyhow::Error) -> ExitCode {
    print_json(&FailReport {
        ok: false,
        stage: "input",
        error: format!("{error:#}"),
        warnings: Vec::new(),
    });
    eprintln!("{USAGE}");
    ExitCode::from(EXIT_INPUT)
}

fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string(value) {
        Ok(line) => println!("{line}"),
        Err(error) => eprintln!("nod-verify: could not encode report: {error}"),
    }
}

fn parse_args(raw: impl Iterator<Item = String>) -> anyhow::Result<Args> {
    let mut args = Args::default();
    let mut raw = raw.peekable();
    while let Some(flag) = raw.next() {
        let mut value = |name: &str| -> anyhow::Result<String> {
            raw.next().ok_or_else(|| anyhow!("{name} requires a value"))
        };
        match flag.as_str() {
            "--request" => args.request = Some(PathBuf::from(value("--request")?)),
            "--decision" => args.decision = Some(PathBuf::from(value("--decision")?)),
            "--keys" => args.keys = Some(PathBuf::from(value("--keys")?)),
            "--expect-user" => args.expect_user = Some(value("--expect-user")?),
            "--expect-option" => args.expect_option = Some(value("--expect-option")?),
            "--strict-time" => args.strict_time = true,
            "--max-clock-skew" => {
                let seconds = value("--max-clock-skew")?;
                let seconds: i64 = seconds.parse().with_context(|| {
                    format!("--max-clock-skew {seconds:?} is not a whole number of seconds")
                })?;
                if seconds < 0 {
                    bail!("--max-clock-skew must not be negative");
                }
                args.max_clock_skew = Some(seconds);
            }
            "--now" => {
                let text = value("--now")?;
                args.now = Some(
                    DateTime::parse_from_rfc3339(&text)
                        .with_context(|| format!("--now {text:?} is not RFC 3339"))?
                        .with_timezone(&Utc),
                );
            }
            "-h" | "--help" => bail!("help requested"),
            other => bail!("unknown argument {other:?}"),
        }
    }
    if args.request.is_none() || args.decision.is_none() || args.keys.is_none() {
        bail!("--request, --decision and --keys are all required");
    }
    Ok(args)
}

fn run(args: &Args) -> anyhow::Result<Outcome> {
    let request_path = args.request.as_ref().expect("validated");
    let decision_path = args.decision.as_ref().expect("validated");
    let keys_path = args.keys.as_ref().expect("validated");

    let (request, mut warnings) = load_request(request_path)?;
    let pinned_keys = load_keys(keys_path)?;
    let decision = match load_decision(decision_path, args.expect_user.as_deref())? {
        DecisionSource::Found(decision) => *decision,
        DecisionSource::NotRecorded(reason) => {
            return Ok(Outcome::Fail {
                error: reason,
                warnings,
            });
        }
    };

    let verified = verify_decision_receipt(ReceiptVerificationInput {
        request: &request,
        decision: &decision,
        pinned_keys: &pinned_keys,
        now: args.now.unwrap_or_else(Utc::now),
        max_clock_skew: args.max_clock_skew.map(Duration::seconds),
        strict_time: args.strict_time,
    });
    let mut verified = match verified {
        Ok(verified) => verified,
        Err(error) => {
            return Ok(Outcome::Fail {
                error: error.to_string(),
                warnings,
            });
        }
    };
    if let Some(expected) = args.expect_user.as_deref() {
        if verified.user_id != expected {
            return Ok(Outcome::Fail {
                error: format!(
                    "verified actor is {:?}, expected {expected:?}",
                    verified.user_id
                ),
                warnings,
            });
        }
    }
    if let Some(expected) = args.expect_option.as_deref() {
        if verified.option_id != expected {
            return Ok(Outcome::Fail {
                error: format!(
                    "verified option is {:?}, expected {expected:?}",
                    verified.option_id
                ),
                warnings,
            });
        }
    }
    warnings.append(&mut verified.warnings);
    verified.warnings = warnings;
    Ok(Outcome::Pass(Box::new(verified)))
}

fn read_json(path: &PathBuf) -> anyhow::Result<Value> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {} as JSON", path.display()))
}

/// Accept the create response or a bare request. A deduped create response
/// may describe an earlier pending request rather than the one just posted,
/// which the caller must reconcile; surface that as a warning.
fn load_request(path: &PathBuf) -> anyhow::Result<(Request, Vec<String>)> {
    let value = read_json(path)?;
    let mut warnings = Vec::new();
    let request_value = match value.get("request") {
        Some(inner @ Value::Object(_)) => {
            if value.get("deduped").and_then(Value::as_bool) == Some(true) {
                warnings.push(
                    "create response was deduped: confirm the returned request content is what you submitted"
                        .to_string(),
                );
            }
            inner.clone()
        }
        _ => value,
    };
    let request: Request = serde_json::from_value(request_value)
        .with_context(|| format!("{} is not a Nod request or create response", path.display()))?;
    if request.id.is_empty() {
        bail!("{} has an empty request id", path.display());
    }
    Ok((request, warnings))
}

enum DecisionSource {
    Found(Box<Decision>),
    NotRecorded(String),
}

/// Accept the `/decision` or `/wait` response, the callback body, or a bare
/// decision record. Per-user responses need `--expect-user` unless exactly one
/// decision is present.
fn load_decision(path: &PathBuf, expect_user: Option<&str>) -> anyhow::Result<DecisionSource> {
    let value = read_json(path)?;
    let Value::Object(object) = &value else {
        bail!("{} is not a JSON object", path.display());
    };
    let looks_bare = object.contains_key("option_id") && object.contains_key("signature");
    if looks_bare {
        let decision: Decision = serde_json::from_value(value.clone())
            .with_context(|| format!("{} is not a Nod decision record", path.display()))?;
        return Ok(DecisionSource::Found(Box::new(decision)));
    }
    let is_view = object.contains_key("decision") || object.contains_key("decisions");
    if !is_view {
        bail!(
            "{} is neither a decision view (decision/decisions) nor a bare decision record",
            path.display()
        );
    }
    if let Some(shared @ Value::Object(_)) = object.get("decision") {
        let decision: Decision = serde_json::from_value(shared.clone())
            .with_context(|| format!("{} has a malformed decision", path.display()))?;
        return Ok(DecisionSource::Found(Box::new(decision)));
    }
    let per_user: Vec<nod_proto::UserDecision> = match object.get("decisions") {
        Some(list @ Value::Array(_)) => serde_json::from_value(list.clone())
            .with_context(|| format!("{} has malformed decisions", path.display()))?,
        _ => Vec::new(),
    };
    if per_user.is_empty() {
        let status = object
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let timed_out = object.get("timed_out").and_then(Value::as_bool) == Some(true);
        return Ok(DecisionSource::NotRecorded(format!(
            "no decision recorded (status {status:?}{})",
            if timed_out { ", wait timed out" } else { "" }
        )));
    }
    match expect_user {
        Some(user_id) => per_user
            .into_iter()
            .find(|entry| entry.user_id == user_id)
            .map(|entry| DecisionSource::Found(Box::new(entry.decision)))
            .map(Ok)
            .unwrap_or_else(|| {
                Ok(DecisionSource::NotRecorded(format!(
                    "no decision recorded for user {user_id:?}"
                )))
            }),
        None if per_user.len() == 1 => Ok(DecisionSource::Found(Box::new(
            per_user.into_iter().next().expect("one entry").decision,
        ))),
        None => {
            let users: Vec<&str> = per_user
                .iter()
                .map(|entry| entry.user_id.as_str())
                .collect();
            bail!(
                "{} holds per-user decisions for {users:?}; pass --expect-user to pick one",
                path.display()
            )
        }
    }
}

fn load_keys(path: &PathBuf) -> anyhow::Result<Vec<PinnedKey>> {
    let value = read_json(path)?;
    let file: KeysFile = serde_json::from_value(value)
        .with_context(|| format!("{} is not a keys file ({{\"keys\":[...]}})", path.display()))?;
    if file.keys.is_empty() {
        bail!("{} pins no keys", path.display());
    }
    file.keys
        .into_iter()
        .map(|entry| {
            nod_proto::validate_public_key(&entry.public_key).map_err(|_| {
                anyhow!(
                    "pinned key {:?} for user {:?} is not a base64url uncompressed P-256 public key",
                    entry.key_id,
                    entry.user_id
                )
            })?;
            if entry.user_id.trim().is_empty() || entry.key_id.trim().is_empty() {
                bail!("pinned keys need a user_id and a key_id");
            }
            Ok(PinnedKey {
                user_id: entry.user_id,
                device_id: entry.device_id,
                key_id: entry.key_id,
                public_key: entry.public_key,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_flags_are_enforced() {
        assert!(parse_args(["--request", "a"].map(String::from).into_iter()).is_err());
        let args = parse_args(
            [
                "--request",
                "a",
                "--decision",
                "b",
                "--keys",
                "c",
                "--strict-time",
                "--max-clock-skew",
                "30",
                "--expect-option",
                "approve",
            ]
            .map(String::from)
            .into_iter(),
        )
        .unwrap();
        assert!(args.strict_time);
        assert_eq!(args.max_clock_skew, Some(30));
        assert_eq!(args.expect_option.as_deref(), Some("approve"));
        assert!(parse_args(["--bogus"].map(String::from).into_iter()).is_err());
    }
}
