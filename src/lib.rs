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
mod tld;
use data::*;
use tld::classify_tld;
use tld::Tld;

const LATIN_ADJACENCY_PENALTY: i64 = -50;

const IMPLAUSIBILITY_PENALTY: i64 = -220;

const IMPLAUSIBLE_LATIN_CASE_TRANSITION_PENALTY: i64 = -180;

const NON_LATIN_CAPITALIZATION_BONUS: i64 = 40;

const NON_LATIN_ALL_CAPS_PENALTY: i64 = -40;

// XXX rework how this gets applied
const NON_LATIN_MIXED_CASE_PENALTY: i64 = -20;

// XXX Remove this
const NON_LATIN_CAMEL_PENALTY: i64 = -80;

const NON_LATIN_IMPLAUSIBLE_CASE_TRANSITION_PENALTY: i64 = -100;

// Manually calibrated relative to windows-1256 Arabic
const CJK_BASE_SCORE: i64 = 41;

const CJK_SECONDARY_BASE_SCORE: i64 = 20; // Was 20

const SHIFT_JIS_SCORE_PER_KANA: i64 = 20;

const SHIFT_JIS_SCORE_PER_LEVEL_1_KANJI: i64 = CJK_BASE_SCORE;

const SHIFT_JIS_SCORE_PER_LEVEL_2_KANJI: i64 = CJK_SECONDARY_BASE_SCORE;

const HALF_WIDTH_KATAKANA_PENALTY: i64 = -(CJK_BASE_SCORE * 3);

const SHIFT_JIS_PUA_PENALTY: i64 = -(CJK_BASE_SCORE * 10); // Should this be larger?

const EUC_JP_SCORE_PER_KANA: i64 = CJK_BASE_SCORE + (CJK_BASE_SCORE / 3); // Relative to Big5

const EUC_JP_SCORE_PER_NEAR_OBSOLETE_KANA: i64 = CJK_BASE_SCORE - 1;

const EUC_JP_SCORE_PER_LEVEL_1_KANJI: i64 = CJK_BASE_SCORE;

const EUC_JP_SCORE_PER_LEVEL_2_KANJI: i64 = CJK_SECONDARY_BASE_SCORE;

const EUC_JP_SCORE_PER_OTHER_KANJI: i64 = CJK_SECONDARY_BASE_SCORE / 4;

const EUC_JP_INITIAL_KANA_PENALTY: i64 = -((CJK_BASE_SCORE / 3) + 1);

const BIG5_SCORE_PER_LEVEL_1_HANZI: i64 = CJK_BASE_SCORE;

const BIG5_SCORE_PER_OTHER_HANZI: i64 = CJK_SECONDARY_BASE_SCORE;

const EUC_KR_SCORE_PER_EUC_HANGUL: i64 = CJK_BASE_SCORE + 1;

const EUC_KR_SCORE_PER_NON_EUC_HANGUL: i64 = CJK_SECONDARY_BASE_SCORE / 5;

const EUC_KR_SCORE_PER_HANJA: i64 = CJK_SECONDARY_BASE_SCORE / 2;

const EUC_KR_HANJA_AFTER_HANGUL_PENALTY: i64 = -(CJK_BASE_SCORE * 10);

const EUC_KR_LONG_WORD_PENALTY: i64 = -6;

const GBK_SCORE_PER_LEVEL_1: i64 = CJK_BASE_SCORE;

const GBK_SCORE_PER_LEVEL_2: i64 = CJK_SECONDARY_BASE_SCORE;

const GBK_SCORE_PER_NON_EUC: i64 = CJK_SECONDARY_BASE_SCORE / 4;

const GBK_PUA_PENALTY: i64 = -(CJK_BASE_SCORE * 10); // Factor should be at least 2, but should it be larger?

const CJK_LATIN_ADJACENCY_PENALTY: i64 = -40; // smaller penalty than LATIN_ADJACENCY_PENALTY

const CJ_PUNCTUATION: i64 = CJK_BASE_SCORE / 2;

const CJK_OTHER: i64 = CJK_SECONDARY_BASE_SCORE / 4;

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
            prev: 0,
            case_state: NonLatinCaseState::Space,
            prev_ascii: true,
            current_word_len: 0,
            longest_word: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> Option<i64> {
        let mut score = 0i64;
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return None;
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
                        score += NON_LATIN_CAPITALIZATION_BONUS;
                    }
                    NonLatinCaseState::LowerUpper => {
                        // Once per word
                        score += NON_LATIN_IMPLAUSIBLE_CASE_TRANSITION_PENALTY;
                    }
                    NonLatinCaseState::LowerUpperLower => {
                        // Once per word
                        score += NON_LATIN_CAMEL_PENALTY;
                    }
                    NonLatinCaseState::AllCaps => {
                        // Intentionally applied only once per word.
                        if self.data == &SINGLE_BYTE_DATA[KOI8_U_INDEX] {
                            // Apply only to KOI8-U.
                            score += NON_LATIN_ALL_CAPS_PENALTY;
                        }
                    }
                    NonLatinCaseState::Mix | NonLatinCaseState::LowerUpperUpper => {
                        // Per letter
                        score += NON_LATIN_MIXED_CASE_PENALTY;
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
                        score += NON_LATIN_CAMEL_PENALTY;
                        self.case_state = NonLatinCaseState::LowerUpperLower;
                    }
                    NonLatinCaseState::LowerUpperUpper => {
                        score += NON_LATIN_MIXED_CASE_PENALTY;
                        self.case_state = NonLatinCaseState::Mix;
                    }
                    NonLatinCaseState::LowerUpperLower => {
                        score += NON_LATIN_CAMEL_PENALTY;
                        self.case_state = NonLatinCaseState::UpperLowerCamel;
                    }
                    NonLatinCaseState::AllCaps => {
                        score += NON_LATIN_IMPLAUSIBLE_CASE_TRANSITION_PENALTY;
                        self.case_state = NonLatinCaseState::Mix;
                    }
                    NonLatinCaseState::Mix => {
                        score += NON_LATIN_MIXED_CASE_PENALTY;
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
                        score += NON_LATIN_IMPLAUSIBLE_CASE_TRANSITION_PENALTY;
                        self.case_state = NonLatinCaseState::LowerUpperUpper;
                    }
                    NonLatinCaseState::LowerUpperUpper | NonLatinCaseState::Mix => {
                        score += NON_LATIN_MIXED_CASE_PENALTY;
                    }
                    NonLatinCaseState::LowerUpperLower => {
                        score += NON_LATIN_IMPLAUSIBLE_CASE_TRANSITION_PENALTY;
                        self.case_state = NonLatinCaseState::Mix;
                    }
                    NonLatinCaseState::AllCaps => {}
                }
            }

            // XXX Apply penalty if > 16
            if non_ascii_alphabetic {
                self.current_word_len += 1;
            } else {
                if self.current_word_len > self.longest_word {
                    self.longest_word = self.current_word_len;
                }
                self.current_word_len = 0;
            }

            if !ascii_pair {
                score += self.data.score(caseless_class, self.prev);

                if self.prev == LATIN_LETTER && non_ascii_alphabetic {
                    score += LATIN_ADJACENCY_PENALTY;
                } else if caseless_class == LATIN_LETTER
                    && self.data.is_non_latin_alphabetic(self.prev)
                {
                    score += LATIN_ADJACENCY_PENALTY;
                }
            }

            self.prev_ascii = ascii;
            self.prev = caseless_class;
        }
        Some(score)
    }
}

