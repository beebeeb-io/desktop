//! `export` items split out of `diagnostic_redaction.rs`.

use super::*;

/// Secrets-only pass, for error text persisted locally in the state DB.
pub fn redact_secrets_only(error: &str) -> String {
    redact(error, &KnownNames::default(), Mode::SecretsOnly).text
}

/// Full export pass.
pub fn redact_for_export(error: &str, names: &KnownNames) -> RedactedError {
    redact(error, names, Mode::Export)
}

pub(super) fn redact(error: &str, names: &KnownNames, mode: Mode) -> RedactedError {
    let mut redactions = 0u32;
    let bounded: String = error
        .chars()
        .filter(|c| *c != NAME_MARK)
        .take(MAX_INPUT_CHARS)
        .collect();
    let scanned = if mode == Mode::Export {
        replace_known_names(&bounded, names)
    } else {
        bounded
    };

    let mut out: Vec<String> = Vec::new();
    // Armed by a credential label; stays armed across further labels
    // (`Authorization: Bearer <tok>`) and bare punctuation until a VALUE token
    // has actually been redacted. The value is redacted whatever it looks like
    // (UUID, number, standard word).
    let mut armed = false;
    for part in scanned.split_whitespace() {
        if out.len() >= MAX_OUTPUT_TOKENS {
            out.push("[truncated]".into());
            redactions += 1;
            break;
        }
        let shape = credential_shape(part);
        if armed {
            match shape {
                // Another label (`Bearer` after `Authorization:`) or a run of
                // punctuation: not the value yet, keep the skip armed.
                CredentialShape::Label | CredentialShape::Punctuation => {
                    out.push(part.replace(NAME_MARK, ""));
                    continue;
                }
                // `Cookie: session=<tok>`: the pair's own value is the secret.
                CredentialShape::LabelledValue {
                    key,
                    delim,
                    value: LabelledValueKind::Secret,
                } => {
                    out.push(format!("{}{delim}[redacted]", key.replace(NAME_MARK, "")));
                    redactions += 1;
                    armed = false;
                    continue;
                }
                CredentialShape::LabelledValue { key, delim, .. } => {
                    out.push(format!("{}{delim}[redacted]", key.replace(NAME_MARK, "")));
                    redactions += 1;
                    continue; // `x=Bearer` then the real value: stay armed
                }
                CredentialShape::Other => {
                    out.push(redact_value(part));
                    redactions += 1;
                    armed = false;
                    continue;
                }
            }
        }
        match shape {
            CredentialShape::Label => {
                out.push(part.replace(NAME_MARK, ""));
                armed = true;
                continue;
            }
            CredentialShape::LabelledValue {
                value: LabelledValueKind::Label,
                ..
            } => {
                // `authorization=Bearer <tok>`: the label run continues.
                out.push(part.replace(NAME_MARK, ""));
                armed = true;
                continue;
            }
            CredentialShape::LabelledValue {
                key,
                delim,
                value: LabelledValueKind::Secret,
            } => {
                out.push(format!("{}{delim}[redacted]", key.replace(NAME_MARK, "")));
                redactions += 1;
                continue;
            }
            CredentialShape::Punctuation | CredentialShape::Other => {}
        }
        if part.chars().count() > 96 {
            out.push("[redacted]".into());
            redactions += 1;
            continue;
        }
        if mode == Mode::SecretsOnly {
            out.push(part.into());
            continue;
        }
        match filter_token(part) {
            Filtered::Keep => out.push(part.into()),
            Filtered::Replace(text, counted) => {
                out.push(text);
                if counted {
                    redactions += 1;
                }
            }
        }
    }
    RedactedError {
        text: out.join(" "),
        redactions,
    }
}
