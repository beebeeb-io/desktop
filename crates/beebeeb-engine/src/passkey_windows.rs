//! v2 B15: the Windows platform call for core's passkey flows: webauthn.dll `WebAuthNAuthenticatorMakeCredential` /
//! `WebAuthNAuthenticatorGetAssertion`, with PRF (`bEnablePrf` at create, the hmac-secret salt at get). Nothing
//! here decides anything: core builds the options and the `clientDataJSON`, and gets the raw response and the PRF
//! output back.
//!
//! The PRF salt: webauthn.dll is given core's `hmac_secret_salt()` (the WebAuthn PRF hash already applied) with
//! `WEBAUTHN_AUTHENTICATOR_HMAC_SECRET_VALUES_FLAG`, so the authenticator sees the salt a browser would send for
//! web's PRF input and returns the same output (see `beebeeb_core::passkey`).
//!
//! The blocking calls run on the blocking pool; webauthn.dll shows its own Windows Security dialog over `hwnd`.

use std::ffi::c_void;

use beebeeb_core::passkey::PrfOutput;
use beebeeb_core::passkey::ceremony::{
    CreationOptions, NativeAssertion, NativeAttestation, PasskeyError, RequestOptions,
};
use beebeeb_core::passkey::flow::{Assertion, PasskeyAuthenticator, Registration};
use windows::Win32::Foundation::{BOOL, HWND};
use windows::Win32::Networking::WindowsWebServices::*;
use windows::core::{HSTRING, PCWSTR};

/// `NTE_USER_CANCELLED`: the person closed the Windows Security dialog (or it timed out).
const NTE_USER_CANCELLED: i32 = 0x8009_0036_u32 as i32;
/// The first API version with PRF at create (`bEnablePrf`) and hmac-secret salts at get.
const MIN_API_VERSION: u32 = 4;

pub struct WindowsAuthenticator {
    hwnd: isize,
}

impl WindowsAuthenticator {
    pub fn new(hwnd: isize) -> Self {
        Self { hwnd }
    }
}

fn join_err(_: tokio::task::JoinError) -> PasskeyError {
    PasskeyError::Platform { code: "join".into() }
}

impl PasskeyAuthenticator for WindowsAuthenticator {
    fn make_credential(
        &self,
        options: &CreationOptions,
    ) -> impl std::future::Future<Output = Result<Registration, PasskeyError>> + Send {
        let (hwnd, options) = (self.hwnd, options.clone());
        async move {
            tokio::task::spawn_blocking(move || make_credential(hwnd, &options))
                .await
                .map_err(join_err)?
        }
    }

    fn get_assertion(
        &self,
        options: &RequestOptions,
    ) -> impl std::future::Future<Output = Result<Assertion, PasskeyError>> + Send {
        let (hwnd, options) = (self.hwnd, options.clone());
        async move {
            tokio::task::spawn_blocking(move || get_assertion(hwnd, &options))
                .await
                .map_err(join_err)?
        }
    }
}

fn api_version() -> Result<u32, PasskeyError> {
    // SAFETY: no arguments; webauthn.dll is a system DLL on every supported Windows.
    let version = unsafe { WebAuthNGetApiVersionNumber() };
    if version < MIN_API_VERSION {
        return Err(PasskeyError::Unavailable);
    }
    Ok(version)
}

/// Whether the WebAuthn API of this Windows can do passkeys with PRF (the version check alone: no dialog).
pub fn platform_available() -> bool {
    api_version().is_ok()
}

fn platform_err(e: windows::core::Error) -> PasskeyError {
    if e.code().0 == NTE_USER_CANCELLED {
        PasskeyError::Cancelled
    } else {
        PasskeyError::Platform {
            code: format!("{:#010x}", e.code().0 as u32),
        }
    }
}

fn user_verification(value: &str) -> u32 {
    match value {
        "required" => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
        "discouraged" => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
        _ => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_PREFERRED,
    }
}

/// Copy `len` bytes the DLL owns (freed by its own free function afterwards).
///
/// # Safety
/// `ptr` must be valid for `len` bytes, or `len` must be 0.
unsafe fn copy(ptr: *const u8, len: u32) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr, len as usize) }.to_vec()
}

/// A credential list (`WEBAUTHN_CREDENTIAL_LIST`) over borrowed ids. Lives as long as the ids.
struct CredentialList {
    _entries: Vec<WEBAUTHN_CREDENTIAL_EX>,
    _pointers: Vec<*mut WEBAUTHN_CREDENTIAL_EX>,
    list: WEBAUTHN_CREDENTIAL_LIST,
}

