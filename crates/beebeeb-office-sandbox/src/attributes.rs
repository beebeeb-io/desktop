//! A token's security attributes, as the system answers `GetTokenInformation(TokenSecurityAttributes)`: a header, an
//! array of entries, and the names and values the entries point to, all inside the one answer buffer. The parse is
//! pure and safe: every pointer is turned into an offset into the buffer and checked against its length before
//! anything behind it is read, so a layout this crate does not expect refuses the helper instead of reading outside
//! the buffer. Pure, so its rules are tested on every operating system even though only the Windows module calls it.
#![cfg_attr(not(windows), allow(dead_code))]

use std::mem::{offset_of, size_of};

use crate::SandboxError;

/// The attribute the system puts on the token of a less-privileged AppContainer: the token is not a member of the
/// "all application packages" group. Compared without regard to ASCII case (the system's attribute names are not
/// case-sensitive).
pub(crate) const NO_ALL_APPLICATION_PACKAGES: &str = "WIN://NOALLAPPPKG";

/// The only version of the answer's layout.
const VERSION_1: u16 = 1;

/// The value type of an attribute whose values are unsigned 64-bit numbers.
const TYPE_UINT64: u16 = 2;

/// The answer's header. Pointers are kept as numbers: the parse only ever turns them into offsets. The three
/// structures are the layout only, read through `offset_of!`, never built.
#[allow(dead_code)]
#[repr(C)]
struct Header {
    version: u16,
    reserved: u16,
    count: u32,
    entries: usize,
}

/// A counted string: its length in bytes (no terminating zero), the room it has, and where its characters are.
#[allow(dead_code)]
#[repr(C)]
struct CountedName {
    length: u16,
    maximum: u16,
    characters: usize,
}

/// One attribute.
#[allow(dead_code)]
#[repr(C)]
struct Entry {
    name: CountedName,
    value_type: u16,
    reserved: u16,
    flags: u32,
    value_count: u32,
    values: usize,
}

const DOES_NOT_FIT: SandboxError = SandboxError::NotConfined("the security-attribute read-back does not fit");

/// Whether the attributes in `answer` say the token is a less-privileged AppContainer: the attribute
/// [`NO_ALL_APPLICATION_PACKAGES`] is present and set (an unsigned number other than zero). `base` is the address the
/// system wrote the answer at: the pointers inside the answer point into it.
///
/// - `Ok(false)`: the token has no such attribute (also: no attributes at all).
/// - `Err`: the answer does not have the expected layout, or the attribute is present but is not one set number.
///   Both refuse the helper, as a `false` does.
pub(crate) fn less_privileged(answer: &[u8], base: usize) -> Result<bool, SandboxError> {
    if answer.len() < size_of::<Header>() {
        return Err(DOES_NOT_FIT);
    }
    let version = u16_at(answer, offset_of!(Header, version)).ok_or(DOES_NOT_FIT)?;
    let count = u32_at(answer, offset_of!(Header, count)).ok_or(DOES_NOT_FIT)?;
    let entries = word_at(answer, offset_of!(Header, entries)).ok_or(DOES_NOT_FIT)?;
    if version != VERSION_1 {
        return Err(SandboxError::NotConfined(
            "the security-attribute read-back has an unknown version",
        ));
    }
    if count == 0 {
        return Ok(false);
    }
    let length = (count as usize).checked_mul(size_of::<Entry>()).ok_or(DOES_NOT_FIT)?;
    let first = inside(answer, base, entries, length).ok_or(DOES_NOT_FIT)?;
    for index in 0..count as usize {
        let entry = first + index * size_of::<Entry>();
        let field = |offset: usize| entry + offset;
        let name_length = u16_at(answer, field(offset_of!(Entry, name) + offset_of!(CountedName, length)))
            .ok_or(DOES_NOT_FIT)? as usize;
        let characters = word_at(
            answer,
            field(offset_of!(Entry, name) + offset_of!(CountedName, characters)),
        )
        .ok_or(DOES_NOT_FIT)?;
        if !name_length.is_multiple_of(2) {
            return Err(DOES_NOT_FIT);
        }
        // An empty name points nowhere; it is not the attribute either way.
        if name_length == 0 {
            continue;
        }
        let name = inside(answer, base, characters, name_length).ok_or(DOES_NOT_FIT)?;
        if !same_name(&answer[name..name + name_length], NO_ALL_APPLICATION_PACKAGES) {
            continue;
        }
        let value_type = u16_at(answer, field(offset_of!(Entry, value_type))).ok_or(DOES_NOT_FIT)?;
        let value_count = u32_at(answer, field(offset_of!(Entry, value_count))).ok_or(DOES_NOT_FIT)?;
        let values = word_at(answer, field(offset_of!(Entry, values))).ok_or(DOES_NOT_FIT)?;
        if value_type != TYPE_UINT64 || value_count == 0 {
            return Err(SandboxError::NotConfined(
                "the less-privileged attribute is not one set number",
            ));
        }
        let value = inside(answer, base, values, size_of::<u64>()).ok_or(DOES_NOT_FIT)?;
        return Ok(u64_at(answer, value).ok_or(DOES_NOT_FIT)? != 0);
    }
    Ok(false)
}

