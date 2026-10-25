//! End-to-end checks of the `nod-verify` binary: shapes it accepts, exit codes,
//! and the one-line JSON report. Keys are generated per test; nothing here is
//! real key material.

use std::{path::Path, process::Command};

use chrono::{TimeZone, Utc};
use nod_proto::{
    decision_signing_payload, generate_signing_key, recipients_commitment, request_digest,
    request_digest_v2, sign_payload, CardField, Decision, DecisionResolution,
    DecisionSignatureRecord, DecisionSigningInput, OptionKind, Request, RequestNotification,
    RequestOption, RequestSigningContext, RequestStatus, SigningKeyPair,
    DECISION_SIGNING_ALGORITHM, PRIVATE_REQUEST_DIGEST_VERSION,
};
use serde_json::{json, Value};

struct Fixture {
    dir: tempfile::TempDir,
    request: Request,
    key: SigningKeyPair,
}

impl Fixture {
    fn new() -> Self {
        let mut request = Request {
            answer_field: None,
            id: "req-cli".to_string(),
            request_id: "req-cli".to_string(),
            channel_id: "default".to_string(),
            recipients: vec!["owner".to_string()],
            decision_resolution: DecisionResolution::Shared,
            title: "Approve the thing".to_string(),
            summary: "It is ready".to_string(),
            body_markdown: String::new(),
            fields: vec![CardField {
                label: "Env".to_string(),
                value: "prod".to_string(),
                style: None,
            }],
            links: Vec::new(),
            image_url: None,
            notification: RequestNotification::default(),
            dedupe_key: None,
            expires_at: None,
            status: RequestStatus::Pending,
            created_at: Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap(),
            resolved_at: None,
            decision: None,
            decisions: Vec::new(),
            callback_url: None,
            options: vec![
                option("approve", "Approve", OptionKind::Approve),
                option("reject", "Reject", OptionKind::Reject),
            ],
            request_digest: None,
            signing: None,
        };
        let commitment = recipients_commitment(&request.recipients, "cli-test-salt").unwrap();
        request.request_digest = Some(request_digest(&request).unwrap());
        request.signing = Some(RequestSigningContext {
            version: PRIVATE_REQUEST_DIGEST_VERSION.to_string(),
            recipients_commitment: commitment.clone(),
            request_digest: request_digest_v2(&request, &commitment).unwrap(),
        });
        Self {
            dir: tempfile::tempdir().unwrap(),
            request,
            key: generate_signing_key(),
        }
    }

    fn created_response(&self) -> Value {
        json!({
            "request_id": self.request.id,
            "deduped": false,
            "request": self.request,
        })
    }

    fn decision(&self, option_id: &str, user_id: &str) -> Decision {
        let option = self
            .request
            .options
            .iter()
            .find(|option| option.id == option_id)
            .unwrap();
        let digest = self.request.request_digest.clone().unwrap();
        let signed_at = "2026-06-01T09:01:00.000Z";
        let payload = decision_signing_payload(DecisionSigningInput {
            request_id: &self.request.id,
            request_digest: &digest,
            option_id,
            option_kind: &option.kind,
            user_id,
            device_id: "device-cli",
            key_id: "key-cli",
            nonce: "nonce-cli",
            signed_at,
            text: None,
        });
        let signature = sign_payload(&self.key.private_key, payload.as_bytes()).unwrap();
        Decision {
            answer_value: None,
            request_id: self.request.id.clone(),
            option_id: option.id.clone(),
            option_kind: option.kind.clone(),
            option_label: option.label.clone(),
            text: None,
            actor_user_id: Some(user_id.to_string()),
            actor_device_id: Some("device-cli".to_string()),
            signature: Some(DecisionSignatureRecord {
                payload_version: None,
                key_id: "key-cli".to_string(),
                algorithm: DECISION_SIGNING_ALGORITHM.to_string(),
                nonce: "nonce-cli".to_string(),
                signed_at: signed_at.to_string(),
                request_digest: digest,
                signing_payload: payload,
                signature,
                verified: true,
                public_key: Some(self.key.public_key.clone()),
            }),
            resolved_at: Utc.with_ymd_and_hms(2026, 6, 1, 9, 1, 1).unwrap(),
        }
    }

