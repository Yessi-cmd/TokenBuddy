//! Read-only identity of the official Codex account (spec §8.1, §8.4, §15.3).
//!
//! Codex stores the signed-in identity in `<CODEX_HOME>/auth.json`: either a
//! ChatGPT OAuth token set or a plain `OPENAI_API_KEY`. TokenBuddy reads that
//! file to answer *whose* quota the rollout logs are reporting — nothing else.
//!
//! Nothing secret survives this module. The access token, refresh token, raw id
//! token, and API key are read, hashed with the per-install salt (spec §20.2),
//! and dropped; only the fingerprint, the auth mode, the plan, and the account
//! label reach the domain model, so a stolen TokenBuddy database cannot be
//! replayed against the upstream API (spec §20.1).

use std::{fs, path::Path};

use serde_json::Value;
use tokenbuddy_domain::{AccountRecord, LauncherKind, ProviderRecord, account_fingerprint};

/// File inside the Codex home that names the signed-in identity.
pub const AUTH_FILENAME: &str = "auth.json";
/// Provider that owns these accounts.
pub const PROVIDER_ID: &str = "openai";
/// Vendor family for grouping.
pub const PROVIDER_FAMILY: &str = "openai";
/// Provider name shown in the UI.
pub const PROVIDER_DISPLAY_NAME: &str = "OpenAI";
/// Auth mode recorded for a ChatGPT OAuth login.
pub const AUTH_MODE_CHATGPT: &str = "chatgpt";
/// Auth mode recorded for a plain API key.
pub const AUTH_MODE_API_KEY: &str = "api_key";

/// The OpenAI provider row that owns the official account. Emitted alongside the
/// account so the Providers and Quotas views resolve a real name even before the
/// first usage event of a fresh install has been imported.
pub fn official_provider() -> ProviderRecord {
    ProviderRecord {
        id: PROVIDER_ID.to_owned(),
        provider_family: PROVIDER_FAMILY.to_owned(),
        display_name: PROVIDER_DISPLAY_NAME.to_owned(),
        upstream_url: None,
        launcher: Some(LauncherKind::Direct),
        source_id: Some(super::SOURCE_ID.to_owned()),
    }
}

/// Read `<codex_home>/auth.json`. Returns `None` when the file is absent,
/// unreadable, malformed, or names no identity — an unknown account stays
/// unknown rather than becoming a placeholder.
pub fn read_official_account(codex_home: &Path, salt: &str) -> Option<AccountRecord> {
    let contents = fs::read_to_string(codex_home.join(AUTH_FILENAME)).ok()?;
    parse_account(&contents, salt)
}

fn parse_account(contents: &str, salt: &str) -> Option<AccountRecord> {
    let value: Value = serde_json::from_str(contents).ok()?;
    chatgpt_account(&value, salt).or_else(|| api_key_account(&value, salt))
}

fn chatgpt_account(value: &Value, salt: &str) -> Option<AccountRecord> {
    let tokens = value.get("tokens").filter(|tokens| !tokens.is_null())?;
    // The id token is a JWT whose payload carries the account id and plan. It is
    // decoded, never stored: only claims land in the record below.
    let claims = tokens
        .get("id_token")
        .and_then(Value::as_str)
        .and_then(decode_jwt_claims);
    let account_id = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| claim_string(claims.as_ref(), "chatgpt_account_id"))?;
    let fingerprint = account_fingerprint(salt, &account_id);
    Some(AccountRecord {
        id: account_row_id(AUTH_MODE_CHATGPT, &fingerprint),
        provider_id: PROVIDER_ID.to_owned(),
        display_name: Some(
            claim_string(claims.as_ref(), "email").unwrap_or_else(|| "ChatGPT 账号".to_owned()),
        ),
        account_fingerprint: fingerprint,
        auth_mode: AUTH_MODE_CHATGPT.to_owned(),
        plan: claim_string(claims.as_ref(), "chatgpt_plan_type"),
    })
}

fn api_key_account(value: &Value, salt: &str) -> Option<AccountRecord> {
    let api_key = value
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?;
    let fingerprint = account_fingerprint(salt, api_key);
    Some(AccountRecord {
        id: account_row_id(AUTH_MODE_API_KEY, &fingerprint),
        provider_id: PROVIDER_ID.to_owned(),
        // Never any part of the key itself — the salted fingerprint is the only
        // thing that identifies it, and it cannot be reversed to the key.
        display_name: Some(format!("API Key · {}", &fingerprint[..8])),
        account_fingerprint: fingerprint,
        auth_mode: AUTH_MODE_API_KEY.to_owned(),
        plan: None,
    })
}

fn account_row_id(auth_mode: &str, fingerprint: &str) -> String {
    format!("{PROVIDER_ID}:{auth_mode}:{}", &fingerprint[..16])
}

/// Claims live either at the payload root or inside one of the namespaced
/// OpenAI claim objects, depending on the token vintage. Look in all of them and
/// keep `None` when the claim is genuinely absent.
fn claim_string(claims: Option<&Value>, key: &str) -> Option<String> {
    let claims = claims?;
    const NAMESPACES: [&str; 2] = [
        "https://api.openai.com/auth",
        "https://api.openai.com/profile",
    ];
    claims
        .get(key)
        .and_then(Value::as_str)
        .or_else(|| {
            NAMESPACES
                .iter()
                .find_map(|namespace| claims.get(namespace)?.get(key)?.as_str())
        })
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Decode a JWT payload without verifying the signature. TokenBuddy is not
/// authenticating anybody here — it is reading the local copy of claims Codex
/// already trusted — so an unverified read is sufficient and keeps a crypto
/// dependency out of the adapter.
fn decode_jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = decode_base64_url(payload)?;
    serde_json::from_slice(&decoded).ok()
}