/// The offset of `length` bytes at address `pointer`, when they lie wholly inside the answer written at `base`.
fn inside(answer: &[u8], base: usize, pointer: usize, length: usize) -> Option<usize> {
    let offset = pointer.checked_sub(base)?;
    (offset.checked_add(length)? <= answer.len()).then_some(offset)
}

/// Whether the UTF-16 characters in `characters` are `name`, without regard to ASCII case.
fn same_name(characters: &[u8], name: &str) -> bool {
    let (units, odd) = characters.as_chunks::<2>();
    odd.is_empty()
        && units.len() == name.len()
        && units.iter().zip(name.bytes()).all(|(unit, wanted)| {
            u8::try_from(u16::from_ne_bytes(*unit)).is_ok_and(|byte| byte.eq_ignore_ascii_case(&wanted))
        })
}

fn bytes_at<const N: usize>(answer: &[u8], offset: usize) -> Option<[u8; N]> {
    answer.get(offset..offset.checked_add(N)?)?.try_into().ok()
}

fn u16_at(answer: &[u8], offset: usize) -> Option<u16> {
    bytes_at(answer, offset).map(u16::from_ne_bytes)
}

fn u32_at(answer: &[u8], offset: usize) -> Option<u32> {
    bytes_at(answer, offset).map(u32::from_ne_bytes)
}

fn u64_at(answer: &[u8], offset: usize) -> Option<u64> {
    bytes_at(answer, offset).map(u64::from_ne_bytes)
}