    fn decision_view(&self, decision: Option<&Decision>) -> Value {
        json!({
            "request_id": self.request.id,
            "status": if decision.is_some() { "resolved" } else { "pending" },
            "decision": decision,
            "decisions": [],
            "decision_resolution": "shared",
            "recipients": ["owner"],
            "pending_recipients": [],
            "request_digest": self.request.request_digest,
        })
    }

    fn keys(&self, user_id: &str, public_key: &str) -> Value {
        json!({ "keys": [{ "user_id": user_id, "key_id": "key-cli", "public_key": public_key }] })
    }

    fn write(&self, name: &str, value: &Value) -> String {
        let path = self.dir.path().join(name);
        std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
        path.display().to_string()
    }
}

fn option(id: &str, label: &str, kind: OptionKind) -> RequestOption {
    RequestOption {
        id: id.to_string(),
        label: label.to_string(),
        kind,
        style: "default".to_string(),
        requires_text: false,
        text_placeholder: None,
        destructive: false,
        foreground: false,
    }
}

fn run(args: &[&str]) -> (i32, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_nod-verify"))
        .args(args)
        .output()
        .expect("run nod-verify");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "expected one JSON line, got {stdout:?}");
    let report: Value = serde_json::from_str(lines[0]).expect("JSON report");
    (output.status.code().unwrap(), report)
}

fn base_args<'a>(request: &'a str, decision: &'a str, keys: &'a str) -> Vec<&'a str> {
    vec![
        "--request",
        request,
        "--decision",
        decision,
        "--keys",
        keys,
        "--now",
        "2026-06-01T09:02:00Z",
    ]
}

#[test]
fn passes_on_a_decision_view_and_reports_the_facts() {
    let fx = Fixture::new();
    let decision = fx.decision("approve", "owner");
    let request = fx.write("created.json", &fx.created_response());
    let view = fx.write("decision.json", &fx.decision_view(Some(&decision)));
    let keys = fx.write("keys.json", &fx.keys("owner", &fx.key.public_key));
    let mut args = base_args(&request, &view, &keys);
    args.extend(["--expect-user", "owner", "--expect-option", "approve"]);
    let (code, report) = run(&args);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["ok"], true);
    assert_eq!(report["user_id"], "owner");
    assert_eq!(report["device_id"], "device-cli");
    assert_eq!(report["key_id"], "key-cli");
    assert_eq!(report["option_id"], "approve");
    assert_eq!(report["option_kind"], "approve");
    assert_eq!(report["digest_version"], "nod-request-v1");
    assert_eq!(report["warnings"], json!([]));
}

#[test]
fn accepts_callback_body_and_bare_decision_shapes() {
    let fx = Fixture::new();
    let decision = fx.decision("reject", "owner");
    let request = fx.write("created.json", &fx.created_response());
    let keys = fx.write("keys.json", &fx.keys("owner", &fx.key.public_key));

    let callback = fx.write(
        "callback.json",
        &json!({
            "request_id": fx.request.id,
            "channel_id": "default",
            "status": "resolved",
            "decision": decision,
            "decisions": [],
            "decision_resolution": "shared",
        }),
    );
    let (code, report) = run(&base_args(&request, &callback, &keys));
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["option_id"], "reject");

    let bare = fx.write("bare.json", &serde_json::to_value(&decision).unwrap());
    let (code, report) = run(&base_args(&request, &bare, &keys));
    assert_eq!(code, 0, "{report}");

    // The bare request shape works too.
    let bare_request = fx.write("request.json", &serde_json::to_value(&fx.request).unwrap());
    let (code, report) = run(&base_args(&bare_request, &bare, &keys));
    assert_eq!(code, 0, "{report}");
}

#[test]
fn per_user_views_pick_by_expect_user() {
    let fx = Fixture::new();
    let owner = fx.decision("approve", "owner");
    let other = fx.decision("reject", "other");
    let request = fx.write("created.json", &fx.created_response());
    let keys = fx.write("keys.json", &fx.keys("owner", &fx.key.public_key));
    let view = fx.write(
        "per-user.json",
        &json!({
            "request_id": fx.request.id,
            "status": "resolved",
            "decision": null,
            "decisions": [
                { "user_id": "owner", "decision": owner },
                { "user_id": "other", "decision": other },
            ],
            "decision_resolution": "per_user",
        }),
    );
    let (code, report) = run(&base_args(&request, &view, &keys));
    assert_eq!(code, 2, "{report}");
    assert_eq!(report["stage"], "input");

    let mut args = base_args(&request, &view, &keys);
    args.extend(["--expect-user", "owner"]);
    let (code, report) = run(&args);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["user_id"], "owner");

    let mut args = base_args(&request, &view, &keys);
    args.extend(["--expect-user", "nobody"]);
    let (code, report) = run(&args);
    assert_eq!(code, 1, "{report}");
}

