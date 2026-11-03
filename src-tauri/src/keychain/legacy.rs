//! `legacy` items split out of `keychain.rs`.

use super::*;

/// Migrate the legacy flat keychain trio into the id-segmented layout.
///
/// `migrate_legacy_keychain_to_account(id)` builds the segmented store for `id`
/// and the legacy flat store, then runs [`migrate_legacy_keychain_between`].
/// LOG+SWALLOW errors at the call site — a migration failure must never block
/// startup, because the legacy READ FALLBACK keeps the vault usable.
pub fn migrate_legacy_keychain_to_account(account_id: &str) -> AuthResult<()> {
    let new = platform_keychain_store_for(account_id);
    let legacy = legacy_platform_keychain_store();
    migrate_legacy_keychain_between(&new, &legacy)
}

/// Core migration algorithm, generic over the two stores so it is unit-testable
/// with in-memory stores (the platform credential vaults can't run in CI).
///
/// `new` is the id-segmented destination, `legacy` the flat source. Steps mirror
/// blueprint §3:
///
/// 1/2. SHORT-CIRCUIT (interrupt-safe). If the new token is present AND
///      (the new key is present OR legacy has no key) → migration is already
///      complete; opportunistically sweep any leftover legacy entries and
///      return. The `OR legacy has no key` guards the partial-write hole: if the
///      new token landed but the new key did NOT and legacy still HAS a key, we
///      must fall THROUGH and re-run 4-6 (don't declare victory with a missing
///      key while a recoverable legacy key still exists).
/// 3.   Read the legacy token. `None` → nothing to migrate (fresh/clean) → Ok.
///      Read legacy key + email (each `Option`).
/// 4.   WRITE new: first remove any email the new store holds (it is not the
///      legacy session's), then token always; key/email only if legacy had them.
///      `SecretBytes` is `!Clone`, so the loaded key is MOVED straight into the
///      save (never bound + reused). `SessionToken` IS `Clone`.
/// 5.   VERIFY readback BEFORE any delete: new token present; if legacy had a
///      key → new key present; if legacy had email → new email present. ANY
///      failure → return `Err` WITHOUT deleting (vault still usable via the
///      legacy fallback).
/// 6.   ONLY now delete the legacy trio (each delete idempotent; NotFound → Ok).
pub fn migrate_legacy_keychain_between<N: AuthSecretStore, L: AuthSecretStore>(new: &N, legacy: &L) -> AuthResult<()> {
    // ── Steps 1-2: short-circuit if already (effectively) migrated ──────────
    let new_token_present = new.load_session_token()?.is_some();
    if new_token_present {
        let new_key_present = new.load_wrapped_master_key()?.is_some();
        let legacy_has_key = legacy.load_wrapped_master_key()?.is_some();
        if new_key_present || !legacy_has_key {
            // Migration complete. Opportunistically sweep any leftover legacy
            // entries (e.g. a crash between writing the new trio and deleting
            // legacy). Each delete is idempotent (NotFound → Ok).
            sweep_legacy(legacy)?;
            return Ok(());
        }
        // else: new token present but new key MISSING while legacy STILL HAS a
        // key → partial write. Fall through to re-run the write/verify/delete.
    }

    // ── Step 3: read legacy; nothing to migrate if there is no legacy token ──
    let Some(legacy_token) = legacy.load_session_token()? else {
        // No legacy session at all (fresh install, or already swept) → nothing
        // to do. If a new token exists it's handled by the short-circuit above.
        return Ok(());
    };
    let legacy_key = legacy.load_wrapped_master_key()?; // Option<SecretBytes>
    let legacy_email = legacy.load_account_email()?; // Option<String>
    let key_existed = legacy_key.is_some();
    let email_existed = legacy_email.is_some();

    // ── Step 4: write the new (segmented) trio ──────────────────────────────
    // An email the segmented store already holds belongs to whatever it held before, not to the legacy session:
    // it is removed BEFORE the legacy token is written, so the token is never kept next to another account's email
    // (the restore knows a stored session by its email). The legacy email, if any, is written below.
    new.delete_account_email()?;
    // A vault key the segmented store kept (a startup 401 keeps the key and drops the token) belongs to the session
    // stored there before, not to the legacy one: with no legacy key it is removed BEFORE the legacy token is written,
    // so the two are never paired (spec §5.6). A legacy key of its own replaces it below.
    if !key_existed {
        new.delete_wrapped_master_key()?;
    }
    // Token: SessionToken is Clone, but we own `legacy_token` here so pass by ref.
    new.save_session_token(&legacy_token)?;
    // Key: SecretBytes is !Clone → MOVE the loaded value straight into save.
    if let Some(key) = legacy_key {
        new.save_wrapped_master_key(key)?;
    }
    // Email: plain string metadata; store best-effort. An Unsupported store
    // (Linux stub) would error here, but on Linux there is no legacy token to
    // migrate in the first place, so this branch is unreachable there.
    if let Some(email) = &legacy_email {
        new.save_account_email(email)?;
    }

    // ── Step 5: VERIFY readback BEFORE deleting anything ────────────────────
    if new.load_session_token()?.is_none() {
        return Err(AuthStoreError::Backend(
            "migration verify failed: new session token not readable after write".to_string(),
        ));
    }
    if key_existed && new.load_wrapped_master_key()?.is_none() {
        return Err(AuthStoreError::Backend(
            "migration verify failed: new wrapped master key not readable after write".to_string(),
        ));
    }
    if email_existed && new.load_account_email()?.is_none() {
        return Err(AuthStoreError::Backend(
            "migration verify failed: new account email not readable after write".to_string(),
        ));
    }

    // ── Step 6: only now delete the legacy trio (idempotent) ────────────────
    sweep_legacy(legacy)
}

/// Delete the legacy flat trio. Each delete is idempotent (NotFound → Ok), so
/// this is safe to call when some/all entries are already gone — used both as
/// the final migration step and the opportunistic short-circuit sweep.
pub(super) fn sweep_legacy<L: AuthSecretStore>(legacy: &L) -> AuthResult<()> {
    legacy.delete_session_token()?;
    legacy.delete_wrapped_master_key()?;
    legacy.delete_account_email()?;
    Ok(())
}
