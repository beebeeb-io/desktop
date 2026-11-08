//! DW-A24: which of the typed recovery words are not in the 2048-word list. Only such a word is marked (a valid word
//! in the wrong place gets no mark). The words go one way, from the web view to Rust, and nothing but their positions
//! comes back; the restore itself is `desktop_unlock_with_recovery_phrase`.

/// Positions (0-based) of the words that are not BIP-39 English words. Empty words are not marked.
pub(crate) fn unknown_positions(words: &[String]) -> Vec<u8> {
    words
        .iter()
        .enumerate()
        .filter(|(_, w)| {
            let w = w.trim().to_lowercase();
            !w.is_empty() && bip39::Language::English.find_word(&w).is_none()
        })
        .map(|(i, _)| i as u8)
        .collect()
}

#[tauri::command]
pub(crate) fn auth_phrase_unknown_words(mut words: Vec<String>) -> Vec<u8> {
    let out = unknown_positions(&words);
    for w in &mut words {
        zeroize::Zeroize::zeroize(w);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_words_outside_the_list_are_marked() {
        let words: Vec<String> = ["abandon", "ability", "abel", "zoo", "", "Zoo"].iter().map(|s| s.to_string()).collect();
        assert_eq!(super::unknown_positions(&words), vec![2]);
    }
}