#[test]
fn verification_failures_exit_one() {
    let fx = Fixture::new();
    let decision = fx.decision("approve", "owner");
    let request = fx.write("created.json", &fx.created_response());
    let keys = fx.write("keys.json", &fx.keys("owner", &fx.key.public_key));

    // Null signature.
    let mut unsigned = decision.clone();
    unsigned.signature = None;
    let view = fx.write("unsigned.json", &fx.decision_view(Some(&unsigned)));
    let (code, report) = run(&base_args(&request, &view, &keys));
    assert_eq!(code, 1, "{report}");
    assert_eq!(report["stage"], "verification");
    assert!(report["error"].as_str().unwrap().contains("no signature"));

    // Still pending.
    let pending = fx.write("pending.json", &fx.decision_view(None));
    let (code, report) = run(&base_args(&request, &pending, &keys));
    assert_eq!(code, 1, "{report}");
    assert!(report["error"].as_str().unwrap().contains("no decision"));

    // Wrong pinned key, even though the record names the real one.
    let view = fx.write("decision.json", &fx.decision_view(Some(&decision)));
    let other_key = fx.write(
        "other-keys.json",
        &fx.keys("owner", &generate_signing_key().public_key),
    );
    let (code, report) = run(&base_args(&request, &view, &other_key));
    assert_eq!(code, 1, "{report}");

    // Tampered title in the create response.
    let mut tampered = fx.created_response();
    tampered["request"]["title"] = json!("Approve something else");
    let tampered = fx.write("tampered.json", &tampered);
    let (code, report) = run(&base_args(&tampered, &view, &keys));
    assert_eq!(code, 1, "{report}");
    assert!(report["error"].as_str().unwrap().contains("request_digest"));

    // Expectation mismatch.
    let mut args = base_args(&request, &view, &keys);
    args.extend(["--expect-option", "reject"]);
    let (code, report) = run(&args);
    assert_eq!(code, 1, "{report}");

    // Strict time with a far-off clock.
    let (code, report) = run(&[
        "--request",
        &request,
        "--decision",
        &view,
        "--keys",
        &keys,
        "--now",
        "2027-01-01T00:00:00Z",
        "--strict-time",
    ]);
    assert_eq!(code, 1, "{report}");
    // ... and only a warning without --strict-time.
    let (code, report) = run(&[
        "--request",
        &request,
        "--decision",
        &view,
        "--keys",
        &keys,
        "--now",
        "2027-01-01T00:00:00Z",
    ]);
    assert_eq!(code, 0, "{report}");
    assert!(!report["warnings"].as_array().unwrap().is_empty());
}

#[test]
fn input_errors_exit_two() {
    let fx = Fixture::new();
    let decision = fx.decision("approve", "owner");
    let request = fx.write("created.json", &fx.created_response());
    let view = fx.write("decision.json", &fx.decision_view(Some(&decision)));
    let keys = fx.write("keys.json", &fx.keys("owner", &fx.key.public_key));

    let missing = fx.dir.path().join("missing.json").display().to_string();
    let (code, report) = run(&base_args(&missing, &view, &keys));
    assert_eq!(code, 2, "{report}");
    assert_eq!(report["stage"], "input");

    let bad_keys = fx.write("bad-keys.json", &json!({ "keys": [] }));
    let (code, _) = run(&base_args(&request, &view, &bad_keys));
    assert_eq!(code, 2);

    let malformed_key = fx.write("malformed-keys.json", &fx.keys("owner", "not-a-key"));
    let (code, _) = run(&base_args(&request, &view, &malformed_key));
    assert_eq!(code, 2);

    let not_a_decision = fx.write("nope.json", &json!({ "hello": "world" }));
    let (code, _) = run(&base_args(&request, &not_a_decision, &keys));
    assert_eq!(code, 2);

    let (code, _) = run(&["--request", &request]);
    assert_eq!(code, 2);
    assert!(Path::new(&request).exists());
}