struct LatinCandidate {
    data: &'static SingleByteData,
    prev: u8,
    case_state: LatinCaseState,
    prev_non_ascii: u32,
}

impl LatinCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        LatinCandidate {
            data: data,
            prev: 0,
            case_state: LatinCaseState::Space,
            prev_non_ascii: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> Option<i64> {
        let mut score = 0i64;
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return None;
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
            score += non_ascii_penalty;
            // XXX if has Vietnamese-only characters and word length > 7,
            // apply penalty

            if !self.data.is_latin_alphabetic(caseless_class) {
                self.case_state = LatinCaseState::Space;
            } else if (class >> 7) == 0 {
                // Penalizing lower case after two upper case
                // is important for avoiding misdetecting
                // windows-1250 as windows-1252 (byte 0x9F).
                if self.case_state == LatinCaseState::AllCaps && !ascii_pair {
                    score += IMPLAUSIBLE_LATIN_CASE_TRANSITION_PENALTY;
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
                            score += IMPLAUSIBLE_LATIN_CASE_TRANSITION_PENALTY;
                        }
                        self.case_state = LatinCaseState::Upper;
                    }
                }
            }

            if !ascii_pair {
                score += self.data.score(caseless_class, self.prev);
            }

            if ascii {
                self.prev_non_ascii = 0;
            } else {
                self.prev_non_ascii += 1;
            }
            self.prev = caseless_class;
        }
        Some(score)
    }
}

struct ArabicFrenchCandidate {
    data: &'static SingleByteData,
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
            prev: 0,
            case_state: LatinCaseState::Space,
            prev_ascii: true,
            current_word_len: 0,
            longest_word: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> Option<i64> {
        let mut score = 0i64;
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return None;
            }
            let caseless_class = class & 0x7F;

            let ascii = b < 0x80;
            let ascii_pair = self.prev_ascii && ascii;

            if caseless_class != LATIN_LETTER {
                // We compute case penalties for French only
                self.case_state = LatinCaseState::Space;
            } else if (class >> 7) == 0 {
                if self.case_state == LatinCaseState::AllCaps && !ascii_pair {
                    score += IMPLAUSIBLE_LATIN_CASE_TRANSITION_PENALTY;
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
                            score += IMPLAUSIBLE_LATIN_CASE_TRANSITION_PENALTY;
                        }
                        self.case_state = LatinCaseState::Upper;
                    }
                }
            }

            // Count only Arabic word length and ignore French
            let non_ascii_alphabetic = self.data.is_non_latin_alphabetic(caseless_class);
            // XXX apply penalty if > 23
            if non_ascii_alphabetic {
                self.current_word_len += 1;
            } else {
                if self.current_word_len > self.longest_word {
                    self.longest_word = self.current_word_len;
                }
                self.current_word_len = 0;
            }

            if !ascii_pair {
                score += self.data.score(caseless_class, self.prev);

                if self.prev == LATIN_LETTER && non_ascii_alphabetic {
                    score += LATIN_ADJACENCY_PENALTY;
                } else if caseless_class == LATIN_LETTER
                    && self.data.is_non_latin_alphabetic(self.prev)
                {
                    score += LATIN_ADJACENCY_PENALTY;
                }
            }

            self.prev_ascii = ascii;
            self.prev = caseless_class;
        }
        Some(score)
    }
}

struct CaselessCandidate {
    data: &'static SingleByteData,
    prev: u8,
    prev_ascii: bool,
    current_word_len: u64,
    longest_word: u64,
}

impl CaselessCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        CaselessCandidate {
            data: data,
            prev: 0,
            prev_ascii: true,
            current_word_len: 0,
            longest_word: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> Option<i64> {
        let mut score = 0i64;
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return None;
            }
            let caseless_class = class & 0x7F;

            let ascii = b < 0x80;
            let ascii_pair = self.prev_ascii && ascii;

            let non_ascii_alphabetic = self.data.is_non_latin_alphabetic(caseless_class);
            // Apply penalty if > 23 and not Thai
            if non_ascii_alphabetic {
                self.current_word_len += 1;
            } else {
                if self.current_word_len > self.longest_word {
                    self.longest_word = self.current_word_len;
                }
                self.current_word_len = 0;
            }

