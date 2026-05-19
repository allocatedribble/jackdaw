//! Host-vs-dylib compatibility checking.
//!
//! Every dylib (extension or game) embeds an API version, a Bevy
//! version string, and a build profile (debug/release). The loader
//! refuses to bring one in unless all three match the host's
//! constants exactly. That catches the three most common reasons a
//! trait-object call across the dylib boundary would go sideways.

use core::ffi::CStr;
use std::ffi::c_char;

use bevy::ecs::world::World;
use jackdaw_api_internal::ffi::{
    API_VERSION, BEVY_VERSION, ExtensionEntry, GameEntry, JackdawExtensionPtr, PROFILE,
};

pub struct VerifiedExtensionEntry {
    pub ctor: unsafe extern "C" fn() -> JackdawExtensionPtr,
    pub dtor: unsafe extern "C" fn(JackdawExtensionPtr),
}

pub struct VerifiedGameEntry {
    pub name: String,
    pub build: unsafe extern "C" fn(*mut World),
    pub teardown: unsafe extern "C" fn(*mut World),
}

#[derive(Debug)]
pub enum CompatError {
    ApiVersionMismatch { host: u32, extension: u32 },
    BevyVersionMismatch { host: String, extension: String },
    ProfileMismatch { host: String, extension: String },
    NullPointer { field: &'static str },
    NonUtf8 { field: &'static str },
}

impl std::fmt::Display for CompatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiVersionMismatch { host, extension } => write!(
                f,
                "jackdaw_api ABI version mismatch: host v{host}, extension v{extension}. \
                 Rebuild the extension against jackdaw_api v{host}."
            ),
            Self::BevyVersionMismatch { host, extension } => write!(
                f,
                "Bevy version mismatch: host was built against {host}, extension against {extension}. \
                 Rebuild the extension against Bevy {host}."
            ),
            Self::ProfileMismatch { host, extension } => write!(
                f,
                "build profile mismatch: host is {host}, extension is {extension}. \
                 Rebuild the extension with the same profile as the host."
            ),
            Self::NullPointer { field } => {
                write!(f, "dylib entry field `{field}` is null")
            }
            Self::NonUtf8 { field } => {
                write!(f, "dylib entry field `{field}` is not valid UTF-8")
            }
        }
    }
}

impl std::error::Error for CompatError {}

/// Verify every embedded version tag against the host's values and
/// sanity-check that pointer fields are non-null.
pub fn verify_compat(entry: &ExtensionEntry) -> Result<VerifiedExtensionEntry, CompatError> {
    verify_version_fields(entry.api_version, entry.bevy_version, entry.profile)?;
    Ok(VerifiedExtensionEntry {
        ctor: require_fn(entry.ctor, "ctor")?,
        dtor: require_fn(entry.dtor, "dtor")?,
    })
}

/// Same as [`verify_compat`] but for a [`GameEntry`]. Both envelopes
/// share the same version-field layout, so the check itself is
/// structurally identical.
pub fn verify_game_compat(entry: &GameEntry) -> Result<VerifiedGameEntry, CompatError> {
    verify_version_fields(entry.api_version, entry.bevy_version, entry.profile)?;
    Ok(VerifiedGameEntry {
        name: cstr_to_string(entry.name, "name")?,
        build: require_fn(entry.build, "build")?,
        teardown: require_fn(entry.teardown, "teardown")?,
    })
}

fn verify_version_fields(
    api_version: u32,
    bevy_version: *const c_char,
    profile: *const c_char,
) -> Result<(), CompatError> {
    if api_version != API_VERSION {
        return Err(CompatError::ApiVersionMismatch {
            host: API_VERSION,
            extension: api_version,
        });
    }

    let ext_bevy = cstr_to_string(bevy_version, "bevy_version")?;
    let host_bevy = cstr_static_string(BEVY_VERSION);
    if ext_bevy != host_bevy {
        return Err(CompatError::BevyVersionMismatch {
            host: host_bevy,
            extension: ext_bevy,
        });
    }

    let ext_profile = cstr_to_string(profile, "profile")?;
    let host_profile = cstr_static_string(PROFILE);
    if ext_profile != host_profile {
        return Err(CompatError::ProfileMismatch {
            host: host_profile,
            extension: ext_profile,
        });
    }

    Ok(())
}

