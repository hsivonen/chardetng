use encoding_rs::Decoder;
use encoding_rs::DecoderResult;
use encoding_rs::Encoding;
use encoding_rs::BIG5;
use encoding_rs::EUC_JP;
use encoding_rs::EUC_KR;
use encoding_rs::GBK;
use encoding_rs::IBM866;
use encoding_rs::ISO_2022_JP;
use encoding_rs::ISO_8859_2;
use encoding_rs::ISO_8859_4;
use encoding_rs::ISO_8859_5;
use encoding_rs::ISO_8859_6;
use encoding_rs::ISO_8859_7;
use encoding_rs::ISO_8859_8;
use encoding_rs::KOI8_U;
use encoding_rs::SHIFT_JIS;
use encoding_rs::UTF_8;
use encoding_rs::WINDOWS_1250;
use encoding_rs::WINDOWS_1251;
use encoding_rs::WINDOWS_1252;
use encoding_rs::WINDOWS_1253;
use encoding_rs::WINDOWS_1255;
use encoding_rs::WINDOWS_1256;
use encoding_rs::WINDOWS_1257;
use encoding_rs::WINDOWS_874;

mod data;
use data::*;

const LATIN_ADJACENCY_PENALTY: i64 = -40;

const IMPLAUSIBILITY_PENALTY: i64 = -220;

const IMPLAUSIBLE_LATIN_CASE_TRANSITION_PENALTY: i64 = -100;

const NON_LATIN_CAPITALIZATION_BONUS: i64 = 40;

const NON_LATIN_ALL_CAPS_PENALTY: i64 = -40;

// XXX rework how this gets applied
const NON_LATIN_MIXED_CASE_PENALTY: i64 = -20;

// XXX Remove this
const NON_LATIN_CAMEL_PENALTY: i64 = -80;

const NON_LATIN_IMPLAUSIBLE_CASE_TRANSITION_PENALTY: i64 = -100;

const SHIFT_JIS_SCORE_PER_KANA_KANJI: i64 = 20;

const EUC_JP_SCORE_PER_KANA: i64 = 20;

const EUC_JP_SCORE_PER_KANJI: i64 = 20;

const BIG5_SCORE_PER_HANZI: i64 = 20;

const EUC_KR_SCORE_PER_MODERN_HANGUL: i64 = 20;

const GBK_SCORE_PER_EUC: i64 = 20;

const GBK_SCORE_PER_NON_EUC: i64 = 5;

/// Latin letter caseless class
const LATIN_LETTER: u8 = 2;

/// ASCII punctionation caseless class for Hebrew
const ASCII_PUNCTUATION: u8 = 3;

// For Latin, we only penalize pairwise bad transitions
// if one participant is non-ASCII. This avoids violating
// the principle that ASCII pairs never contribute to the
// score. (Maybe that's a bad principle, though!)
#[derive(PartialEq)]
enum LatinCaseState {
    Space,
    Upper,
    Lower,
    AllCaps,
}

// Fon non-Latin, we calculate case-related penalty
// or bonus on a per-non-Latin-word basis.
#[derive(PartialEq)]
enum NonLatinCaseState {
    Space,
    Upper,
    Lower,
    UpperLower,
    LowerUpper,
    LowerUpperUpper,
    LowerUpperLower,
    UpperLowerCamel, // State like UpperLower but has been through something else
    AllCaps,
    Mix,
}

struct NonLatinCasedCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
    case_state: NonLatinCaseState,
    prev_ascii: bool,
    current_word_len: u64,
    longest_word: u64,
}

impl NonLatinCasedCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        NonLatinCasedCandidate {
            data: data,
            score: 0,
            prev: 0,
            case_state: NonLatinCaseState::Space,
            prev_ascii: true,
            current_word_len: 0,
            longest_word: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> bool {
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return true;
            }
            let caseless_class = class & 0x7F;

            let ascii = b < 0x80;
            let ascii_pair = self.prev_ascii && ascii;

            let non_ascii_alphabetic = self.data.is_non_latin_alphabetic(caseless_class);

            // The purpose of this state machine is to avoid misdetecting Greek as
            // Cyrillic by:
            //
            // * Giving a small bonus to words that start with an upper-case letter
            //   and are lower-case for the rest.
            // * Giving a large penalty to start with one lower-case letter followed
            //   by all upper-case (obviously upper and lower case inverted, which
            //   unfortunately is possible due to KOI8-U).
            // * Giving a small per-word penalty to all-uppercase KOI8-U (to favor
            //   all-lowercase Greek over all-caps KOI8-U).
            // * Giving large penalties for random mixed-case while making the
            //   penalties for CamelCase recoverable. Going easy on CamelCase
            //   might not actually be necessary.

            // ASCII doesn't participate in non-Latin casing.
            if caseless_class == LATIN_LETTER {
                // Latin
                // Mark this word as a mess. If there end up being non-Latin
                // letters in this word, the ASCII-adjacency penalty gets
                // applied to Latin/non-Latin pairs and the mix penalty
                // to non-Latin/non-Latin pairs.
                self.case_state = NonLatinCaseState::Mix;
            } else if !non_ascii_alphabetic {
                // Space
                match self.case_state {
                    NonLatinCaseState::Space
                    | NonLatinCaseState::Upper
                    | NonLatinCaseState::Lower
                    | NonLatinCaseState::UpperLowerCamel => {}
                    NonLatinCaseState::UpperLower => {
                        // Intentionally applied only once per word.
                        self.score += NON_LATIN_CAPITALIZATION_BONUS;
                    }
                    NonLatinCaseState::LowerUpper => {
                        // Once per word
                        self.score = NON_LATIN_IMPLAUSIBLE_CASE_TRANSITION_PENALTY;
                    }
                    NonLatinCaseState::LowerUpperLower => {
                        // Once per word
                        self.score += NON_LATIN_CAMEL_PENALTY;
                    }
                    NonLatinCaseState::AllCaps => {
                        // Intentionally applied only once per word.
                        if self.data == &SINGLE_BYTE_DATA[KOI8_U_INDEX] {
                            // Apply only to KOI8-U.
                            self.score += NON_LATIN_ALL_CAPS_PENALTY;
                        }
                    }
                    NonLatinCaseState::Mix | NonLatinCaseState::LowerUpperUpper => {
                        // Per letter
                        self.score += NON_LATIN_MIXED_CASE_PENALTY;
                    }
                }
                self.case_state = NonLatinCaseState::Space;
            } else if (class >> 7) == 0 {
                // Lower case
                match self.case_state {
                    NonLatinCaseState::Space => {
                        self.case_state = NonLatinCaseState::Lower;
                    }
                    NonLatinCaseState::Upper => {
                        self.case_state = NonLatinCaseState::UpperLower;
                    }
                    NonLatinCaseState::Lower
                    | NonLatinCaseState::UpperLower
                    | NonLatinCaseState::UpperLowerCamel => {}
                    NonLatinCaseState::LowerUpper => {
                        self.score += NON_LATIN_CAMEL_PENALTY;
                        self.case_state = NonLatinCaseState::LowerUpperLower;
                    }
                    NonLatinCaseState::LowerUpperUpper => {
                        self.score += NON_LATIN_MIXED_CASE_PENALTY;
                        self.case_state = NonLatinCaseState::Mix;
                    }
                    NonLatinCaseState::LowerUpperLower => {
                        self.score += NON_LATIN_CAMEL_PENALTY;
                        self.case_state = NonLatinCaseState::UpperLowerCamel;
                    }
                    NonLatinCaseState::AllCaps => {
                        self.score = NON_LATIN_IMPLAUSIBLE_CASE_TRANSITION_PENALTY;
                        self.case_state = NonLatinCaseState::Mix;
                    }
                    NonLatinCaseState::Mix => {
                        self.score += NON_LATIN_MIXED_CASE_PENALTY;
                    }
                }
            } else {
                // Upper case
                match self.case_state {
                    NonLatinCaseState::Space => {
                        self.case_state = NonLatinCaseState::Upper;
                    }
                    NonLatinCaseState::Upper => {
                        self.case_state = NonLatinCaseState::AllCaps;
                    }
                    NonLatinCaseState::Lower
                    | NonLatinCaseState::UpperLower
                    | NonLatinCaseState::UpperLowerCamel => {
                        // No penalty, yet. The next transition decides how much.
                        self.case_state = NonLatinCaseState::LowerUpper;
                    }
                    NonLatinCaseState::LowerUpper => {
                        // Once per word
                        self.score = NON_LATIN_IMPLAUSIBLE_CASE_TRANSITION_PENALTY;
                        self.case_state = NonLatinCaseState::LowerUpperUpper;
                    }
                    NonLatinCaseState::LowerUpperUpper | NonLatinCaseState::Mix => {
                        self.score += NON_LATIN_MIXED_CASE_PENALTY;
                    }
                    NonLatinCaseState::LowerUpperLower => {
                        self.score = NON_LATIN_IMPLAUSIBLE_CASE_TRANSITION_PENALTY;
                        self.case_state = NonLatinCaseState::Mix;
                    }
                    NonLatinCaseState::AllCaps => {}
                }
            }

