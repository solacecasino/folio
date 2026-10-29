//! Spell checking with the system's own spell checker where there is one,
//! so text9000 uses the same dictionaries, languages and learned words as the
//! other apps on the computer:
//!
//! - macOS: the system spell checker (NSSpellChecker).
//! - Windows: the Windows spell checker (Settings → Time & language).
//! - Linux: Hunspell dictionaries, from Folio's `dictionaries/` folder or the
//!   system's (`/usr/share/hunspell`). Learned words go to
//!   `dictionaries/personal.txt`.

use std::collections::HashMap;
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
use std::ops::Range;

pub struct Speller {
    backend: backend::Backend,
    /// Misspelled character ranges by paragraph text.
    cache: HashMap<u64, Vec<Range<usize>>>,
}

impl Speller {
    /// `language` is a code like "en" or "fr"; empty means the system language.
    pub fn new(language: &str) -> Self {
        Self {
            backend: backend::Backend::new(language),
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
            let found = if text.iter().all(|c| !c.is_alphabetic()) {
                Vec::new()
            } else {
                self.backend.check(text)
            };
            self.cache.insert(key, found);
        }
        &self.cache[&key]
    }

    /// Spelling suggestions for a misspelled range, best first.
    pub fn suggestions(&self, text: &[char], range: Range<usize>) -> Vec<String> {
        self.backend.suggestions(text, range)
    }

    /// Adds a word to the user's dictionary (shared with other apps on
    /// macOS and Windows).
    pub fn learn(&mut self, word: &str) {
        self.backend.learn(word);
        self.cache.clear();
    }

    /// Stops flagging a word until text9000 quits.
    pub fn ignore(&mut self, word: &str) {
        self.backend.ignore(word);
        self.cache.clear();
    }
}

#[cfg(target_os = "macos")]
mod backend {
    use std::ops::Range;

    use objc2::rc::Retained;
    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSRange, NSString};

    use super::Utf16Map;

    pub struct Backend {
        checker: Retained<NSSpellChecker>,
        /// Folio's own "spell document", so Ignore only lasts for this session.
        tag: isize,
        language: Option<Retained<NSString>>,
    }

    impl Backend {
        pub fn new(language: &str) -> Self {
            Self {
                checker: NSSpellChecker::sharedSpellChecker(),
                tag: NSSpellChecker::uniqueSpellDocumentTag(),
                language: (!language.is_empty()).then(|| NSString::from_str(language)),
            }
        }

        pub fn check(&self, text: &[char]) -> Vec<Range<usize>> {
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

        pub fn learn(&mut self, word: &str) {
            self.checker.learnWord(&NSString::from_str(word));
        }

        pub fn ignore(&mut self, word: &str) {
            self.checker.ignoreWord_inSpellDocumentWithTag(&NSString::from_str(word), self.tag);
        }
    }
}

#[cfg(windows)]
mod backend {
    use std::ops::Range;