            if !ascii_pair {
                score += self.data.score(caseless_class, self.prev);

                if self.prev == LATIN_LETTER && non_ascii_alphabetic {
                    score += LATIN_ADJACENCY_PENALTY;
                } else if caseless_class == LATIN_LETTER
                    && self.data.is_non_latin_alphabetic(self.prev)
                {
                    score += LATIN_ADJACENCY_PENALTY;
                }
            }

            self.prev_ascii = ascii;
            self.prev = caseless_class;
        }
        Some(score)
    }
}

struct LogicalCandidate {
    data: &'static SingleByteData,
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
            prev: 0,
            prev_ascii: true,
            plausible_punctuation: 0,
            current_word_len: 0,
            longest_word: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> Option<i64> {
        let mut score = 0i64;
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return None;
            }
            let caseless_class = class & 0x7F;

            let ascii = b < 0x80;
            let ascii_pair = self.prev_ascii && ascii;

            let non_ascii_alphabetic = self.data.is_non_latin_alphabetic(caseless_class);
            // XXX apply penalty if > 22
            if non_ascii_alphabetic {
                self.current_word_len += 1;
            } else {
                if self.current_word_len > self.longest_word {
                    self.longest_word = self.current_word_len;
                }
                self.current_word_len = 0;
            }

            if !ascii_pair {
                score += self.data.score(caseless_class, self.prev);

                let prev_non_ascii_alphabetic = self.data.is_non_latin_alphabetic(self.prev);
                if caseless_class == ASCII_PUNCTUATION && prev_non_ascii_alphabetic {
                    self.plausible_punctuation += 1;
                }

                if self.prev == LATIN_LETTER && non_ascii_alphabetic {
                    score += LATIN_ADJACENCY_PENALTY;
                } else if caseless_class == LATIN_LETTER && prev_non_ascii_alphabetic {
                    score += LATIN_ADJACENCY_PENALTY;
                }
            }

            self.prev_ascii = ascii;
            self.prev = caseless_class;
        }
        Some(score)
    }
}

struct VisualCandidate {
    data: &'static SingleByteData,
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
            prev: 0,
            prev_ascii: true,
            plausible_punctuation: 0,
            current_word_len: 0,
            longest_word: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> Option<i64> {
        let mut score = 0i64;
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return None;
            }
            let caseless_class = class & 0x7F;

            let ascii = b < 0x80;
            let ascii_pair = self.prev_ascii && ascii;

            let non_ascii_alphabetic = self.data.is_non_latin_alphabetic(caseless_class);
            // XXX apply penalty if > 22
            if non_ascii_alphabetic {
                self.current_word_len += 1;
            } else {
                if self.current_word_len > self.longest_word {
                    self.longest_word = self.current_word_len;
                }
                self.current_word_len = 0;
            }

            if !ascii_pair {
                score += self.data.score(caseless_class, self.prev);

                if non_ascii_alphabetic && self.prev == ASCII_PUNCTUATION {
                    self.plausible_punctuation += 1;
                }

                if self.prev == LATIN_LETTER && non_ascii_alphabetic {
                    score += LATIN_ADJACENCY_PENALTY;
                } else if caseless_class == LATIN_LETTER
                    && self.data.is_non_latin_alphabetic(self.prev)
                {
                    score += LATIN_ADJACENCY_PENALTY;
                }
            }

            self.prev_ascii = ascii;
            self.prev = caseless_class;
        }
        Some(score)
    }
}

struct Utf8Candidate {
    decoder: Decoder,
}

impl Utf8Candidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> Option<i64> {
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
                    return Some(0);
                }
                DecoderResult::Malformed(_, _) => {
                    return None;
                }
                DecoderResult::OutputFull => {
                    continue;
                }
            }
        }
    }
}

#[derive(PartialEq)]
enum LatinCj {
    AsciiLetter,
    Cj,
    Other,
}

#[derive(PartialEq)]
enum LatinKorean {
    AsciiLetter,
    Hangul,
    Hanja,
    Other,
}

fn cjk_extra_score(u: u16, table: &'static [u16; 128]) -> i64 {
    if let Some(pos) = table.iter().position(|&x| x == u) {
        ((128 - pos) / 16) as i64
    } else {
        0
    }
}

struct GbkCandidate {
    decoder: Decoder,
    prev_byte: u8,
    prev: LatinCj,
}

