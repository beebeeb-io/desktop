//! The helper's executable does not load Windows' media libraries when it starts. On a Windows N or KN edition without
//! the Media Feature Pack they do not exist, and an import would stop the helper from starting at all, for FilmCraft's
//! formats too. The system-decoder route loads them at run time (`src/mf/api.rs`) and answers `NoSystemMedia` when they
//! are missing. Read from the built executable's own import table.
#![cfg(windows)]

/// The libraries of the Media Feature Pack the helper uses, and the COM libraries behind them: ole32, combase and
/// oleaut32 are delay-loaded (`build.rs`); loaded at start they failed with 0xc0000142 on the build VM and the helper
/// never started.
const MEDIA_LIBRARIES: [&str; 6] = [
    "mfplat.dll",
    "mfreadwrite.dll",
    "mf.dll",
    "ole32.dll",
    "combase.dll",
    "oleaut32.dll",
];

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The names of the libraries in a PE32+ executable's import directory, lower case.
fn imported_libraries(image: &[u8]) -> Option<Vec<String>> {
    let pe = u32_at(image, 0x3c)? as usize;
    if image.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let coff = pe + 4;
    let sections = u16_at(image, coff + 2)? as usize;
    let optional = coff + 20;
    let optional_len = u16_at(image, coff + 16)? as usize;
    if u16_at(image, optional)? != 0x20b {
        return None; // not PE32+
    }
    // Data directory 1 (imports), after the 112 bytes of the PE32+ optional header's fixed part.
    let imports_rva = u32_at(image, optional + 112 + 8)? as usize;
    let table = optional + optional_len;
    let to_offset = |rva: usize| -> Option<usize> {
        (0..sections).find_map(|i| {
            let s = table + i * 40;
            let virtual_size = u32_at(image, s + 8)? as usize;
            let address = u32_at(image, s + 12)? as usize;
            let raw_size = u32_at(image, s + 16)? as usize;
            let raw = u32_at(image, s + 20)? as usize;
            let span = virtual_size.max(raw_size);
            (address <= rva && rva < address + span).then(|| rva - address + raw)
        })
    };
    let mut names = Vec::new();
    let mut entry = to_offset(imports_rva)?;
    loop {
        // An import descriptor is 20 bytes; the name's address is at +12; an all-zero one ends the list.
        let name_rva = u32_at(image, entry + 12)? as usize;
        if name_rva == 0 {
            break;
        }
        let at = to_offset(name_rva)?;
        let end = at + image.get(at..)?.iter().position(|&b| b == 0)?;
        names.push(String::from_utf8_lossy(&image[at..end]).to_ascii_lowercase());
        entry += 20;
        if names.len() > 512 {
            return None;
        }
    }
    Some(names)
}

#[test]
fn the_helper_does_not_load_windows_media_libraries_at_start() {
    let image = std::fs::read(env!("CARGO_BIN_EXE_beebeeb-media-helper")).expect("the helper was built");
    let names = imported_libraries(&image).expect("a PE32+ import table");
    assert!(
        names.iter().any(|name| name == "kernel32.dll"),
        "the instrument reads the import table: {names:?}"
    );
    for library in MEDIA_LIBRARIES {
        assert!(
            !names.iter().any(|name| name == library),
            "{library} is imported at start: {names:?}"
        );
    }
}