            if non_ascii_alphabetic {
                self.current_word_len += 1;
            } else {
                if self.current_word_len > self.longest_word {
                    self.longest_word = self.current_word_len;
                }
                self.current_word_len = 0;
            }

            if !ascii_pair {
                self.score += self.data.score(caseless_class, self.prev);

                if self.prev == LATIN_LETTER && non_ascii_alphabetic {
                    self.score += LATIN_ADJACENCY_PENALTY;
                } else if caseless_class == LATIN_LETTER
                    && self.data.is_non_latin_alphabetic(self.prev)
                {
                    self.score += LATIN_ADJACENCY_PENALTY;
                }
            }

            self.prev_ascii = ascii;
            self.prev = caseless_class;
        }
        false
    }
}

struct LatinCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
    case_state: LatinCaseState,
    prev_non_ascii: u32,
}

impl LatinCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        LatinCandidate {
            data: data,
            score: 0,
            prev: 0,
            case_state: LatinCaseState::Space,
            prev_non_ascii: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> bool {
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return true;
            }
            let caseless_class = class & 0x7F;

            let ascii = b < 0x80;
            let ascii_pair = self.prev_non_ascii == 0 && ascii;

            let non_ascii_penalty = match self.prev_non_ascii {
                0 | 1 | 2 => 0,
                3 => -5,
                4 => -20,
                _ => -200,
            };
            self.score += non_ascii_penalty;

            if !self.data.is_latin_alphabetic(caseless_class) {
                self.case_state = LatinCaseState::Space;
            } else if (class >> 7) == 0 {
                // Penalizing lower case after two upper case
                // is important for avoiding misdetecting
                // windows-1250 as windows-1252 (byte 0x9F).
                if self.case_state == LatinCaseState::AllCaps && !ascii_pair {
                    self.score += IMPLAUSIBLE_LATIN_CASE_TRANSITION_PENALTY;
                }
                self.case_state = LatinCaseState::Lower;
            } else {
                match self.case_state {
                    LatinCaseState::Space => {
                        self.case_state = LatinCaseState::Upper;
                    }
                    LatinCaseState::Upper | LatinCaseState::AllCaps => {
                        self.case_state = LatinCaseState::AllCaps;
                    }
                    LatinCaseState::Lower => {
                        if !ascii_pair {
                            // XXX How bad is this for Irish Gaelic?
                            self.score += IMPLAUSIBLE_LATIN_CASE_TRANSITION_PENALTY;
                        }
                        self.case_state = LatinCaseState::Upper;
                    }
                }
            }

            if !ascii_pair {
                self.score += self.data.score(caseless_class, self.prev);
            }

            if ascii {
                self.prev_non_ascii = 0;
            } else {
                self.prev_non_ascii += 1;
            }
            self.prev = caseless_class;
        }
        false
    }
}

struct ArabicFrenchCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
    case_state: LatinCaseState,
    prev_ascii: bool,
    current_word_len: u64,
    longest_word: u64,
}

impl ArabicFrenchCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        ArabicFrenchCandidate {
            data: data,
            score: 0,
            prev: 0,
            case_state: LatinCaseState::Space,
            prev_ascii: true,
            current_word_len: 0,
            longest_word: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> bool {
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return true;
            }
            let caseless_class = class & 0x7F;

            let ascii = b < 0x80;
            let ascii_pair = self.prev_ascii && ascii;

            if caseless_class != LATIN_LETTER {
                // We compute case penalties for French only
                self.case_state = LatinCaseState::Space;
            } else if (class >> 7) == 0 {
                if self.case_state == LatinCaseState::AllCaps && !ascii_pair {
                    self.score += IMPLAUSIBLE_LATIN_CASE_TRANSITION_PENALTY;
                }
                self.case_state = LatinCaseState::Lower;
            } else {
                match self.case_state {
                    LatinCaseState::Space => {
                        self.case_state = LatinCaseState::Upper;
                    }
                    LatinCaseState::Upper | LatinCaseState::AllCaps => {
                        self.case_state = LatinCaseState::AllCaps;
                    }
                    LatinCaseState::Lower => {
                        if !ascii_pair {
                            self.score += IMPLAUSIBLE_LATIN_CASE_TRANSITION_PENALTY;
                        }
                        self.case_state = LatinCaseState::Upper;
                    }
                }
            }

            // Count only Arabic word length and ignore French
            let non_ascii_alphabetic = self.data.is_non_latin_alphabetic(caseless_class);
            if non_ascii_alphabetic {
                self.current_word_len += 1;
            } else {
                if self.current_word_len > self.longest_word {
                    self.longest_word = self.current_word_len;
                }
                self.current_word_len = 0;
            }

            if !ascii_pair {
                self.score += self.data.score(caseless_class, self.prev);

                if self.prev == LATIN_LETTER && non_ascii_alphabetic {
                    self.score += LATIN_ADJACENCY_PENALTY;
                } else if caseless_class == LATIN_LETTER
                    && self.data.is_non_latin_alphabetic(self.prev)
                {
                    self.score += LATIN_ADJACENCY_PENALTY;
                }
            }

            self.prev_ascii = ascii;
            self.prev = caseless_class;
        }
        false
    }
}

struct CaselessCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
    prev_ascii: bool,
    current_word_len: u64,
    longest_word: u64,
}