/// Read a dylib-provided C string into an owned `String`. Returns
/// errors tagged with `field` for readable diagnostics.
fn cstr_to_string(ptr: *const c_char, field: &'static str) -> Result<String, CompatError> {
    if ptr.is_null() {
        return Err(CompatError::NullPointer { field });
    }
    // SAFETY: caller contract: the pointer references a
    // NUL-terminated static string embedded in the dylib. The dylib
    // is kept alive for the duration of this call.
    let cstr = unsafe { CStr::from_ptr(ptr) };
    cstr.to_str()
        .map(ToOwned::to_owned)
        .map_err(|_| CompatError::NonUtf8 { field })
}

fn require_fn<F: Copy>(ptr: Option<F>, field: &'static str) -> Result<F, CompatError> {
    ptr.ok_or(CompatError::NullPointer { field })
}

/// Read one of our own host-side constant `CStrs` into an owned
/// `String`. The `to_str` cannot fail for the hard-coded values but
/// we still return `String` to share the comparison type with the
/// extension-side lookup.
fn cstr_static_string(cstr: &'static CStr) -> String {
    cstr.to_str().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::*;

    unsafe extern "C" fn dummy_ctor() -> JackdawExtensionPtr {
        unsafe { std::mem::zeroed() }
    }

    unsafe extern "C" fn dummy_dtor(_ptr: JackdawExtensionPtr) {}

    unsafe extern "C" fn dummy_game(_world: *mut World) {}

    fn valid_extension_entry() -> ExtensionEntry {
        ExtensionEntry {
            api_version: API_VERSION,
            bevy_version: BEVY_VERSION.as_ptr(),
            profile: PROFILE.as_ptr(),
            ctor: Some(dummy_ctor),
            dtor: Some(dummy_dtor),
        }
    }

    fn valid_game_entry() -> GameEntry {
        GameEntry {
            api_version: API_VERSION,
            bevy_version: BEVY_VERSION.as_ptr(),
            profile: PROFILE.as_ptr(),
            name: c"test_game".as_ptr(),
            build: Some(dummy_game),
            teardown: Some(dummy_game),
        }
    }

    macro_rules! assert_null_field {
        ($expr:expr, $field:literal) => {
            assert!(matches!(
                $expr,
                Err(CompatError::NullPointer { field }) if field == $field
            ));
        };
    }

    #[test]
    fn extension_entry_rejects_null_callable_fields() {
        let mut entry = valid_extension_entry();
        entry.ctor = None;
        assert_null_field!(verify_compat(&entry), "ctor");

        let mut entry = valid_extension_entry();
        entry.dtor = None;
        assert_null_field!(verify_compat(&entry), "dtor");
    }

    #[test]
    fn extension_entry_rejects_null_string_fields() {
        let mut entry = valid_extension_entry();
        entry.bevy_version = ptr::null();
        assert_null_field!(verify_compat(&entry), "bevy_version");

        let mut entry = valid_extension_entry();
        entry.profile = ptr::null();
        assert_null_field!(verify_compat(&entry), "profile");
    }

    #[test]
    fn game_entry_rejects_null_callable_fields() {
        let mut entry = valid_game_entry();
        entry.build = None;
        assert_null_field!(verify_game_compat(&entry), "build");

        let mut entry = valid_game_entry();
        entry.teardown = None;
        assert_null_field!(verify_game_compat(&entry), "teardown");
    }

    #[test]
    fn game_entry_rejects_null_and_invalid_name() {
        let mut entry = valid_game_entry();
        entry.name = ptr::null();
        assert_null_field!(verify_game_compat(&entry), "name");

        let bad_name = [0xff_u8, 0];
        let mut entry = valid_game_entry();
        entry.name = bad_name.as_ptr().cast();
        assert!(matches!(
            verify_game_compat(&entry),
            Err(CompatError::NonUtf8 { field }) if field == "name"
        ));
    }
}