impl CredentialList {
    fn new(ids: &[Vec<u8>], kind: &HSTRING) -> Self {
        let mut entries: Vec<WEBAUTHN_CREDENTIAL_EX> = ids
            .iter()
            .map(|id| WEBAUTHN_CREDENTIAL_EX {
                dwVersion: WEBAUTHN_CREDENTIAL_EX_CURRENT_VERSION,
                cbId: id.len() as u32,
                pbId: id.as_ptr() as *mut u8,
                pwszCredentialType: PCWSTR(kind.as_ptr()),
                dwTransports: 0,
            })
            .collect();
        let mut pointers: Vec<*mut WEBAUTHN_CREDENTIAL_EX> = entries.iter_mut().map(|e| e as *mut _).collect();
        let list = WEBAUTHN_CREDENTIAL_LIST {
            cCredentials: pointers.len() as u32,
            ppCredentials: pointers.as_mut_ptr(),
        };
        Self {
            _entries: entries,
            _pointers: pointers,
            list,
        }
    }

    fn ptr(&mut self) -> *mut WEBAUTHN_CREDENTIAL_LIST {
        if self.list.cCredentials == 0 {
            std::ptr::null_mut()
        } else {
            &mut self.list
        }
    }
}

fn make_credential(hwnd: isize, o: &CreationOptions) -> Result<Registration, PasskeyError> {
    let version = api_version()?;
    let public_key = HSTRING::from("public-key");
    let sha256 = HSTRING::from("SHA-256");
    let rp_id = HSTRING::from(o.rp_id.as_str());
    let rp_name = HSTRING::from(o.rp_name.as_str());
    let user_name = HSTRING::from(o.user_name.as_str());
    let user_display = HSTRING::from(o.user_display_name.as_str());
    let mut user_id = o.user_id.clone();
    let mut client_data = o.client_data_json();

    let rp = WEBAUTHN_RP_ENTITY_INFORMATION {
        dwVersion: WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION,
        pwszId: PCWSTR(rp_id.as_ptr()),
        pwszName: PCWSTR(rp_name.as_ptr()),
        pwszIcon: PCWSTR::null(),
    };
    let user = WEBAUTHN_USER_ENTITY_INFORMATION {
        dwVersion: WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
        cbId: user_id.len() as u32,
        pbId: user_id.as_mut_ptr(),
        pwszName: PCWSTR(user_name.as_ptr()),
        pwszIcon: PCWSTR::null(),
        pwszDisplayName: PCWSTR(user_display.as_ptr()),
    };
    let mut params: Vec<WEBAUTHN_COSE_CREDENTIAL_PARAMETER> = o
        .algorithms
        .iter()
        .map(|alg| WEBAUTHN_COSE_CREDENTIAL_PARAMETER {
            dwVersion: WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION,
            pwszCredentialType: PCWSTR(public_key.as_ptr()),
            lAlg: *alg,
        })
        .collect();
    let cose = WEBAUTHN_COSE_CREDENTIAL_PARAMETERS {
        cCredentialParameters: params.len() as u32,
        pCredentialParameters: params.as_mut_ptr(),
    };
    let client = WEBAUTHN_CLIENT_DATA {
        dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
        cbClientDataJSON: client_data.len() as u32,
        pbClientDataJSON: client_data.as_mut_ptr(),
        pwszHashAlgId: PCWSTR(sha256.as_ptr()),
    };
    let mut exclude = CredentialList::new(&o.exclude_credentials, &public_key);
    let mut opts = WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS {
        // Version 6 carries `bEnablePrf`; 7 adds fields we leave zero.
        dwVersion: if version >= 7 { 7 } else { 6 },
        dwTimeoutMilliseconds: o.timeout_ms,
        dwAuthenticatorAttachment: match o.authenticator_attachment.as_deref() {
            Some("platform") => WEBAUTHN_AUTHENTICATOR_ATTACHMENT_PLATFORM,
            Some("cross-platform") => WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
            _ => WEBAUTHN_AUTHENTICATOR_ATTACHMENT_ANY,
        },
        bRequireResidentKey: BOOL::from(o.resident_key == "required"),
        bPreferResidentKey: BOOL::from(o.resident_key == "preferred"),
        dwUserVerificationRequirement: user_verification(&o.user_verification),
        dwAttestationConveyancePreference: match o.attestation.as_str() {
            "direct" => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_DIRECT,
            "indirect" => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_INDIRECT,
            _ => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE,
        },
        pExcludeCredentialList: exclude.ptr(),
        bEnablePrf: BOOL::from(true),
        ..Default::default()
    };

    // SAFETY: every pointer above points into a local that outlives this call; the DLL copies what it keeps.
    let result = unsafe {
        WebAuthNAuthenticatorMakeCredential(
            HWND(hwnd as *mut c_void),
            &rp,
            &user,
            &cose,
            &client,
            Some(&mut opts as *mut _ as *const _),
        )
    }
    .map_err(platform_err)?;
    if result.is_null() {
        return Err(PasskeyError::Platform { code: "null".into() });
    }
    // SAFETY: a non-null result from the DLL, read before it is freed below.
    let registration = unsafe {
        let att = &*result;
        let prf_enabled = (att.dwVersion >= 5).then(|| att.bPrfEnabled.as_bool());
        Registration {
            attestation: NativeAttestation {
                credential_id: copy(att.pbCredentialId, att.cbCredentialId),
                attestation_object: copy(att.pbAttestationObject, att.cbAttestationObject),
                client_data_json: client_data.clone(),
            },
            // webauthn.dll evaluates no PRF during create; core's flow asks for it with one local assertion.
            prf_output: None,
            prf_enabled,
        }
    };
    // SAFETY: the pointer came from WebAuthNAuthenticatorMakeCredential and is freed once.
    unsafe { WebAuthNFreeCredentialAttestation(Some(result)) };
    if registration.attestation.credential_id.is_empty() || registration.attestation.attestation_object.is_empty() {
        return Err(PasskeyError::Malformed("attestation"));
    }
    Ok(registration)
}