impl CaselessCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        CaselessCandidate {
            data: data,
            score: 0,
            prev: 0,
            prev_ascii: true,
            current_word_len: 0,
            longest_word: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> bool {
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return true;
            }
            let caseless_class = class & 0x7F;

            let ascii = b < 0x80;
            let ascii_pair = self.prev_ascii && ascii;

            let non_ascii_alphabetic = self.data.is_non_latin_alphabetic(caseless_class);
            if non_ascii_alphabetic {
                self.current_word_len += 1;
            } else {
                if self.current_word_len > self.longest_word {
                    self.longest_word = self.current_word_len;
                }
                self.current_word_len = 0;
            }

            if !ascii_pair {
                self.score += self.data.score(caseless_class, self.prev);

                if self.prev == LATIN_LETTER && non_ascii_alphabetic {
                    self.score += LATIN_ADJACENCY_PENALTY;
                } else if caseless_class == LATIN_LETTER
                    && self.data.is_non_latin_alphabetic(self.prev)
                {
                    self.score += LATIN_ADJACENCY_PENALTY;
                }
            }

            self.prev_ascii = ascii;
            self.prev = caseless_class;
        }
        false
    }
}

struct LogicalCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
    prev_ascii: bool,
    plausible_punctuation: u64,
    current_word_len: u64,
    longest_word: u64,
}

impl LogicalCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        LogicalCandidate {
            data: data,
            score: 0,
            prev: 0,
            prev_ascii: true,
            plausible_punctuation: 0,
            current_word_len: 0,
            longest_word: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> bool {
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return true;
            }
            let caseless_class = class & 0x7F;

            let ascii = b < 0x80;
            let ascii_pair = self.prev_ascii && ascii;

            let non_ascii_alphabetic = self.data.is_non_latin_alphabetic(caseless_class);
            if non_ascii_alphabetic {
                self.current_word_len += 1;
            } else {
                if self.current_word_len > self.longest_word {
                    self.longest_word = self.current_word_len;
                }
                self.current_word_len = 0;
            }

            if !ascii_pair {
                self.score += self.data.score(caseless_class, self.prev);

                let prev_non_ascii_alphabetic = self.data.is_non_latin_alphabetic(self.prev);
                if caseless_class == ASCII_PUNCTUATION && prev_non_ascii_alphabetic {
                    self.plausible_punctuation += 1;
                }

                if self.prev == LATIN_LETTER && non_ascii_alphabetic {
                    self.score += LATIN_ADJACENCY_PENALTY;
                } else if caseless_class == LATIN_LETTER && prev_non_ascii_alphabetic {
                    self.score += LATIN_ADJACENCY_PENALTY;
                }
            }

            self.prev_ascii = ascii;
            self.prev = caseless_class;
        }
        false
    }
}

struct VisualCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
    prev_ascii: bool,
    plausible_punctuation: u64,
    current_word_len: u64,
    longest_word: u64,
}

impl VisualCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        VisualCandidate {
            data: data,
            score: 0,
            prev: 0,
            prev_ascii: true,
            plausible_punctuation: 0,
            current_word_len: 0,
            longest_word: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> bool {
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return true;
            }
            let caseless_class = class & 0x7F;

            let ascii = b < 0x80;
            let ascii_pair = self.prev_ascii && ascii;

            let non_ascii_alphabetic = self.data.is_non_latin_alphabetic(caseless_class);
            if non_ascii_alphabetic {
                self.current_word_len += 1;
            } else {
                if self.current_word_len > self.longest_word {
                    self.longest_word = self.current_word_len;
                }
                self.current_word_len = 0;
            }

            if !ascii_pair {
                self.score += self.data.score(caseless_class, self.prev);

                if non_ascii_alphabetic && self.prev == ASCII_PUNCTUATION {
                    self.plausible_punctuation += 1;
                }

                if self.prev == LATIN_LETTER && non_ascii_alphabetic {
                    self.score += LATIN_ADJACENCY_PENALTY;
                } else if caseless_class == LATIN_LETTER
                    && self.data.is_non_latin_alphabetic(self.prev)
                {
                    self.score += LATIN_ADJACENCY_PENALTY;
                }
            }

            self.prev_ascii = ascii;
            self.prev = caseless_class;
        }
        false
    }
}

struct Utf8Candidate {
    decoder: Decoder,
}

impl Utf8Candidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> bool {
        let mut dst = [0u8; 1024];
        let mut total_read = 0;
        loop {
            let (result, read, _) = self.decoder.decode_to_utf8_without_replacement(
                &buffer[total_read..],
                &mut dst,
                last,
            );
            total_read += read;
            match result {
                DecoderResult::InputEmpty => {
                    return false;
                }
                DecoderResult::Malformed(_, _) => {
                    return true;
                }
                DecoderResult::OutputFull => {
                    continue;
                }
            }
        }
    }
}

enum AsciiCjk {
    AsciiLetter,
    Cjk,
    Other,
}

struct GbkCandidate {
    decoder: Decoder,
    euc_range: u64,
    non_euc_range: u64,
    prev_was_euc_range: bool,
    cjk_pairs: u64,
    ascii_cjk_pairs: u64,
    prev: AsciiCjk,
    pua: u64,
    score: i64,
}