impl GbkCandidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> Option<i64> {
        let mut score = 0i64;
        let mut src = [0u8];
        let mut dst = [0u16; 2];
        for &b in buffer {
            src[0] = b;
            let (result, read, written) = self
                .decoder
                .decode_to_utf16_without_replacement(&src, &mut dst, false);
            if written == 1 {
                let u = dst[0];
                if (u >= u16::from(b'a') && u <= u16::from(b'z'))
                    || (u >= u16::from(b'A') && u <= u16::from(b'Z'))
                {
                    if self.prev == LatinCj::Cj {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::AsciiLetter;
                } else if u >= 0x4E00 && u <= 0x9FA5 {
                    if b >= 0xA1 && b <= 0xFE {
                        match self.prev_byte {
                            0xA1..=0xD7 => {
                                score += GBK_SCORE_PER_LEVEL_1;
                                score +=
                                    cjk_extra_score(u, &data::DETECTOR_DATA.frequent_simplified);
                            }
                            0xD8..=0xFE => score += GBK_SCORE_PER_LEVEL_2,
                            _ => {
                                score += GBK_SCORE_PER_NON_EUC;
                            }
                        }
                    } else {
                        score += GBK_SCORE_PER_NON_EUC;
                    }
                    if self.prev == LatinCj::AsciiLetter {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::Cj;
                } else if (u >= 0x3400 && u < 0xA000) || (u >= 0xF900 && u < 0xFB00) {
                    if self.prev == LatinCj::AsciiLetter {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::Cj;
                } else if u >= 0xE000 && u < 0xF900 {
                    // Treat the GB18030-required PUA mappings as non-EUC ideographs.
                    match u {
                        0xE78D..=0xE796
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
                        | 0xE864 => {
                            score += GBK_SCORE_PER_NON_EUC;
                            if self.prev == LatinCj::AsciiLetter {
                                score += CJK_LATIN_ADJACENCY_PENALTY;
                            }
                            self.prev = LatinCj::Cj;
                        }
                        _ => {
                            score += GBK_PUA_PENALTY;
                            self.prev = LatinCj::Other;
                        }
                    }
                } else {
                    match u {
                        0x3000 // Distinct from Korean, space
                        | 0x3001 // Distinct from Korean, enumeration comma
                        | 0x3002 // Distinct from Korean, full stop
                        | 0xFF08 // Distinct from Korean, parenthesis
                        | 0xFF09 // Distinct from Korean, parenthesis
                        | 0xFF01 // Distinct from Japanese, exclamation
                        | 0xFF0C // Distinct from Japanese, comma
                        | 0xFF1B // Distinct from Japanese, semicolon
                        | 0xFF1F // Distinct from Japanese, question
                        => {
                            score += CJ_PUNCTUATION;
                        }
                        0..=0x7F => {}
                        _ => {
                            score += CJK_OTHER;
                        }
                    }
                    self.prev = LatinCj::Other;
                }
            } else if written == 2 {
                let u = dst[0];
                if u >= 0xDB80 && u <= 0xDBFF {
                    score += GBK_PUA_PENALTY;
                    self.prev = LatinCj::Other;
                } else if u >= 0xD480 && u < 0xD880 {
                    score += GBK_SCORE_PER_NON_EUC;
                    if self.prev == LatinCj::AsciiLetter {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::Cj;
                } else {
                    score += CJK_OTHER;
                    self.prev = LatinCj::Other;
                }
            }
            match result {
                DecoderResult::InputEmpty => {
                    assert_eq!(read, 1);
                }
                DecoderResult::Malformed(_, _) => {
                    return None;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
            self.prev_byte = b;
        }
        if last {
            let (result, _, _) = self
                .decoder
                .decode_to_utf16_without_replacement(b"", &mut dst, true);
            match result {
                DecoderResult::InputEmpty => {}
                DecoderResult::Malformed(_, _) => {
                    return None;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
        }
        Some(score)
    }
}

fn problematic_lead(b: u8) -> bool {
    b == 0x92
}

struct ShiftJisCandidate {
    decoder: Decoder,
    non_ascii_seen: bool,
    prev: LatinCj,
    prev_byte: u8,
    pending_score: Option<i64>,
}

impl ShiftJisCandidate {
    fn maybe_set_as_pending(&mut self, s: i64) -> i64 {
        assert!(self.pending_score.is_none());
        if self.prev == LatinCj::Cj || !problematic_lead(self.prev_byte) {
            s
        } else {
            self.pending_score = Some(s);
            0
        }
    }

    fn feed(&mut self, buffer: &[u8], last: bool) -> Option<i64> {
        let mut score = 0i64;
        let mut src = [0u8];
        let mut dst = [0u16; 2];
        for &b in buffer {
            src[0] = b;
            let (result, read, written) = self
                .decoder
                .decode_to_utf16_without_replacement(&src, &mut dst, false);
            if written > 0 {
                let u = dst[0];
                if !self.non_ascii_seen && u >= 0x80 {
                    self.non_ascii_seen = true;
                    if u >= 0xFF61 && u <= 0xFF9F {
                        return None;
                    }
                }
                if (u >= u16::from(b'a') && u <= u16::from(b'z'))
                    || (u >= u16::from(b'A') && u <= u16::from(b'Z'))
                {
                    self.pending_score = None; // Discard pending score
                    if self.prev == LatinCj::Cj {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::AsciiLetter;
                } else if u >= 0xFF61 && u <= 0xFF9F {
                    self.pending_score = None; // Discard pending score
                    score += HALF_WIDTH_KATAKANA_PENALTY;
                    self.prev = LatinCj::Cj;
                } else if u >= 0x3040 && u < 0x3100 {
                    if let Some(pending) = self.pending_score {
                        score += pending;
                        self.pending_score = None;
                    }
                    score += SHIFT_JIS_SCORE_PER_KANA;
                    if self.prev == LatinCj::AsciiLetter {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::Cj;
                } else if (u >= 0x3400 && u < 0xA000) || (u >= 0xF900 && u < 0xFB00) {
                    if let Some(pending) = self.pending_score {
                        score += pending;
                        self.pending_score = None;
                    }
                    if self.prev_byte < 0x98 || (self.prev_byte == 0x98 && b < 0x73) {
                        score += self.maybe_set_as_pending(
                            SHIFT_JIS_SCORE_PER_LEVEL_1_KANJI
                                + cjk_extra_score(u, &data::DETECTOR_DATA.frequent_kanji),
                        );
                    } else {
                        score += self.maybe_set_as_pending(SHIFT_JIS_SCORE_PER_LEVEL_2_KANJI);
                    }
                    if self.prev == LatinCj::AsciiLetter {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::Cj;
                } else if u >= 0xE000 && u < 0xF900 {
                    self.pending_score = None; // Discard pending score
                    score += SHIFT_JIS_PUA_PENALTY;
                    self.prev = LatinCj::Other;
                } else {
                    self.pending_score = None; // Discard pending score
                    match u {
                        0x3000 // Distinct from Korean, space
                        | 0x3001 // Distinct from Korean, enumeration comma
                        | 0x3002 // Distinct from Korean, full stop
                        | 0xFF08 // Distinct from Korean, parenthesis
                        | 0xFF09 // Distinct from Korean, parenthesis
                        => {
                            // Not really needed for CJK distinction
                            // but let's give non-zero score for these
                            // common byte pairs anyway.
                            score += CJ_PUNCTUATION;
                        }
                        0..=0x7F => {}
                        _ => {
                            score += CJK_OTHER;
                        }
                    }
                    self.prev = LatinCj::Other;
                }
            }
            match result {
                DecoderResult::InputEmpty => {
                    assert_eq!(read, 1);
                }
                DecoderResult::Malformed(_, _) => {
                    return None;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
            self.prev_byte = b;
        }
        if last {
            let (result, _, _) = self
                .decoder
                .decode_to_utf16_without_replacement(b"", &mut dst, true);
            match result {
                DecoderResult::InputEmpty => {}
                DecoderResult::Malformed(_, _) => {
                    return None;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
        }
        Some(score)
    }
}

struct EucJpCandidate {
    decoder: Decoder,
    non_ascii_seen: bool,
    prev: LatinCj,
    prev_byte: u8,
    prev_prev_byte: u8,
}

impl EucJpCandidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> Option<i64> {
        let mut score = 0i64;
        let mut src = [0u8];
        let mut dst = [0u16; 2];
        for &b in buffer {
            src[0] = b;
            let (result, read, written) = self
                .decoder
                .decode_to_utf16_without_replacement(&src, &mut dst, false);
            if written > 0 {
                let u = dst[0];
                if !self.non_ascii_seen && u >= 0x80 {
                    self.non_ascii_seen = true;
                    if u >= 0xFF61 && u <= 0xFF9F {
                        return None;
                    }
                    if u >= 0x3040 && u < 0x3100 {
                        // Remove the kana advantage over initial Big5
                        // hanzi.
                        score += EUC_JP_INITIAL_KANA_PENALTY;
                    }
                }
                if (u >= u16::from(b'a') && u <= u16::from(b'z'))
                    || (u >= u16::from(b'A') && u <= u16::from(b'Z'))
                {
                    if self.prev == LatinCj::Cj {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::AsciiLetter;
                } else if u >= 0xFF61 && u <= 0xFF9F {
                    score += HALF_WIDTH_KATAKANA_PENALTY;
                    self.prev = LatinCj::Other;
                } else if (u >= 0x3041 && u <= 0x3093) || (u >= 0x30A1 && u <= 0x30F6) {
                    match u {
                        0x3090 // hiragana wi
                        | 0x3091 // hiragana we
                        | 0x30F0 // katakana wi
                        | 0x30F1 // katakana we
                        => {
                            // Remove advantage over Big5 Hanzi
                            score += EUC_JP_SCORE_PER_NEAR_OBSOLETE_KANA;
                        }
                        _ => {
                            score += EUC_JP_SCORE_PER_KANA;
                        }
                    }
                    if self.prev == LatinCj::AsciiLetter {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::Cj;
                } else if (u >= 0x3400 && u < 0xA000) || (u >= 0xF900 && u < 0xFB00) {
                    if self.prev_prev_byte == 0x8F {
                        score += EUC_JP_SCORE_PER_OTHER_KANJI;
                    } else if self.prev_byte < 0xD0 {
                        score += EUC_JP_SCORE_PER_LEVEL_1_KANJI;
                        score += cjk_extra_score(u, &data::DETECTOR_DATA.frequent_kanji);
                    } else {
                        score += EUC_JP_SCORE_PER_LEVEL_2_KANJI;
                    }
                    if self.prev == LatinCj::AsciiLetter {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::Cj;
                } else {
                    match u {
                        0x3000 // Distinct from Korean, space
                        | 0x3001 // Distinct from Korean, enumeration comma
                        | 0x3002 // Distinct from Korean, full stop
                        | 0xFF08 // Distinct from Korean, parenthesis
                        | 0xFF09 // Distinct from Korean, parenthesis
                        => {
                            score += CJ_PUNCTUATION;
                        }
                        0..=0x7F => {}
                        _ => {
                            score += CJK_OTHER;
                        }
                    }
                    self.prev = LatinCj::Other;
                }
            }
            match result {
                DecoderResult::InputEmpty => {
                    assert_eq!(read, 1);
                }
                DecoderResult::Malformed(_, _) => {
                    return None;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
            self.prev_prev_byte = self.prev_byte;
            self.prev_byte = b;
        }
        if last {
            let (result, _, _) = self
                .decoder
                .decode_to_utf16_without_replacement(b"", &mut dst, true);
            match result {
                DecoderResult::InputEmpty => {}
                DecoderResult::Malformed(_, _) => {
                    return None;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
        }
        Some(score)
    }
}

struct Big5Candidate {
    decoder: Decoder,
    prev: LatinCj,
    prev_byte: u8,
}

impl Big5Candidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> Option<i64> {
        let mut score = 0i64;
        let mut src = [0u8];
        let mut dst = [0u16; 2];
        for &b in buffer {
            src[0] = b;
            let (result, read, written) = self
                .decoder
                .decode_to_utf16_without_replacement(&src, &mut dst, false);
            if written == 1 {
                let u = dst[0];
                if (u >= u16::from(b'a') && u <= u16::from(b'z'))
                    || (u >= u16::from(b'A') && u <= u16::from(b'Z'))
                {
                    if self.prev == LatinCj::Cj {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::AsciiLetter;
                } else if (u >= 0x3400 && u < 0xA000) || (u >= 0xF900 && u < 0xFB00) {
                    match self.prev_byte {
                        0xA4..=0xC6 => {
                            score += BIG5_SCORE_PER_LEVEL_1_HANZI;
                            // score += cjk_extra_score(u, &data::DETECTOR_DATA.frequent_traditional);
                        }
                        _ => {
                            score += BIG5_SCORE_PER_OTHER_HANZI;
                        }
                    }
                    if self.prev == LatinCj::AsciiLetter {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::Cj;
                } else {
                    match u {
                        0x3000 // Distinct from Korean, space
                        | 0x3001 // Distinct from Korean, enumeration comma
                        | 0x3002 // Distinct from Korean, full stop
                        | 0xFF08 // Distinct from Korean, parenthesis
                        | 0xFF09 // Distinct from Korean, parenthesis
                        | 0xFF01 // Distinct from Japanese, exclamation
                        | 0xFF0C // Distinct from Japanese, comma
                        | 0xFF1B // Distinct from Japanese, semicolon
                        | 0xFF1F // Distinct from Japanese, question
                        => {
                            // Not really needed for CJK distinction
                            // but let's give non-zero score for these
                            // common byte pairs anyway.
                            score += CJ_PUNCTUATION;
                        }
                        0..=0x7F => {}
                        _ => {
                            score += CJK_OTHER;
                        }
                    }
                    self.prev = LatinCj::Other;
                }
            } else if written == 2 {
                if dst[0] == 0xCA || dst[0] == 0xEA {
                    score += CJK_OTHER;
                    self.prev = LatinCj::Other;
                } else {
                    assert!(dst[0] >= 0xD480 && dst[0] < 0xD880);
                    score += BIG5_SCORE_PER_OTHER_HANZI;
                    if self.prev == LatinCj::AsciiLetter {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinCj::Cj;
                }
            }
            match result {
                DecoderResult::InputEmpty => {
                    assert_eq!(read, 1);
                }
                DecoderResult::Malformed(_, _) => {
                    return None;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
            self.prev_byte = b;
        }
        if last {
            let (result, _, _) = self
                .decoder
                .decode_to_utf16_without_replacement(b"", &mut dst, true);
            match result {
                DecoderResult::InputEmpty => {}
                DecoderResult::Malformed(_, _) => {
                    return None;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
        }
        Some(score)
    }
}

struct EucKrCandidate {
    decoder: Decoder,
    prev_was_euc_range: bool,
    prev: LatinKorean,
    current_word_len: u64,
}

impl EucKrCandidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> Option<i64> {
        let mut score = 0i64;
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
                        LatinKorean::Hangul | LatinKorean::Hanja => {
                            score += CJK_LATIN_ADJACENCY_PENALTY;
                        }
                        _ => {}
                    }
                    self.prev = LatinKorean::AsciiLetter;
                    self.current_word_len = 0;
                } else if u >= 0xAC00 && u <= 0xD7A3 {
                    if self.prev_was_euc_range && in_euc_range {
                        score += EUC_KR_SCORE_PER_EUC_HANGUL;
                        score += cjk_extra_score(u, &data::DETECTOR_DATA.frequent_hangul);
                    } else {
                        score += EUC_KR_SCORE_PER_NON_EUC_HANGUL;
                    }
                    if self.prev == LatinKorean::AsciiLetter {
                        score += CJK_LATIN_ADJACENCY_PENALTY;
                    }
                    self.prev = LatinKorean::Hangul;
                    self.current_word_len += 1;
                    if self.current_word_len > 5 {
                        score += EUC_KR_LONG_WORD_PENALTY;
                    }
                } else if (u >= 0x4E00 && u < 0xAC00) || (u >= 0xF900 && u <= 0xFA0B) {
                    score += EUC_KR_SCORE_PER_HANJA;
                    match self.prev {
                        LatinKorean::AsciiLetter => {
                            score += CJK_LATIN_ADJACENCY_PENALTY;
                        }
                        LatinKorean::Hangul => {
                            score += EUC_KR_HANJA_AFTER_HANGUL_PENALTY;
                        }
                        _ => {}
                    }
                    self.prev = LatinKorean::Hanja;
                    self.current_word_len += 1;
                    if self.current_word_len > 5 {
                        score += EUC_KR_LONG_WORD_PENALTY;
                    }
                } else {
                    if u >= 0x80 {
                        score += CJK_OTHER;
                    }
                    self.prev = LatinKorean::Other;
                    self.current_word_len = 0;
                }
            }
            match result {
                DecoderResult::InputEmpty => {
                    assert_eq!(read, 1);
                }
                DecoderResult::Malformed(_, _) => {
                    return None;
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
                    return None;
                }
                DecoderResult::OutputFull => {
                    unreachable!();
                }
            }
        }
        Some(score)
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
    Shift(ShiftJisCandidate),
    EucJp(EucJpCandidate),
    EucKr(EucKrCandidate),
    Big5(Big5Candidate),
    Gbk(GbkCandidate),
}

impl InnerCandidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> Option<i64> {
        match self {
            InnerCandidate::Latin(c) => {
                if let Some(new_score) = c.feed(buffer) {
                    if last {
                        // Treat EOF as space-like
                        if let Some(additional_score) = c.feed(b" ") {
                            Some(new_score + additional_score)
                        } else {
                            None
                        }
                    } else {
                        Some(new_score)
                    }
                } else {
                    None
                }
            }
            InnerCandidate::NonLatinCased(c) => {
                if let Some(new_score) = c.feed(buffer) {
                    if last {
                        // Treat EOF as space-like
                        if let Some(additional_score) = c.feed(b" ") {
                            Some(new_score + additional_score)
                        } else {
                            None
                        }
                    } else {
                        Some(new_score)
                    }
                } else {
                    None
                }
            }
            InnerCandidate::Caseless(c) => {
                if let Some(new_score) = c.feed(buffer) {
                    if last {
                        // Treat EOF as space-like
                        if let Some(additional_score) = c.feed(b" ") {
                            Some(new_score + additional_score)
                        } else {
                            None
                        }
                    } else {
                        Some(new_score)
                    }
                } else {
                    None
                }
            }
            InnerCandidate::ArabicFrench(c) => {
                if let Some(new_score) = c.feed(buffer) {
                    if last {
                        // Treat EOF as space-like
                        if let Some(additional_score) = c.feed(b" ") {
                            Some(new_score + additional_score)
                        } else {
                            None
                        }
                    } else {
                        Some(new_score)
                    }
                } else {
                    None
                }
            }
            InnerCandidate::Logical(c) => {
                if let Some(new_score) = c.feed(buffer) {
                    if last {
                        // Treat EOF as space-like
                        if let Some(additional_score) = c.feed(b" ") {
                            Some(new_score + additional_score)
                        } else {
                            None
                        }
                    } else {
                        Some(new_score)
                    }
                } else {
                    None
                }
            }
            InnerCandidate::Visual(c) => {
                if let Some(new_score) = c.feed(buffer) {
                    if last {
                        // Treat EOF as space-like
                        if let Some(additional_score) = c.feed(b" ") {
                            Some(new_score + additional_score)
                        } else {
                            None
                        }
                    } else {
                        Some(new_score)
                    }
                } else {
                    None
                }
            }
            InnerCandidate::Utf8(c) => c.feed(buffer, last),
            InnerCandidate::Shift(c) => c.feed(buffer, last),
            InnerCandidate::EucJp(c) => c.feed(buffer, last),
            InnerCandidate::EucKr(c) => c.feed(buffer, last),
            InnerCandidate::Big5(c) => c.feed(buffer, last),
            InnerCandidate::Gbk(c) => c.feed(buffer, last),
        }
    }
}

struct Candidate {
    inner: InnerCandidate,
    score: Option<i64>,
}

impl Candidate {
    fn feed(&mut self, buffer: &[u8], last: bool) {
        if let Some(old_score) = self.score {
            if let Some(new_score) = self.inner.feed(buffer, last) {
                self.score = Some(old_score + new_score);
            } else {
                self.score = None;
            }
        }
    }

    fn new_latin(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::Latin(LatinCandidate::new(data)),
            score: Some(0),
        }
    }

    fn new_non_latin_cased(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::NonLatinCased(NonLatinCasedCandidate::new(data)),
            score: Some(0),
        }
    }

    fn new_caseless(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::Caseless(CaselessCandidate::new(data)),
            score: Some(0),
        }
    }

    fn new_arabic_french(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::ArabicFrench(ArabicFrenchCandidate::new(data)),
            score: Some(0),
        }
    }

    fn new_logical(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::Logical(LogicalCandidate::new(data)),
            score: Some(0),
        }
    }

    fn new_visual(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::Visual(VisualCandidate::new(data)),
            score: Some(0),
        }
    }

    fn new_utf_8() -> Self {
        Candidate {
            inner: InnerCandidate::Utf8(Utf8Candidate {
                decoder: UTF_8.new_decoder_without_bom_handling(),
            }),
            score: Some(0),
        }
    }

    fn new_shift_jis() -> Self {
        Candidate {
            inner: InnerCandidate::Shift(ShiftJisCandidate {
                decoder: SHIFT_JIS.new_decoder_without_bom_handling(),
                non_ascii_seen: false,
                prev: LatinCj::Other,
                prev_byte: 0,
                pending_score: None,
            }),
            score: Some(0),
        }
    }

    fn new_euc_jp() -> Self {
        Candidate {
            inner: InnerCandidate::EucJp(EucJpCandidate {
                decoder: EUC_JP.new_decoder_without_bom_handling(),
                non_ascii_seen: false,
                prev: LatinCj::Other,
                prev_byte: 0,
                prev_prev_byte: 0,
            }),
            score: Some(0),
        }
    }

    fn new_euc_kr() -> Self {
        Candidate {
            inner: InnerCandidate::EucKr(EucKrCandidate {
                decoder: EUC_KR.new_decoder_without_bom_handling(),
                prev_was_euc_range: false,
                prev: LatinKorean::Other,
                current_word_len: 0,
            }),
            score: Some(0),
        }
    }

    fn new_big5() -> Self {
        Candidate {
            inner: InnerCandidate::Big5(Big5Candidate {
                decoder: BIG5.new_decoder_without_bom_handling(),
                prev: LatinCj::Other,
                prev_byte: 0,
            }),
            score: Some(0),
        }
    }

    fn new_gbk() -> Self {
        Candidate {
            inner: InnerCandidate::Gbk(GbkCandidate {
                decoder: GBK.new_decoder_without_bom_handling(),
                prev: LatinCj::Other,
                prev_byte: 0,
            }),
            score: Some(0),
        }
    }

    fn score(&self, _: Tld) -> Option<i64> {
        match &self.inner {
            InnerCandidate::NonLatinCased(c) => {
                if c.longest_word < 2 {
                    return None;
                }
            }
            InnerCandidate::Caseless(c) => {
                if c.longest_word < 2 {
                    return None;
                }
            }
            InnerCandidate::ArabicFrench(c) => {
                if c.longest_word < 2 {
                    return None;
                }
            }
            InnerCandidate::Logical(c) => {
                if c.longest_word < 2 {
                    return None;
                }
            }
            InnerCandidate::Visual(c) => {
                if c.longest_word < 2 {
                    return None;
                }
            }
            _ => {}
        }
        self.score
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
            InnerCandidate::Utf8(_) => {
                return UTF_8;
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
    candidates: [Candidate; 26],
    non_ascii_seen: u64,
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

    pub fn feed(&mut self, buffer: &[u8], last: bool) -> bool {
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
                return self.non_ascii_seen != 0;
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
        self.non_ascii_seen != 0
    }

    pub fn guess(&self, tld: Option<&[u8]>, allow_utf8: bool) -> &'static Encoding {
        let tld_type = tld.map_or(Tld::Generic, classify_tld);

        if self.non_ascii_seen == 0 && self.esc_seen
        // XXX scan for the rest of escape
        {
            return ISO_2022_JP;
        }

        if allow_utf8
            && self.candidates[Self::UTF_8_INDEX].score.is_some()
            && self.non_ascii_seen > 0
        {
            return UTF_8;
        }

        let mut encoding = WINDOWS_1252;
        let mut max = 0i64;
        for candidate in (&self.candidates[Self::FIRST_NORMAL..]).iter() {
            if let Some(score) = candidate.score(tld_type) {
                if score > max {
                    max = score;
                    encoding = candidate.encoding();
                }
            }
        }
        let visual = &self.candidates[Self::VISUAL_INDEX];
        if let Some(visual_score) = visual.score(tld_type) {
            if visual_score > max
                && visual.plausible_punctuation()
                    > self.candidates[Self::LOGICAL_INDEX].plausible_punctuation()
            {
                // max = visual_score;
                encoding = ISO_8859_8;
            }
        }

        encoding
    }

    // XXX Test-only API
    pub fn find_score(&self, encoding: &'static Encoding) -> Option<i64> {
        for candidate in self.candidates.iter() {
            if encoding == candidate.encoding() {
                return candidate.score(Tld::Generic);
            }
        }
        Some(0)
    }

    const UTF_8_INDEX: usize = 0;

    // const SHIFT_JIS_INDEX: usize = 2;

    // const EUC_JP_INDEX: usize = 3;

    // const EUC_KR_INDEX: usize = 4;

    // const BIG5_INDEX: usize = 5;

    // const GBK_INDEX: usize = 6;

    // const FIRST_SINGLE_BYTE: usize = 7;

    const FIRST_NORMAL: usize = 2;

    const VISUAL_INDEX: usize = 1;

    const LOGICAL_INDEX: usize = 15;

    // const WINDOWS_1250_SINGLE_BYTE: usize = 10;

    // const WINDOWS_1251_SINGLE_BYTE: usize = 9;

    // const WINDOWS_1252_SINGLE_BYTE: usize = 8;

    // const WINDOWS_1253_SINGLE_BYTE: usize = 16;

    // const WINDOWS_1254_SINGLE_BYTE: usize = 13;

    // const WINDOWS_1255_SINGLE_BYTE: usize = 15;

    // const WINDOWS_1256_SINGLE_BYTE: usize = 12;

    // const WINDOWS_1257_SINGLE_BYTE: usize = 18;

    // const WINDOWS_1258_SINGLE_BYTE: usize = 22;

    // const ISO_8859_3_SINGLE_BYTE: usize = 11;

    // const ISO_8859_4_SINGLE_BYTE: usize = 23;

    // const ISO_8859_5_SINGLE_BYTE: usize = 24;

    // const ISO_8859_6_SINGLE_BYTE: usize = 21;

    pub fn new() -> Self {
        EncodingDetector {
            candidates: [
                Candidate::new_utf_8(),                                                // 0
                Candidate::new_visual(&SINGLE_BYTE_DATA[ISO_8859_8_INDEX]),            // 1
                Candidate::new_gbk(),                                                  // 2
                Candidate::new_euc_jp(),                                               // 3
                Candidate::new_euc_kr(),                                               // 4
                Candidate::new_shift_jis(),                                            // 5
                Candidate::new_big5(),                                                 // 6
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1252_INDEX]),           // 7
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[WINDOWS_1251_INDEX]), // 8
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1250_INDEX]),           // 9
                Candidate::new_latin(&SINGLE_BYTE_DATA[ISO_8859_2_INDEX]),             // 10
                Candidate::new_arabic_french(&SINGLE_BYTE_DATA[WINDOWS_1256_INDEX]),   // 11
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1252_ICELANDIC_INDEX]), // 12
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1254_INDEX]),           // 13
                Candidate::new_caseless(&SINGLE_BYTE_DATA[WINDOWS_874_INDEX]),         // 14
                Candidate::new_logical(&SINGLE_BYTE_DATA[WINDOWS_1255_INDEX]),         // 15
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[WINDOWS_1253_INDEX]), // 16
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[ISO_8859_7_INDEX]),   // 17
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1257_INDEX]),           // 18
                Candidate::new_latin(&SINGLE_BYTE_DATA[ISO_8859_13_INDEX]),            // 19
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[KOI8_U_INDEX]),       // 20
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[IBM866_INDEX]),       // 21
                Candidate::new_caseless(&SINGLE_BYTE_DATA[ISO_8859_6_INDEX]),          // 22
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1258_INDEX]),           // 23
                Candidate::new_latin(&SINGLE_BYTE_DATA[ISO_8859_4_INDEX]),             // 24
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[ISO_8859_5_INDEX]),   // 25
            ],
            non_ascii_seen: 0,
            last_before_non_ascii: None,
            esc_seen: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(input: &str, encoding: &'static Encoding) {
        let (bytes, _, _) = encoding.encode(input);
        let mut det = EncodingDetector::new();
        det.feed(&bytes, true);
        let enc = det.guess(None, false);
        let (decoded, _) = enc.decode_without_bom_handling(&bytes);
        println!("{:?}", decoded);
        assert_eq!(enc, encoding);
    }

    #[test]
    fn test_empty() {
        let mut det = EncodingDetector::new();
        let seen_non_ascii = det.feed(b"", true);
        let enc = det.guess(None, true);
        assert_eq!(enc, WINDOWS_1252);
        assert!(!seen_non_ascii);
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

    // #[test]
    // fn test_th() {
    //     check("ไทย", WINDOWS_874);
    // }

    #[test]
    fn test_foo() {
        check("Straße", WINDOWS_1252);
    }
}
