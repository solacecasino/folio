//! Spell checking with the macOS spell checker, so Folio uses the same
//! dictionaries, languages and learned words as every other Mac app.

use std::collections::HashMap;
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
use std::ops::Range;

use objc2::rc::Retained;
use objc2_app_kit::NSSpellChecker;
use objc2_foundation::{NSRange, NSString};

pub struct Speller {
    checker: Retained<NSSpellChecker>,
    /// Folio's own "spell document", so Ignore only lasts for this session.
    tag: isize,
    language: Option<Retained<NSString>>,
    /// Misspelled character ranges by paragraph text.
    cache: HashMap<u64, Vec<Range<usize>>>,
}

impl Speller {
    /// `language` is a code like "en" or "fr"; empty means the system language.
    pub fn new(language: &str) -> Self {
        Self {
            checker: NSSpellChecker::sharedSpellChecker(),
            tag: NSSpellChecker::uniqueSpellDocumentTag(),
            language: (!language.is_empty()).then(|| NSString::from_str(language)),
            cache: HashMap::new(),
        }
    }

    /// Misspelled words in a paragraph, as character ranges. Cached, so
    /// calling it every frame only does work when the text changes.
    pub fn misspellings(&mut self, text: &[char]) -> &[Range<usize>] {
        let key = BuildHasherDefault::<DefaultHasher>::default().hash_one(text);
        if self.cache.len() > 20_000 {
            self.cache.clear();
        }
        if !self.cache.contains_key(&key) {
            let found = self.check(text);
            self.cache.insert(key, found);
        }
        &self.cache[&key]
    }

    fn check(&self, text: &[char]) -> Vec<Range<usize>> {
        if text.iter().all(|c| !c.is_alphabetic()) {
            return Vec::new();
        }
        let string: String = text.iter().collect();
        let ns = NSString::from_str(&string);
        let utf16 = Utf16Map::new(text);
        let mut found = Vec::new();
        let mut offset = 0usize;
        while offset < utf16.len() {
            // SAFETY: a null word-count pointer is allowed.
            let r: NSRange = unsafe {
                self.checker.checkSpellingOfString_startingAt_language_wrap_inSpellDocumentWithTag_wordCount(
                    &ns,
                    offset as isize,
                    self.language.as_deref(),
                    false,
                    self.tag,
                    std::ptr::null_mut(),
                )
            };
            // NSNotFound (isize::MAX) means no more misspellings.
            if r.length == 0 || r.location >= isize::MAX as usize {
                break;
            }
            found.push(utf16.to_chars(r.location)..utf16.to_chars(r.location + r.length));
            offset = r.location + r.length;
        }
        found
    }

    /// Spelling suggestions for a misspelled range, best first.
    pub fn suggestions(&self, text: &[char], range: Range<usize>) -> Vec<String> {
        let string: String = text.iter().collect();
        let ns = NSString::from_str(&string);
        let utf16 = Utf16Map::new(text);
        let start = utf16.to_utf16(range.start);
        let r = NSRange::new(start, utf16.to_utf16(range.end) - start);
        self.checker
            .guessesForWordRange_inString_language_inSpellDocumentWithTag(r, &ns, self.language.as_deref(), self.tag)
            .map(|guesses| guesses.iter().map(|g| g.to_string()).take(6).collect())
            .unwrap_or_default()
    }

    /// Adds a word to the user's macOS dictionary (shared with other apps).
    pub fn learn(&mut self, word: &str) {
        self.checker.learnWord(&NSString::from_str(word));
        self.cache.clear();
    }

    /// Stops flagging a word until Folio quits.
    pub fn ignore(&mut self, word: &str) {
        self.checker.ignoreWord_inSpellDocumentWithTag(&NSString::from_str(word), self.tag);
        self.cache.clear();
    }
}

/// Converts between macOS's UTF-16 offsets and Folio's character indexes.
struct Utf16Map {
    /// UTF-16 offset where each character starts, plus the total at the end.
    starts: Vec<usize>,
}

impl Utf16Map {
    fn new(text: &[char]) -> Self {
        let mut starts = Vec::with_capacity(text.len() + 1);
        let mut at = 0;
        for c in text {
            starts.push(at);
            at += c.len_utf16();
        }
        starts.push(at);
        Self { starts }
    }

    fn len(&self) -> usize {
        self.starts[self.starts.len() - 1]
    }

    fn to_chars(&self, utf16: usize) -> usize {
        self.starts.partition_point(|&s| s < utf16)
    }

    fn to_utf16(&self, chars: usize) -> usize {
        self.starts[chars.min(self.starts.len() - 1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_map_to_characters() {
        let text: Vec<char> = "a😀b".chars().collect();
        let map = Utf16Map::new(&text);
        assert_eq!(map.len(), 4, "the emoji is two UTF-16 units");
        assert_eq!(map.to_chars(3), 2);
        assert_eq!(map.to_utf16(2), 3);
    }

    #[test]
    fn finds_misspellings_and_suggests_fixes() {
        let mut speller = Speller::new("en");
        let text: Vec<char> = "This sentnce has a typo in the 😀 middle.".chars().collect();
        let found = speller.misspellings(&text).to_vec();
        assert_eq!(found.len(), 1, "{found:?}");
        let word: String = text[found[0].clone()].iter().collect();
        assert_eq!(word, "sentnce");
        assert!(speller.suggestions(&text, found[0].clone()).iter().any(|s| s == "sentence"));

        speller.ignore("sentnce");
        assert!(speller.misspellings(&text).is_empty(), "ignored for this session");
        let clean: Vec<char> = "Nothing wrong here.".chars().collect();
        assert!(speller.misspellings(&clean).is_empty());
    }
}