impl GbkCandidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> bool {
        let mut src = [0u8];
        let mut dst = [0u16; 2];
        for &b in buffer {
            let in_euc_range = b >= 0xA1 && b <= 0xFE;
            src[0] = b;
            let (result, read, written) = self
                .decoder
                .decode_to_utf16_without_replacement(&src, &mut dst, false);
            if written == 1 {
                let u = dst[0];
                if (u >= u16::from(b'a') && u <= u16::from(b'z'))
                    || (u >= u16::from(b'A') && u <= u16::from(b'Z'))
                {
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.ascii_cjk_pairs += 1;
                        }
                        _ => {}
                    }
                    self.prev = AsciiCjk::AsciiLetter;
                } else if u >= 0x4E00 && u <= 0x9FA5 {
                    if self.prev_was_euc_range && in_euc_range {
                        self.euc_range += 1;
                    } else {
                        self.non_euc_range += 1;
                    }
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.cjk_pairs += 1;
                        }
                        AsciiCjk::AsciiLetter => {
                            self.ascii_cjk_pairs += 1;
                        }
                        AsciiCjk::Other => {}
                    }
                    self.prev = AsciiCjk::Cjk;
                } else if (u >= 0x3400 && u < 0xA000) || (u >= 0xF900 && u < 0xFB00) {
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.cjk_pairs += 1;
                        }
                        AsciiCjk::AsciiLetter => {
                            self.ascii_cjk_pairs += 1;
                        }
                        AsciiCjk::Other => {}
                    }
                    self.prev = AsciiCjk::Cjk;
                } else if u >= 0xE000 && u < 0xF900 {
                    // Allow PUA characters in AR PL UMing CN on the
                    // assumption that since they complete logical sequences
                    // they might be supported by other fonts, too.
                    match u {
                        0xE78D..=0xE793
                        | 0xE794..=0xE796
                        | 0xE816..=0xE818
                        | 0xE81E
                        | 0xE826
                        | 0xE82B
                        | 0xE82C
                        | 0xE831
                        | 0xE832
                        | 0xE83B
                        | 0xE843
                        | 0xE854
                        | 0xE855
                        | 0xE864 => {}
                        _ => {
                            self.pua += 1;
                        }
                    }
                    self.prev = AsciiCjk::Other;
                } else {
                    self.prev = AsciiCjk::Other;
                }
            } else if written == 2 {
                let u = dst[0];
                if u >= 0xDB80 && u <= 0xDBFF {
                    self.pua += 1;
                    self.prev = AsciiCjk::Other;
                } else if u >= 0xD480 && u < 0xD880 {
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.cjk_pairs += 1;
                        }
                        AsciiCjk::AsciiLetter => {
                            self.ascii_cjk_pairs += 1;
                        }
                        AsciiCjk::Other => {}
                    }
                    self.prev = AsciiCjk::Cjk;
                } else if !(u >= 0xDC00 && u <= 0xDBFF) {
                    self.prev = AsciiCjk::Other;
                }
            }
            match result {
                DecoderResult::InputEmpty => {
                    assert_eq!(read, 1);
                }
                DecoderResult::Malformed(_, _) => {
                    return true;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
            self.prev_was_euc_range = in_euc_range;
        }
        if last {
            let (result, _, _) = self
                .decoder
                .decode_to_utf16_without_replacement(b"", &mut dst, true);
            match result {
                DecoderResult::InputEmpty => {}
                DecoderResult::Malformed(_, _) => {
                    return true;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
        }
        return false;
    }
}

struct ShiftJisCandidate {
    decoder: Decoder,
    non_ascii_seen: bool,
    cjk_pairs: u64,
    ascii_cjk_pairs: u64,
    prev: AsciiCjk,
    pua: u64,
    kana_kanji: u64,
    score: i64,
}

impl ShiftJisCandidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> bool {
        let mut dst = [0u16; 1024];
        let mut total_read = 0;
        loop {
            let (result, read, written) = self.decoder.decode_to_utf16_without_replacement(
                &buffer[total_read..],
                &mut dst,
                last,
            );
            total_read += read;
            for &u in dst[..written].iter() {
                if !self.non_ascii_seen && u >= 0x80 {
                    self.non_ascii_seen = true;
                    if u >= 0xFF61 && u <= 0xFF9F {
                        return true;
                    }
                }
                if (u >= u16::from(b'a') && u <= u16::from(b'z'))
                    || (u >= u16::from(b'A') && u <= u16::from(b'Z'))
                {
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.ascii_cjk_pairs += 1;
                        }
                        _ => {}
                    }
                    self.prev = AsciiCjk::AsciiLetter;
                } else if (u >= 0x3400 && u < 0xA000)
                    || (u >= 0xF900 && u < 0xFB00)
                    || (u >= 0x3040 && u < 0x3100)
                {
                    self.kana_kanji += 1;
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.cjk_pairs += 1;
                        }
                        AsciiCjk::AsciiLetter => {
                            self.ascii_cjk_pairs += 1;
                        }
                        AsciiCjk::Other => {}
                    }
                    self.prev = AsciiCjk::Cjk;
                } else if u >= 0xE000 && u < 0xF900 {
                    self.pua += 1;
                    self.prev = AsciiCjk::Other;
                } else {
                    self.prev = AsciiCjk::Other;
                }
            }
            match result {
                DecoderResult::InputEmpty => {
                    return false;
                }
                DecoderResult::Malformed(_, _) => {
                    return true;
                }
                DecoderResult::OutputFull => {
                    continue;
                }
            }
        }
    }
}

struct EucJpCandidate {
    decoder: Decoder,
    kana: u64,
    kanji: u64,
    non_ascii_seen: bool,
    cjk_pairs: u64,
    ascii_cjk_pairs: u64,
    prev: AsciiCjk,
    score: i64,
}

impl EucJpCandidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> bool {
        let mut dst = [0u16; 1024];
        let mut total_read = 0;
        loop {
            let (result, read, written) = self.decoder.decode_to_utf16_without_replacement(
                &buffer[total_read..],
                &mut dst,
                last,
            );
            total_read += read;
            for &u in dst[..written].iter() {
                if !self.non_ascii_seen && u >= 0x80 {
                    self.non_ascii_seen = true;
                    if u >= 0xFF61 && u <= 0xFF9F {
                        return true;
                    }
                }
                if (u >= u16::from(b'a') && u <= u16::from(b'z'))
                    || (u >= u16::from(b'A') && u <= u16::from(b'Z'))
                {
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.ascii_cjk_pairs += 1;
                        }
                        _ => {}
                    }
                    self.prev = AsciiCjk::AsciiLetter;
                } else if u >= 0x3040 && u < 0x3100 {
                    self.kana += 1;
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.cjk_pairs += 1;
                        }
                        AsciiCjk::AsciiLetter => {
                            self.ascii_cjk_pairs += 1;
                        }
                        AsciiCjk::Other => {}
                    }
                    self.prev = AsciiCjk::Cjk;
                } else if (u >= 0x3400 && u < 0xA000) || (u >= 0xF900 && u < 0xFB00) {
                    self.kanji += 1;
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.cjk_pairs += 1;
                        }
                        AsciiCjk::AsciiLetter => {
                            self.ascii_cjk_pairs += 1;
                        }
                        AsciiCjk::Other => {}
                    }
                    self.prev = AsciiCjk::Cjk;
                } else {
                    self.prev = AsciiCjk::Other;
                }
            }
            match result {
                DecoderResult::InputEmpty => {
                    return false;
                }
                DecoderResult::Malformed(_, _) => {
                    return true;
                }
                DecoderResult::OutputFull => {
                    continue;
                }
            }
        }
    }
}

struct Iso2022Candidate {
    decoder: Decoder,
}

impl Iso2022Candidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> bool {
        let mut dst = [0u16; 1024];
        let mut total_read = 0;
        loop {
            let (result, read, _) = self.decoder.decode_to_utf16_without_replacement(
                &buffer[total_read..],
                &mut dst,
                last,
            );
            total_read += read;
            match result {
                DecoderResult::InputEmpty => {
                    return false;
                }
                DecoderResult::Malformed(_, _) => {
                    return true;
                }
                DecoderResult::OutputFull => {
                    continue;
                }
            }
        }
    }
}

struct Big5Candidate {
    decoder: Decoder,
    cjk_pairs: u64,
    ascii_cjk_pairs: u64,
    hanzi: u64,
    prev: AsciiCjk,
    score: i64,
}

impl Big5Candidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> bool {
        let mut dst = [0u16; 1024];
        let mut total_read = 0;
        loop {
            let (result, read, written) = self.decoder.decode_to_utf16_without_replacement(
                &buffer[total_read..],
                &mut dst,
                last,
            );
            total_read += read;
            for &u in dst[..written].iter() {
                if (u >= u16::from(b'a') && u <= u16::from(b'z'))
                    || (u >= u16::from(b'A') && u <= u16::from(b'Z'))
                {
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.ascii_cjk_pairs += 1;
                        }
                        _ => {}
                    }
                    self.prev = AsciiCjk::AsciiLetter;
                } else if (u >= 0x3400 && u < 0xA000)
                    || (u >= 0xF900 && u < 0xFB00)
                    || (u >= 0xD480 && u < 0xD880)
                {
                    self.hanzi += 1;
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.cjk_pairs += 1;
                        }
                        AsciiCjk::AsciiLetter => {
                            self.ascii_cjk_pairs += 1;
                        }
                        AsciiCjk::Other => {}
                    }
                    self.prev = AsciiCjk::Cjk;
                } else if !(u >= 0xDC00 && u <= 0xDBFF) {
                    self.prev = AsciiCjk::Other;
                }
            }
            match result {
                DecoderResult::InputEmpty => {
                    return false;
                }
                DecoderResult::Malformed(_, _) => {
                    return true;
                }
                DecoderResult::OutputFull => {
                    continue;
                }
            }
        }
    }
}