fn decode_base64_url(input: &str) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::{
        AUTH_MODE_API_KEY, AUTH_MODE_CHATGPT, decode_base64_url, parse_account,
        read_official_account,
    };

    const SALT: &str = "test-salt";

    fn jwt(payload: &str) -> String {
        // Header and signature are irrelevant to claim reading; only the middle
        // segment is decoded.
        format!("header.{}.signature", base64_url(payload.as_bytes()))
    }

    fn base64_url(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut encoded = String::new();
        for chunk in bytes.chunks(3) {
            let mut buffer = 0u32;
            for (index, byte) in chunk.iter().enumerate() {
                buffer |= u32::from(*byte) << (16 - 8 * index);
            }
            let characters = chunk.len() + 1;
            for index in 0..characters {
                let value = (buffer >> (18 - 6 * index)) & 0b11_1111;
                encoded.push(char::from(ALPHABET[value as usize]));
            }
        }
        encoded
    }

    #[test]
    fn reads_the_chatgpt_account_plan_without_storing_any_token() {
        let id_token = jwt(
            r#"{"email":"user@example.com","https://api.openai.com/auth":{"chatgpt_account_id":"acct-fixture","chatgpt_plan_type":"pro"}}"#,
        );
        let contents = format!(
            r#"{{"OPENAI_API_KEY":null,"tokens":{{"id_token":"{id_token}","access_token":"access-secret","refresh_token":"refresh-secret","account_id":"acct-fixture"}},"last_refresh":"2026-07-26T00:00:00Z"}}"#
        );

        let account = parse_account(&contents, SALT).expect("account");
        assert_eq!(account.auth_mode, AUTH_MODE_CHATGPT);
        assert_eq!(account.plan.as_deref(), Some("pro"));
        assert_eq!(account.display_name.as_deref(), Some("user@example.com"));
        assert_eq!(account.provider_id, "openai");
        assert!(account.id.starts_with("openai:chatgpt:"));
        // The fingerprint is salted, so it is neither the raw account id nor a
        // value another install could reproduce.
        assert_ne!(account.account_fingerprint, "acct-fixture");
        let serialized = serde_json::to_string(&account).expect("serialize");
        for secret in [
            "access-secret",
            "refresh-secret",
            "acct-fixture",
            id_token.as_str(),
        ] {
            assert!(
                !serialized.contains(secret),
                "account record leaked {secret}"
            );
        }
    }

    #[test]
    fn falls_back_to_the_jwt_account_id_when_the_token_set_omits_it() {
        let id_token = jwt(
            r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct-from-claims","chatgpt_plan_type":"plus"}}"#,
        );
        let contents = format!(r#"{{"tokens":{{"id_token":"{id_token}"}}}}"#);

        let account = parse_account(&contents, SALT).expect("account");
        assert_eq!(account.plan.as_deref(), Some("plus"));
        assert_eq!(account.display_name.as_deref(), Some("ChatGPT 账号"));
    }

    #[test]
    fn api_key_mode_is_identified_only_by_its_salted_fingerprint() {
        let account = parse_account(r#"{"OPENAI_API_KEY":"sk-fixture-secret"}"#, SALT)
            .expect("api key account");
        assert_eq!(account.auth_mode, AUTH_MODE_API_KEY);
        assert_eq!(account.plan, None);
        assert!(account.id.starts_with("openai:api_key:"));
        assert!(!account.account_fingerprint.contains("sk-fixture"));
        assert!(
            !account
                .display_name
                .as_deref()
                .expect("display name")
                .contains("sk-")
        );
    }

    #[test]
    fn the_same_account_keeps_one_stable_row_id_and_different_salts_do_not_collide() {
        let contents = r#"{"tokens":{"account_id":"acct-stable"}}"#;
        let first = parse_account(contents, SALT).expect("account");
        let second = parse_account(contents, SALT).expect("account");
        let other_install = parse_account(contents, "other-salt").expect("account");

        assert_eq!(first.id, second.id);
        assert_ne!(first.id, other_install.id);
    }

    #[test]
    fn missing_malformed_and_empty_auth_files_stay_unavailable() {
        let home = tempfile::tempdir().expect("home");
        assert!(read_official_account(home.path(), SALT).is_none());

        std::fs::write(home.path().join("auth.json"), "not json").expect("write");
        assert!(read_official_account(home.path(), SALT).is_none());

        std::fs::write(
            home.path().join("auth.json"),
            r#"{"OPENAI_API_KEY":null,"tokens":null}"#,
        )
        .expect("write");
        assert!(read_official_account(home.path(), SALT).is_none());
    }

    #[test]
    fn base64_url_decoding_rejects_input_that_is_not_base64() {
        assert_eq!(decode_base64_url("aGk"), Some(b"hi".to_vec()));
        assert_eq!(decode_base64_url("aGk="), Some(b"hi".to_vec()));
        assert_eq!(decode_base64_url("not base64!"), None);
    }
}