fn get_assertion(hwnd: isize, o: &RequestOptions) -> Result<Assertion, PasskeyError> {
    let version = api_version()?;
    let public_key = HSTRING::from("public-key");
    let sha256 = HSTRING::from("SHA-256");
    let rp_id = HSTRING::from(o.rp_id.as_str());
    let mut client_data = o.client_data_json();
    let mut salt = o.prf.hmac_secret_salt.clone();
    let client = WEBAUTHN_CLIENT_DATA {
        dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
        cbClientDataJSON: client_data.len() as u32,
        pbClientDataJSON: client_data.as_mut_ptr(),
        pwszHashAlgId: PCWSTR(sha256.as_ptr()),
    };
    let mut global_salt = WEBAUTHN_HMAC_SECRET_SALT {
        cbFirst: salt.len() as u32,
        pbFirst: salt.as_mut_ptr(),
        cbSecond: 0,
        pbSecond: std::ptr::null_mut(),
    };
    let mut salts = WEBAUTHN_HMAC_SECRET_SALT_VALUES {
        pGlobalHmacSalt: &mut global_salt,
        cCredWithHmacSecretSaltList: 0,
        pCredWithHmacSecretSaltList: std::ptr::null_mut(),
    };
    let mut allow = CredentialList::new(&o.allow_credentials, &public_key);
    let mut opts = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS {
        // Version 6 carries `pHmacSecretSaltValues`; 7 adds fields we leave zero.
        dwVersion: if version >= 7 { 7 } else { 6 },
        dwTimeoutMilliseconds: o.timeout_ms,
        dwAuthenticatorAttachment: WEBAUTHN_AUTHENTICATOR_ATTACHMENT_ANY,
        dwUserVerificationRequirement: user_verification(&o.user_verification),
        // Core passes the hmac-secret salt with the PRF hash already applied: use it as is.
        dwFlags: WEBAUTHN_AUTHENTICATOR_HMAC_SECRET_VALUES_FLAG,
        pAllowCredentialList: allow.ptr(),
        pHmacSecretSaltValues: &mut salts,
        ..Default::default()
    };

    // SAFETY: every pointer above points into a local that outlives this call.
    let result = unsafe {
        WebAuthNAuthenticatorGetAssertion(
            HWND(hwnd as *mut c_void),
            PCWSTR(rp_id.as_ptr()),
            &client,
            Some(&mut opts as *mut _ as *const _),
        )
    }
    .map_err(platform_err)?;
    if result.is_null() {
        return Err(PasskeyError::Platform { code: "null".into() });
    }
    // SAFETY: a non-null result from the DLL, read before it is freed below.
    let assertion = unsafe {
        let a = &*result;
        let prf_output = if a.dwVersion >= 3 && !a.pHmacSecret.is_null() {
            let s = &*a.pHmacSecret;
            if s.cbFirst == 32 && !s.pbFirst.is_null() {
                let prf = PrfOutput::from_slice(std::slice::from_raw_parts(s.pbFirst, 32)).ok();
                // Wipe the DLL's copy before the DLL frees it.
                std::ptr::write_bytes(s.pbFirst, 0, 32);
                prf
            } else {
                None
            }
        } else {
            None
        };
        let user_handle = copy(a.pbUserId, a.cbUserId);
        Assertion {
            assertion: NativeAssertion {
                credential_id: copy(a.Credential.pbId, a.Credential.cbId),
                authenticator_data: copy(a.pbAuthenticatorData, a.cbAuthenticatorData),
                client_data_json: client_data.clone(),
                signature: copy(a.pbSignature, a.cbSignature),
                user_handle: (!user_handle.is_empty()).then_some(user_handle),
            },
            prf_output,
        }
    };
    // SAFETY: the pointer came from WebAuthNAuthenticatorGetAssertion and is freed once.
    unsafe { WebAuthNFreeAssertion(result) };
    if assertion.assertion.credential_id.is_empty() || assertion.assertion.signature.is_empty() {
        return Err(PasskeyError::Malformed("assertion"));
    }
    Ok(assertion)
}