struct EucKrCandidate {
    decoder: Decoder,
    modern_hangul: u64,
    other_hangul: u64,
    hanja: u64,
    prev_was_euc_range: bool,
    cjk_pairs: u64,
    ascii_cjk_pairs: u64,
    prev: AsciiCjk,
    score: i64,
}

impl EucKrCandidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> bool {
        let mut src = [0u8];
        let mut dst = [0u16; 2];
        for &b in buffer {
            let in_euc_range = b >= 0xA1 && b <= 0xFE;
            src[0] = b;
            let (result, read, written) = self
                .decoder
                .decode_to_utf16_without_replacement(&src, &mut dst, false);
            if written > 0 {
                let u = dst[0];
                if (u >= u16::from(b'a') && u <= u16::from(b'z'))
                    || (u >= u16::from(b'A') && u <= u16::from(b'Z'))
                {
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.ascii_cjk_pairs += 1;
                        }
                        _ => {}
                    }
                    self.prev = AsciiCjk::AsciiLetter;
                } else if u >= 0xAC00 && u <= 0xD7A3 {
                    if self.prev_was_euc_range && in_euc_range {
                        self.modern_hangul += 1;
                    } else {
                        self.other_hangul += 1;
                    }
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.cjk_pairs += 1;
                        }
                        AsciiCjk::AsciiLetter => {
                            self.ascii_cjk_pairs += 1;
                        }
                        AsciiCjk::Other => {}
                    }
                    self.prev = AsciiCjk::Cjk;
                } else if (u >= 0x4E00 && u < 0xAC00) || (u >= 0xF900 && u <= 0xFA0B) {
                    self.hanja += 1;
                    match self.prev {
                        AsciiCjk::Cjk => {
                            self.cjk_pairs += 1;
                        }
                        AsciiCjk::AsciiLetter => {
                            self.ascii_cjk_pairs += 1;
                        }
                        AsciiCjk::Other => {}
                    }
                    self.prev = AsciiCjk::Cjk;
                } else {
                    self.prev = AsciiCjk::Other;
                }
            }
            match result {
                DecoderResult::InputEmpty => {
                    assert_eq!(read, 1);
                }
                DecoderResult::Malformed(_, _) => {
                    return true;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
            self.prev_was_euc_range = in_euc_range;
        }
        if last {
            let (result, _, _) = self
                .decoder
                .decode_to_utf16_without_replacement(b"", &mut dst, true);
            match result {
                DecoderResult::InputEmpty => {}
                DecoderResult::Malformed(_, _) => {
                    return true;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
        }
        return false;
    }
}

enum InnerCandidate {
    Latin(LatinCandidate),
    NonLatinCased(NonLatinCasedCandidate),
    Caseless(CaselessCandidate),
    ArabicFrench(ArabicFrenchCandidate),
    Logical(LogicalCandidate),
    Visual(VisualCandidate),
    Utf8(Utf8Candidate),
    Iso2022(Iso2022Candidate),
    Shift(ShiftJisCandidate),
    EucJp(EucJpCandidate),
    EucKr(EucKrCandidate),
    Big5(Big5Candidate),
    Gbk(GbkCandidate),
}

impl InnerCandidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> bool {
        match self {
            InnerCandidate::Latin(c) => {
                let disqualified = c.feed(buffer);
                if disqualified {
                    return true;
                }
                if last {
                    // Treat EOF as space-like
                    return c.feed(b" ");
                }
                return false;
            }
            InnerCandidate::NonLatinCased(c) => {
                let disqualified = c.feed(buffer);
                if disqualified {
                    return true;
                }
                if last {
                    // Treat EOF as space-like
                    return c.feed(b" ");
                }
                return false;
            }
            InnerCandidate::Caseless(c) => {
                let disqualified = c.feed(buffer);
                if disqualified {
                    return true;
                }
                if last {
                    // Treat EOF as space-like
                    return c.feed(b" ");
                }
                return false;
            }
            InnerCandidate::ArabicFrench(c) => {
                let disqualified = c.feed(buffer);
                if disqualified {
                    return true;
                }
                if last {
                    // Treat EOF as space-like
                    return c.feed(b" ");
                }
                return false;
            }
            InnerCandidate::Logical(c) => {
                let disqualified = c.feed(buffer);
                if disqualified {
                    return true;
                }
                if last {
                    // Treat EOF as space-like
                    return c.feed(b" ");
                }
                return false;
            }
            InnerCandidate::Visual(c) => {
                let disqualified = c.feed(buffer);
                if disqualified {
                    return true;
                }
                if last {
                    // Treat EOF as space-like
                    return c.feed(b" ");
                }
                return false;
            }
            InnerCandidate::Utf8(c) => {
                return c.feed(buffer, last);
            }
            InnerCandidate::Iso2022(c) => {
                return c.feed(buffer, last);
            }
            InnerCandidate::Shift(c) => {
                return c.feed(buffer, last);
            }
            InnerCandidate::EucJp(c) => {
                return c.feed(buffer, last);
            }
            InnerCandidate::EucKr(c) => {
                return c.feed(buffer, last);
            }
            InnerCandidate::Big5(c) => {
                return c.feed(buffer, last);
            }
            InnerCandidate::Gbk(c) => {
                return c.feed(buffer, last);
            }
        }
    }
}

struct Candidate {
    inner: InnerCandidate,
    disqualified: bool,
}

impl Candidate {
    fn feed(&mut self, buffer: &[u8], last: bool) {
        if self.disqualified {
            return;
        }
        self.disqualified |= self.inner.feed(buffer, last);
    }