    use windows::Win32::Foundation::S_OK;
    use windows::Win32::Globalization::{GetUserDefaultLocaleName, ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory};
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};
    use windows::core::{HSTRING, PCWSTR, PWSTR};

    use super::Utf16Map;

    pub struct Backend {
        /// None when Windows has no spell checker for the language.
        checker: Option<ISpellChecker>,
    }

    impl Backend {
        pub fn new(language: &str) -> Self {
            // SAFETY: plain COM calls; the factory is only used here.
            let checker = unsafe {
                // Already initialized by the windowing code, usually; that's fine.
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                CoCreateInstance::<_, ISpellCheckerFactory>(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER)
                    .ok()
                    .and_then(|factory| {
                        candidates(language).into_iter().find_map(|tag| {
                            let tag = HSTRING::from(tag);
                            if !factory.IsSupported(&tag).ok()?.as_bool() {
                                return None;
                            }
                            factory.CreateSpellChecker(&tag).ok()
                        })
                    })
            };
            if checker.is_none() {
                eprintln!("text9000: no Windows spell checker for \"{language}\"");
            }
            Self { checker }
        }

        pub fn check(&self, text: &[char]) -> Vec<Range<usize>> {
            let Some(checker) = &self.checker else { return Vec::new() };
            let utf16 = Utf16Map::new(text);
            let wide: Vec<u16> = text.iter().collect::<String>().encode_utf16().chain([0]).collect();
            let mut found = Vec::new();
            // SAFETY: `wide` is NUL-terminated and outlives the call.
            unsafe {
                let Ok(errors) = checker.Check(PCWSTR(wide.as_ptr())) else { return found };
                loop {
                    let mut error = None;
                    if errors.Next(&mut error) != S_OK {
                        break;
                    }
                    let Some(error) = error else { break };
                    let (Ok(start), Ok(len)) = (error.StartIndex(), error.Length()) else { continue };
                    let (start, len) = (start as usize, len as usize);
                    found.push(utf16.to_chars(start)..utf16.to_chars(start + len));
                }
            }
            found
        }

        pub fn suggestions(&self, text: &[char], range: Range<usize>) -> Vec<String> {
            let Some(checker) = &self.checker else { return Vec::new() };
            let word: String = text[range].iter().collect();
            let mut out = Vec::new();
            // SAFETY: each string Windows hands back is read once, then freed.
            unsafe {
                let Ok(list) = checker.Suggest(&HSTRING::from(word)) else { return out };
                while out.len() < 6 {
                    let mut item = [PWSTR::null()];
                    let mut fetched = 0u32;
                    if list.Next(&mut item, Some(&mut fetched)) != S_OK || fetched == 0 {
                        break;
                    }
                    if let Ok(s) = item[0].to_string() {
                        out.push(s);
                    }
                    CoTaskMemFree(Some(item[0].0 as *const _));
                }
            }
            out
        }

        pub fn learn(&mut self, word: &str) {
            if let Some(checker) = &self.checker {
                // SAFETY: a plain COM call with a valid string.
                let _ = unsafe { checker.Add(&HSTRING::from(word)) };
            }
        }

        pub fn ignore(&mut self, word: &str) {
            if let Some(checker) = &self.checker {
                // SAFETY: a plain COM call with a valid string.
                let _ = unsafe { checker.Ignore(&HSTRING::from(word)) };
            }
        }
    }

    /// Language tags to try: "en" becomes "en", "en-US", ...; empty means
    /// the user's Windows language.
    fn candidates(language: &str) -> Vec<String> {
        let mut out = Vec::new();
        if language.is_empty() {
            let mut buf = [0u16; 85];
            // SAFETY: the buffer is the size Windows asks for (LOCALE_NAME_MAX_LENGTH).
            let n = unsafe { GetUserDefaultLocaleName(&mut buf) };
            if n > 1 {
                out.push(String::from_utf16_lossy(&buf[..n as usize - 1]));
            }
        } else {
            let tag = language.replace('_', "-");
            out.push(tag.clone());
            if !tag.contains('-') {
                out.push(format!("{tag}-{}", tag.to_uppercase()));
                if tag == "en" {
                    out.extend(["en-US".into(), "en-GB".into()]);
                }
            }
        }
        out.push("en-US".into());
        out
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod backend {
    use std::collections::HashSet;
    use std::io::Write;
    use std::ops::Range;
    use std::path::{Path, PathBuf};

    use spellbook::Dictionary;

    pub struct Backend {
        /// None when no dictionary for the language was found.
        dict: Option<Dictionary>,
        ignored: HashSet<String>,
    }

    fn personal_path() -> PathBuf {
        crate::settings::config_dir().join("dictionaries/personal.txt")
    }

    impl Backend {
        pub fn new(language: &str) -> Self {
            let dict = find_dictionary(language).and_then(|(aff, dic)| {
                let aff = std::fs::read_to_string(aff).ok()?;
                let dic = std::fs::read_to_string(dic).ok()?;
                Dictionary::new(&aff, &dic).map_err(|e| eprintln!("text9000: dictionary: {e}")).ok()
            });
            let dict = dict.map(|mut d| {
                for word in std::fs::read_to_string(personal_path()).unwrap_or_default().lines() {
                    let _ = d.add(word.trim());
                }
                d
            });
            if dict.is_none() {
                eprintln!(
                    "text9000: no Hunspell dictionary for \"{language}\"; install one (e.g. hunspell-en-us) \
                     or put <lang>.aff and <lang>.dic in {}",
                    crate::settings::config_dir().join("dictionaries").display()
                );
            }
            Self {
                dict,
                ignored: HashSet::new(),
            }
        }

        pub fn check(&self, text: &[char]) -> Vec<Range<usize>> {
            let Some(dict) = &self.dict else { return Vec::new() };
            words(text)
                .filter(|r| {
                    let word = normalize(&text[r.clone()]);
                    !self.ignored.contains(&word) && !dict.check(&word)
                })
                .collect()
        }

        pub fn suggestions(&self, text: &[char], range: Range<usize>) -> Vec<String> {
            let Some(dict) = &self.dict else { return Vec::new() };
            let mut out = Vec::new();
            dict.suggest(&normalize(&text[range]), &mut out);
            out.truncate(6);
            out
        }

        pub fn learn(&mut self, word: &str) {
            if let Some(dict) = &mut self.dict {
                let _ = dict.add(word);
            }
            let path = personal_path();
            let _ = std::fs::create_dir_all(path.parent().expect("has parent"));
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{word}");
            }
        }

        pub fn ignore(&mut self, word: &str) {
            self.ignored.insert(word.replace('’', "'"));
        }
    }

    fn normalize(word: &[char]) -> String {
        word.iter().map(|&c| if c == '’' { '\'' } else { c }).collect()
    }

    /// Words as character ranges: runs of letters, with apostrophes inside
    /// a word ("don't") kept.
    pub(super) fn words(text: &[char]) -> impl Iterator<Item = Range<usize>> + '_ {
        let mut i = 0;
        std::iter::from_fn(move || {
            while i < text.len() && !text[i].is_alphabetic() {
                i += 1;
            }
            if i >= text.len() {
                return None;
            }
            let start = i;
            while i < text.len()
                && (text[i].is_alphabetic()
                    || (matches!(text[i], '\'' | '’') && text.get(i + 1).is_some_and(|c| c.is_alphabetic())))
            {
                i += 1;
            }
            Some(start..i)
        })
    }

    /// The .aff and .dic files for a language like "en" or "en_GB"; empty
    /// means the language from $LANG.
    fn find_dictionary(language: &str) -> Option<(PathBuf, PathBuf)> {
        let wanted = if language.is_empty() {
            ["LC_ALL", "LC_MESSAGES", "LANG"]
                .iter()
                .filter_map(|v| std::env::var(v).ok())
                .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
                .map(|v| v.split(['.', '@']).next().unwrap_or("").to_owned())
                .unwrap_or_else(|| "en_US".into())
        } else {
            language.replace('-', "_")
        };
        let dirs = [
            crate::settings::config_dir().join("dictionaries"),
            "/usr/share/hunspell".into(),
            "/usr/share/myspell".into(),
            "/usr/share/myspell/dicts".into(),
        ];
        let pair = |dir: &Path, name: &str| {
            let (aff, dic) = (dir.join(format!("{name}.aff")), dir.join(format!("{name}.dic")));
            (aff.is_file() && dic.is_file()).then_some((aff, dic))
        };
        let base = wanted.split('_').next().unwrap_or("en").to_owned();
        // The exact language, then the same language anywhere ("en" → en_US), then English.
        for name in [wanted.as_str(), &format!("{base}_{}", base.to_uppercase()), "en_US"] {
            if let Some(found) = dirs.iter().find_map(|d| pair(d, name)) {
                return Some(found);
            }
        }
        dirs.iter().find_map(|d| {
            let mut names: Vec<String> = std::fs::read_dir(d)
                .ok()?
                .flatten()
                .filter_map(|e| e.file_name().to_str()?.strip_suffix(".dic").map(str::to_owned))
                .filter(|n| n.starts_with(&format!("{base}_")))
                .collect();
            names.sort();
            names.iter().find_map(|n| pair(d, n))
        })
    }
}

/// Converts between the system's UTF-16 offsets and Folio's character indexes.
// Linux checks words one at a time and doesn't need UTF-16 offsets.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct Utf16Map {
    /// UTF-16 offset where each character starts, plus the total at the end.
    starts: Vec<usize>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn splits_words_keeping_apostrophes() {
        let text: Vec<char> = "Don’t stop—it's 2am, 'quoted'.".chars().collect();
        let words: Vec<String> = backend::words(&text).map(|r| text[r].iter().collect()).collect();
        assert_eq!(words, ["Don’t", "stop", "it's", "am", "quoted"]);
    }
}