fn word_at(answer: &[u8], offset: usize) -> Option<usize> {
    bytes_at(answer, offset).map(usize::from_ne_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the made-up answers below say they were written. Any address works: the parse never follows a pointer,
    /// it only turns it into an offset.
    const BASE: usize = 0x0000_7ff0_1234_5000;

    /// One attribute of a made-up answer.
    struct Attribute {
        name: &'static str,
        value_type: u16,
        values: Vec<u64>,
    }

    fn uint64(name: &'static str, values: &[u64]) -> Attribute {
        Attribute {
            name,
            value_type: TYPE_UINT64,
            values: values.to_vec(),
        }
    }

    fn put(answer: &mut [u8], offset: usize, bytes: &[u8]) {
        answer[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    /// An answer laid out as the system lays it out: the header, the entries, then each name and its values.
    fn answer(attributes: &[Attribute]) -> Vec<u8> {
        let entries = size_of::<Header>();
        let mut tail = entries + attributes.len() * size_of::<Entry>();
        let mut answer = vec![0u8; tail];
        put(&mut answer, offset_of!(Header, version), &VERSION_1.to_ne_bytes());
        put(
            &mut answer,
            offset_of!(Header, count),
            &(attributes.len() as u32).to_ne_bytes(),
        );
        put(
            &mut answer,
            offset_of!(Header, entries),
            &(BASE + entries).to_ne_bytes(),
        );
        for (index, attribute) in attributes.iter().enumerate() {
            let entry = entries + index * size_of::<Entry>();
            let name: Vec<u8> = attribute.name.encode_utf16().flat_map(u16::to_ne_bytes).collect();
            let values: Vec<u8> = attribute.values.iter().flat_map(|value| value.to_ne_bytes()).collect();
            let (name_at, values_at) = (tail, tail + name.len().next_multiple_of(8));
            tail = values_at + values.len();
            answer.resize(tail, 0);
            put(&mut answer, name_at, &name);
            put(&mut answer, values_at, &values);
            let field = |offset: usize| entry + offset;
            put(
                &mut answer,
                field(offset_of!(Entry, name) + offset_of!(CountedName, length)),
                &(name.len() as u16).to_ne_bytes(),
            );
            put(
                &mut answer,
                field(offset_of!(Entry, name) + offset_of!(CountedName, maximum)),
                &(name.len() as u16).to_ne_bytes(),
            );
            put(
                &mut answer,
                field(offset_of!(Entry, name) + offset_of!(CountedName, characters)),
                &(BASE + name_at).to_ne_bytes(),
            );
            put(
                &mut answer,
                field(offset_of!(Entry, value_type)),
                &attribute.value_type.to_ne_bytes(),
            );
            put(
                &mut answer,
                field(offset_of!(Entry, value_count)),
                &(attribute.values.len() as u32).to_ne_bytes(),
            );
            put(
                &mut answer,
                field(offset_of!(Entry, values)),
                &(BASE + values_at).to_ne_bytes(),
            );
        }
        answer
    }

    /// The layout is the system's on a 64-bit Windows: a 16-byte header, 40-byte entries whose name is a counted
    /// string (length, room, then the pointer at 8), the value type at 16, the value count at 24, the values at 32.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn the_layout_is_the_systems() {
        assert_eq!((size_of::<Header>(), offset_of!(Header, entries)), (16, 8));
        assert_eq!(offset_of!(CountedName, characters), 8);
        assert_eq!(size_of::<Entry>(), 40);
        assert_eq!(
            [
                offset_of!(Entry, value_type),
                offset_of!(Entry, value_count),
                offset_of!(Entry, values)
            ],
            [16, 24, 32]
        );
    }

    /// The attribute set to 1 among the others a container token carries: less privileged.
    #[test]
    fn the_set_attribute_among_others_is_less_privileged() {
        let attributes = [
            Attribute {
                name: "WIN://SYSAPPID",
                value_type: 3,
                values: vec![0],
            },
            uint64("TSA://ProcUnique", &[7, 9]),
            uint64(NO_ALL_APPLICATION_PACKAGES, &[1]),
        ];
        assert_eq!(less_privileged(&answer(&attributes), BASE), Ok(true));
    }

    /// The name is compared without regard to ASCII case; any other name is not the attribute.
    #[test]
    fn only_that_name_counts() {
        assert_eq!(
            less_privileged(&answer(&[uint64("win://noallapppkg", &[1])]), BASE),
            Ok(true)
        );
        for other in [
            "WIN://NOALLAPPPK",
            "WIN://NOALLAPPPKGX",
            "WIN://PKG",
            "TSA://ProcUnique",
            "WIN://NOALLAPPPKG\0",
        ] {
            assert_eq!(
                less_privileged(&answer(&[uint64(other, &[1])]), BASE),
                Ok(false),
                "{other:?}"
            );
        }
    }

    /// No attribute, no attributes at all, or the attribute set to zero: not less privileged.
    #[test]
    fn a_token_without_the_set_attribute_is_not_less_privileged() {
        assert_eq!(less_privileged(&answer(&[]), BASE), Ok(false));
        assert_eq!(
            less_privileged(&answer(&[uint64("TSA://ProcUnique", &[7, 9])]), BASE),
            Ok(false)
        );
        assert_eq!(
            less_privileged(&answer(&[uint64(NO_ALL_APPLICATION_PACKAGES, &[0])]), BASE),
            Ok(false)
        );
    }

    /// The attribute with another value type or without a value is refused, not read as set.
    #[test]
    fn the_attribute_that_is_not_one_number_is_refused() {
        let refused = Err(SandboxError::NotConfined(
            "the less-privileged attribute is not one set number",
        ));
        let string = Attribute {
            name: NO_ALL_APPLICATION_PACKAGES,
            value_type: 3,
            values: vec![1],
        };
        assert_eq!(less_privileged(&answer(&[string]), BASE), refused);
        assert_eq!(
            less_privileged(&answer(&[uint64(NO_ALL_APPLICATION_PACKAGES, &[])]), BASE),
            refused
        );
    }

    /// Every pointer, count and length that leaves the buffer refuses the helper; nothing outside it is read.
    #[test]
    fn an_answer_that_does_not_fit_is_refused() {
        let good = answer(&[uint64(NO_ALL_APPLICATION_PACKAGES, &[1])]);
        let entry = size_of::<Header>();
        let name = entry + offset_of!(Entry, name);
        let cases: Vec<(&str, Vec<u8>, usize)> = vec![
            ("empty", Vec::new(), BASE),
            ("header cut short", good[..size_of::<Header>() - 1].to_vec(), BASE),
            ("value cut short", good[..good.len() - 1].to_vec(), BASE),
            ("written elsewhere", good.clone(), BASE + 8),
            ("written higher", good.clone(), BASE - 0x1000_0000),
            (
                "count past the end",
                with(&good, offset_of!(Header, count), &3u32.to_ne_bytes()),
                BASE,
            ),
            (
                "count overflows",
                with(&good, offset_of!(Header, count), &u32::MAX.to_ne_bytes()),
                BASE,
            ),
            (
                "entries before the buffer",
                with(&good, offset_of!(Header, entries), &(BASE - 8).to_ne_bytes()),
                BASE,
            ),
            (
                "entries at the top",
                with(&good, offset_of!(Header, entries), &usize::MAX.to_ne_bytes()),
                BASE,
            ),
            (
                "odd name length",
                with(&good, name + offset_of!(CountedName, length), &33u16.to_ne_bytes()),
                BASE,
            ),
            (
                "name past the end",
                with(&good, name + offset_of!(CountedName, length), &0xfffeu16.to_ne_bytes()),
                BASE,
            ),
            (
                "name elsewhere",
                with(&good, name + offset_of!(CountedName, characters), &0usize.to_ne_bytes()),
                BASE,
            ),
            (
                "values elsewhere",
                with(&good, entry + offset_of!(Entry, values), &usize::MAX.to_ne_bytes()),
                BASE,
            ),
        ];
        for (case, bytes, base) in cases {
            assert!(
                matches!(less_privileged(&bytes, base), Err(SandboxError::NotConfined(_))),
                "{case}"
            );
        }
    }

    /// An answer of another version is refused.
    #[test]
    fn another_version_is_refused() {
        let good = answer(&[uint64(NO_ALL_APPLICATION_PACKAGES, &[1])]);
        assert_eq!(
            less_privileged(&with(&good, offset_of!(Header, version), &2u16.to_ne_bytes()), BASE),
            Err(SandboxError::NotConfined(
                "the security-attribute read-back has an unknown version"
            ))
        );
    }

    fn with(answer: &[u8], offset: usize, bytes: &[u8]) -> Vec<u8> {
        let mut changed = answer.to_vec();
        put(&mut changed, offset, bytes);
        changed
    }
}