    fn new_latin(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::Latin(LatinCandidate::new(data)),
            disqualified: false,
        }
    }

    fn new_non_latin_cased(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::NonLatinCased(NonLatinCasedCandidate::new(data)),
            disqualified: false,
        }
    }

    fn new_caseless(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::Caseless(CaselessCandidate::new(data)),
            disqualified: false,
        }
    }

    fn new_arabic_french(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::ArabicFrench(ArabicFrenchCandidate::new(data)),
            disqualified: false,
        }
    }

    fn new_logical(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::Logical(LogicalCandidate::new(data)),
            disqualified: false,
        }
    }

    fn new_visual(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::Visual(VisualCandidate::new(data)),
            disqualified: false,
        }
    }

    fn new_iso_2022_jp() -> Self {
        Candidate {
            inner: InnerCandidate::Iso2022(Iso2022Candidate {
                decoder: ISO_2022_JP.new_decoder_without_bom_handling(),
            }),
            disqualified: false,
        }
    }

    fn new_utf_8() -> Self {
        Candidate {
            inner: InnerCandidate::Utf8(Utf8Candidate {
                decoder: UTF_8.new_decoder_without_bom_handling(),
            }),
            disqualified: false,
        }
    }

    fn new_shift_jis() -> Self {
        Candidate {
            inner: InnerCandidate::Shift(ShiftJisCandidate {
                decoder: SHIFT_JIS.new_decoder_without_bom_handling(),
                non_ascii_seen: false,
                cjk_pairs: 0,
                ascii_cjk_pairs: 0,
                prev: AsciiCjk::Other,
                pua: 0,
                kana_kanji: 0,
            }),
            disqualified: false,
        }
    }

    fn new_euc_jp() -> Self {
        Candidate {
            inner: InnerCandidate::EucJp(EucJpCandidate {
                decoder: EUC_JP.new_decoder_without_bom_handling(),
                non_ascii_seen: false,
                kana: 0,
                kanji: 0,
                cjk_pairs: 0,
                ascii_cjk_pairs: 0,
                prev: AsciiCjk::Other,
            }),
            disqualified: false,
        }
    }

    fn new_euc_kr() -> Self {
        Candidate {
            inner: InnerCandidate::EucKr(EucKrCandidate {
                decoder: EUC_KR.new_decoder_without_bom_handling(),
                modern_hangul: 0,
                other_hangul: 0,
                hanja: 0,
                prev_was_euc_range: false,
                cjk_pairs: 0,
                ascii_cjk_pairs: 0,
                prev: AsciiCjk::Other,
            }),
            disqualified: false,
        }
    }

    fn new_big5() -> Self {
        Candidate {
            inner: InnerCandidate::Big5(Big5Candidate {
                decoder: BIG5.new_decoder_without_bom_handling(),
                cjk_pairs: 0,
                ascii_cjk_pairs: 0,
                hanzi: 0,
                prev: AsciiCjk::Other,
            }),
            disqualified: false,
        }
    }

    fn new_gbk() -> Self {
        Candidate {
            inner: InnerCandidate::Gbk(GbkCandidate {
                decoder: GBK.new_decoder_without_bom_handling(),
                euc_range: 0,
                non_euc_range: 0,
                prev_was_euc_range: false,
                cjk_pairs: 0,
                ascii_cjk_pairs: 0,
                prev: AsciiCjk::Other,
                pua: 0,
            }),
            disqualified: false,
        }
    }

    fn score(&self) -> i64 {
        match &self.inner {
            InnerCandidate::Latin(c) => {
                return c.score;
            }
            InnerCandidate::NonLatinCased(c) => {
                if c.longest_word >= 2 {
                    return c.score;
                }
                return i64::min_value();
            }
            InnerCandidate::Caseless(c) => {
                if c.longest_word >= 2 {
                    return c.score;
                }
                return i64::min_value();
            }
            InnerCandidate::ArabicFrench(c) => {
                if c.longest_word >= 2 {
                    return c.score;
                }
                return i64::min_value();
            }
            InnerCandidate::Logical(c) => {
                if c.longest_word >= 2 {
                    return c.score;
                }
                return i64::min_value();
            }
            InnerCandidate::Visual(c) => {
                if c.longest_word >= 2 {
                    return c.score;
                }
                return i64::min_value();
            }
            InnerCandidate::Shift(c) => {
                return c.score;
            }
            InnerCandidate::EucJp(c) => {
                return c.score;
            }
            InnerCandidate::Big5(c) => {
                return c.score;
            }
            InnerCandidate::EucKr(c) => {
                return c.score;
            }
            InnerCandidate::Gbk(c) => {
                return c.score;
            }
            _ => {
                unreachable!();
            }
        }
    }

    fn plausible_punctuation(&self) -> u64 {
        match &self.inner {
            InnerCandidate::Logical(c) => {
                return c.plausible_punctuation;
            }
            InnerCandidate::Visual(c) => {
                return c.plausible_punctuation;
            }
            _ => {
                unreachable!();
            }
        }
    }

    fn encoding(&self) -> &'static Encoding {
        match &self.inner {
            InnerCandidate::Latin(c) => {
                return c.data.encoding;
            }
            InnerCandidate::NonLatinCased(c) => {
                return c.data.encoding;
            }
            InnerCandidate::Caseless(c) => {
                return c.data.encoding;
            }
            InnerCandidate::ArabicFrench(c) => {
                return c.data.encoding;
            }
            InnerCandidate::Logical(c) => {
                return c.data.encoding;
            }
            InnerCandidate::Visual(c) => {
                return c.data.encoding;
            }
            InnerCandidate::Shift(_) => {
                return SHIFT_JIS;
            }
            InnerCandidate::EucJp(_) => {
                return EUC_JP;
            }
            InnerCandidate::Big5(_) => {
                return BIG5;
            }
            InnerCandidate::EucKr(_) => {
                return EUC_KR;
            }
            InnerCandidate::Gbk(_) => {
                return GBK;
            }
            _ => {
                unreachable!();
            }
        }
    }

    fn add_to_score(&mut self, delta: i64) {
        match &mut self.inner {
            InnerCandidate::Latin(c) => {
                c.score += delta;
            }
            InnerCandidate::NonLatinCased(c) => {
                c.score += delta;
            }
            InnerCandidate::Caseless(c) => {
                c.score += delta;
            }
            InnerCandidate::ArabicFrench(c) => {
                c.score += delta;
            }
            InnerCandidate::Logical(c) => {
                c.score += delta;
            }
            InnerCandidate::Visual(c) => {
                c.score += delta;
            }
            _ => {
                unreachable!();
            }
        }
    }

    fn euc_jp_kana(&self) -> u64 {
        match &self.inner {
            InnerCandidate::EucJp(c) => {
                return c.kana;
            }
            _ => {
                unreachable!();
            }
        }
    }

    fn euc_kr_stats(&self) -> (u64, u64, u64) {
        match &self.inner {
            InnerCandidate::EucKr(c) => {
                return (c.modern_hangul, c.other_hangul, c.hanja);
            }
            _ => {
                unreachable!();
            }
        }
    }

    fn ascii_pair_ratio_ok(&self) -> bool {
        // TODO: Adjust the minimum required pair threshold on a per-encoding basis
        // The ratio is for avoiding misdetecting Latin as CJK.
        // The threshold is for avoiding misdetecting non-Latin single-byte
        // as CJK.
        let (cjk_pairs, ascii_cjk_pairs) = match &self.inner {
            InnerCandidate::Shift(c) => (c.cjk_pairs, c.ascii_cjk_pairs),
            InnerCandidate::EucJp(c) => (c.cjk_pairs, c.ascii_cjk_pairs),
            InnerCandidate::EucKr(c) => (c.cjk_pairs, c.ascii_cjk_pairs),
            InnerCandidate::Big5(c) => (c.cjk_pairs, c.ascii_cjk_pairs),
            InnerCandidate::Gbk(c) => (c.cjk_pairs, c.ascii_cjk_pairs),
            _ => {
                unreachable!();
            }
        };
        ascii_cjk_pairs <= cjk_pairs / 128 // Arbitrary allowance
    }

    fn pua_ratio_ok(&self) -> bool {
        let (cjk_pairs, pua) = match &self.inner {
            InnerCandidate::Shift(c) => (c.cjk_pairs, c.pua),
            InnerCandidate::Gbk(c) => (c.cjk_pairs, c.pua),
            _ => {
                unreachable!();
            }
        };
        pua <= cjk_pairs / 256 // Arbitrary allowance
    }

    fn gbk_euc_ratio_ok(&self) -> bool {
        match &self.inner {
            InnerCandidate::Gbk(c) => {
                // Arbitrary allowance
                c.non_euc_range <= c.euc_range / 256
            }
            _ => {
                unreachable!();
            }
        }
    }
}

fn count_non_ascii(buffer: &[u8]) -> u64 {
    let mut count = 0;
    for &b in buffer {
        if b >= 0x80 {
            count += 1;
        }
    }
    count
}

pub struct EncodingDetector {
    candidates: [Candidate; 25],
    non_ascii_seen: u64,
    fallback: Option<&'static Encoding>,
    last_before_non_ascii: Option<u8>,
    esc_seen: bool,
}

impl EncodingDetector {
    fn feed_impl(&mut self, buffer: &[u8], last: bool) {
        for candidate in self.candidates.iter_mut() {
            candidate.feed(buffer, last);
        }
        self.non_ascii_seen += count_non_ascii(buffer);
    }

    fn feed_without_guessing_impl(&mut self, buffer: &[u8], last: bool) {
        let start = if self.non_ascii_seen == 0 && !self.esc_seen {
            let up_to = Encoding::ascii_valid_up_to(buffer);
            let start = if let Some(escape) = memchr::memchr(0x1B, &buffer[..up_to]) {
                self.esc_seen = true;
                escape
            } else {
                up_to
            };
            if start == buffer.len() && !buffer.is_empty() {
                self.last_before_non_ascii = Some(buffer[buffer.len() - 1]);
                return;
            }
            if start == 0 {
                if let Some(ascii) = self.last_before_non_ascii {
                    let src = [ascii];
                    self.feed_impl(&src, false);
                }
                start
            } else {
                start - 1
            }
        } else {
            0
        };
        self.feed_impl(&buffer[start..], last);
    }

    fn guess(&self) -> (&'static Encoding, bool) {
        if self.non_ascii_seen == 0
            && self.esc_seen
            && !self.candidates[Self::ISO_2022_JP_INDEX].disqualified
        {
            return (ISO_2022_JP, false);
        }

        let utf = !self.candidates[Self::UTF_8_INDEX].disqualified && self.non_ascii_seen > 0;

        let mut euc_kr_ok = !self.candidates[Self::EUC_KR_INDEX].disqualified;
        let mut big5_ok = !self.candidates[Self::BIG5_INDEX].disqualified;
        let mut gbk_ok = !self.candidates[Self::GBK_INDEX].disqualified;
        let mut shift_jis_ok = !self.candidates[Self::SHIFT_JIS_INDEX].disqualified;
        let mut euc_jp_ok = !self.candidates[Self::EUC_JP_INDEX].disqualified;
        if let Some(fallback) = self.fallback {
            if fallback == EUC_KR && euc_kr_ok {
                return (EUC_KR, utf);
            }
            if fallback == BIG5 {
                if big5_ok {
                    return (BIG5, utf);
                }
                if gbk_ok {
                    return (GBK, utf);
                }
            }
            if fallback == GBK {
                if gbk_ok {
                    return (GBK, utf);
                }
                if big5_ok {
                    return (BIG5, utf);
                }
            }
            if fallback == SHIFT_JIS {
                if shift_jis_ok {
                    return (SHIFT_JIS, utf);
                }
                if euc_jp_ok {
                    return (EUC_JP, utf);
                }
            }
            if fallback == EUC_JP {
                if euc_jp_ok {
                    return (EUC_JP, utf);
                }
                if shift_jis_ok {
                    return (SHIFT_JIS, utf);
                }
            }
        }

        let mut encoding = WINDOWS_1252;
        let mut max = i64::min_value() + 1;
        for candidate in (&self.candidates[Self::FIRST_NORMAL_SINGLE_BYTE..]).iter() {
            let score = candidate.score();
            // println!(
            //     "{} {} {}",
            //     candidate.encoding().name(),
            //     score,
            //     candidate.disqualified
            // );
            if !candidate.disqualified && score > max {
                max = score;
                encoding = candidate.encoding();
            }
        }
        let visual = &self.candidates[Self::VISUAL_INDEX];
        let visual_score = visual.score();
        if !visual.disqualified
            && visual_score > max
            && visual.plausible_punctuation()
                > self.candidates[Self::LOGICAL_INDEX].plausible_punctuation()
        {
            max = visual_score;
            encoding = ISO_8859_8;
        }

        if !self.candidates[Self::EUC_KR_INDEX].ascii_pair_ratio_ok() {
            euc_kr_ok = false;
        }
        if !self.candidates[Self::BIG5_INDEX].ascii_pair_ratio_ok() {
            big5_ok = false;
        }
        if !self.candidates[Self::GBK_INDEX].ascii_pair_ratio_ok()
            || !self.candidates[Self::GBK_INDEX].pua_ratio_ok()
            || !self.candidates[Self::GBK_INDEX].gbk_euc_ratio_ok()
        {
            gbk_ok = false;
        }
        if !self.candidates[Self::SHIFT_JIS_INDEX].ascii_pair_ratio_ok()
            || !self.candidates[Self::SHIFT_JIS_INDEX].pua_ratio_ok()
        {
            shift_jis_ok = false;
        }
        if !self.candidates[Self::EUC_JP_INDEX].ascii_pair_ratio_ok() {
            euc_jp_ok = false;
        }

        // The most plausible CJK candidate is first decided by mechanism
        // other than a score, and then a score is computed for that
        // candidate for comparison with the best single-byte encoding.
        // It's likely possible to formulate a scoring mechanism that
        // would have the same outcome, but this formulation makes it
        // explicit how the legacy CJK encodings are compared relative
        // to each other.
        let mut cjk_encoding = SHIFT_JIS;
        let mut cjk_score = i64::min_value();

        // Loop only broken out of a goto forward.
        loop {
            if shift_jis_ok {
                cjk_score = self.candidates[Self::SHIFT_JIS_INDEX].score();
                break;
            }
            let euc_jp_kana = if euc_jp_ok {
                self.candidates[Self::EUC_JP_INDEX].euc_jp_kana()
            } else {
                0
            };
            if euc_jp_kana != 0 {
                gbk_ok = false;
            }
            let (modern_hangul, other_hangul, hanja) =
                self.candidates[Self::EUC_KR_INDEX].euc_kr_stats();
            // Arbitrary allowances for Hanja and non-modern Hangul
            if other_hangul > modern_hangul / 256 || hanja > modern_hangul / 128 {
                euc_kr_ok = false;
            }
            if gbk_ok {
                if euc_kr_ok {
                    cjk_encoding = EUC_KR;
                    cjk_score = self.candidates[Self::EUC_KR_INDEX].score();
                    break;
                }
                cjk_encoding = GBK;
                cjk_score = self.candidates[Self::GBK_INDEX].score();
                break;
            }
            if euc_jp_ok {
                // Arbitrary allowance for standalone jamo, which overlap
                // the kana range.
                if euc_kr_ok && euc_jp_kana <= modern_hangul / 256 {
                    cjk_encoding = EUC_KR;
                    cjk_score = self.candidates[Self::EUC_KR_INDEX].score();
                    break;
                }
                cjk_encoding = EUC_JP;
                cjk_score = self.candidates[Self::EUC_JP_INDEX].score();
                break;
            }
            if euc_kr_ok {
                cjk_encoding = EUC_KR;
                cjk_score = self.candidates[Self::EUC_KR_INDEX].score();
                break;
            }
            if big5_ok {
                cjk_encoding = BIG5;
                cjk_score = self.candidates[Self::BIG5_INDEX].score();
                break;
            }
            break;
        }
        if cjk_score > max {
            encoding = cjk_encoding;
        }
        (encoding, utf)
    }

    pub fn feed_without_guessing(&mut self, buffer: &[u8]) {
        self.feed_without_guessing_impl(buffer, false);
    }

    pub fn feed(&mut self, buffer: &[u8], last: bool) -> (&'static Encoding, bool, u64) {
        self.feed_without_guessing_impl(buffer, last);
        let (enc, utf) = self.guess();
        (enc, utf, self.non_ascii_seen)
    }

    pub fn new() -> Self {
        EncodingDetector::new_with_fallback(None)
    }

    // XXX Test-only API
    pub fn find_score(&self, encoding: &'static Encoding) -> (i64, bool) {
        for candidate in (&self.candidates[Self::SHIFT_JIS_INDEX..]).iter() {
            if encoding == candidate.encoding() {
                return (candidate.score(), candidate.disqualified);
            }
        }
        (0, false)
    }

    const UTF_8_INDEX: usize = 0;

    const ISO_2022_JP_INDEX: usize = 1;

    const SHIFT_JIS_INDEX: usize = 2;

    const EUC_JP_INDEX: usize = 3;

    const EUC_KR_INDEX: usize = 4;

    const BIG5_INDEX: usize = 5;

    const GBK_INDEX: usize = 6;

    const FIRST_SINGLE_BYTE: usize = 7;

    const FIRST_NORMAL_SINGLE_BYTE: usize = 8;

    const VISUAL_INDEX: usize = 7;

    const LOGICAL_INDEX: usize = 15;

    const WINDOWS_1250_SINGLE_BYTE: usize = 10;

    const WINDOWS_1251_SINGLE_BYTE: usize = 9;

    const WINDOWS_1252_SINGLE_BYTE: usize = 8;

    const WINDOWS_1253_SINGLE_BYTE: usize = 16;

    const WINDOWS_1254_SINGLE_BYTE: usize = 13;

    const WINDOWS_1255_SINGLE_BYTE: usize = 15;

    const WINDOWS_1256_SINGLE_BYTE: usize = 12;

    const WINDOWS_1257_SINGLE_BYTE: usize = 18;

    const WINDOWS_1258_SINGLE_BYTE: usize = 22;

    const ISO_8859_3_SINGLE_BYTE: usize = 11;

    const ISO_8859_4_SINGLE_BYTE: usize = 23;

    const ISO_8859_5_SINGLE_BYTE: usize = 24;

    const ISO_8859_6_SINGLE_BYTE: usize = 21;

    pub fn new_with_fallback(fallback: Option<&'static Encoding>) -> Self {
        let mut det = EncodingDetector {
            candidates: [
                Candidate::new_utf_8(),                                                // 0
                Candidate::new_iso_2022_jp(),                                          // 1
                Candidate::new_shift_jis(),                                            // 2
                Candidate::new_euc_jp(),                                               // 3
                Candidate::new_euc_kr(),                                               // 4
                Candidate::new_big5(),                                                 // 5
                Candidate::new_gbk(),                                                  // 6
                Candidate::new_visual(&SINGLE_BYTE_DATA[ISO_8859_8_INDEX]),            // 7
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1252_INDEX]),           // 8
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[WINDOWS_1251_INDEX]), // 9
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1250_INDEX]),           // 10
                Candidate::new_latin(&SINGLE_BYTE_DATA[ISO_8859_2_INDEX]),             // 11
                Candidate::new_arabic_french(&SINGLE_BYTE_DATA[WINDOWS_1256_INDEX]),   // 12
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1254_INDEX]),           // 13
                Candidate::new_caseless(&SINGLE_BYTE_DATA[WINDOWS_874_INDEX]),         // 14
                Candidate::new_logical(&SINGLE_BYTE_DATA[WINDOWS_1255_INDEX]),         // 15
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[WINDOWS_1253_INDEX]), // 16
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[ISO_8859_7_INDEX]),   // 17
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1257_INDEX]),           // 18
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[KOI8_U_INDEX]),       // 19
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[IBM866_INDEX]),       // 20
                Candidate::new_caseless(&SINGLE_BYTE_DATA[ISO_8859_6_INDEX]),          // 21
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1258_INDEX]),           // 22
                Candidate::new_latin(&SINGLE_BYTE_DATA[ISO_8859_4_INDEX]),             // 23
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[ISO_8859_5_INDEX]),   // 24
            ],
            non_ascii_seen: 0,
            fallback: fallback,
            last_before_non_ascii: None,
            esc_seen: false,
        };
        // When in doubt, guess windows-1252
        // det.candidates[Self::WINDOWS_1252_SINGLE_BYTE].add_to_score(10);

        // It's questionable whether ISO-8859-4 should even be a possible outcome.
        // det.candidates[Self::ISO_8859_4_SINGLE_BYTE].add_to_score(-10);
        // For short strings, windows-1257 gets confused with windows-1252 a lot
        // det.candidates[Self::WINDOWS_1257_SINGLE_BYTE].add_to_score(0);

        // For short strings, windows-1250 gets confused with windows-1252 a lot
        // det.candidates[Self::WINDOWS_1250_SINGLE_BYTE].add_to_score(0);

        if let Some(fallback) = fallback {
            for single_byte in det.candidates[Self::FIRST_SINGLE_BYTE..].iter_mut() {
                if single_byte.encoding() == fallback {
                    single_byte.add_to_score(1);
                    break;
                }
            }
        }
        det
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(input: &str, encoding: &'static Encoding) {
        let (bytes, _, _) = encoding.encode(input);
        let mut det = EncodingDetector::new();
        let (enc, _, _) = det.feed(&bytes, true);
        let (decoded, _) = enc.decode_without_bom_handling(&bytes);
        println!("{:?}", decoded);
        assert_eq!(enc, encoding);
    }

    #[test]
    fn test_empty() {
        let mut det = EncodingDetector::new();
        let (enc, utf, non_ascii) = det.feed(b"", true);
        assert_eq!(enc, WINDOWS_1252);
        assert!(!utf);
        assert_eq!(non_ascii, 0);
    }

    #[test]
    fn test_fi() {
        check("Ääni", WINDOWS_1252);
    }

    #[test]
    fn test_ru() {
        check("Русский", WINDOWS_1251);
    }

    #[test]
    fn test_el() {
        check("Ελληνικά", WINDOWS_1253);
    }

    #[test]
    fn test_de() {
        check("Straße", WINDOWS_1252);
    }

    #[test]
    fn test_he() {
        check("\u{5E2}\u{5D1}\u{5E8}\u{5D9}\u{5EA}", WINDOWS_1255);
    }

    #[test]
    fn test_th() {
        check("ไทย", WINDOWS_874);
    }

    #[test]
    fn test_foo() {
        check("Straße", WINDOWS_1252);
    }
}
