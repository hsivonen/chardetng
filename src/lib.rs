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
const IMPLAUSIBILITY_PENALTY: i64 = -200;
const IMPLAUSIBLE_CASE_TRANSITION_PENALTY: i64 = -100;

#[derive(PartialEq)]
enum Case {
    Space,
    Upper,
    Lower,
}

struct NonLatinCasedCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
    prev_case: Case,
    prev_ascii: bool,
}

impl NonLatinCasedCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        NonLatinCasedCandidate {
            data: data,
            score: 0,
            prev: 0,
            prev_case: Case::Space,
            prev_ascii: true,
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
            let case = if class == 0 {
                Case::Space
            } else if (class >> 7) == 0 {
                Case::Lower
            } else {
                Case::Upper
            };
            if !(self.prev_ascii && ascii) && self.prev_case == Case::Lower && case == Case::Upper {
                self.score += IMPLAUSIBLE_CASE_TRANSITION_PENALTY;
            }
            self.prev_ascii = ascii;
            self.prev_case = case;
            self.score += self.data.score(caseless_class, self.prev);
            self.prev = caseless_class;
        }
        false
    }
}

struct LatinCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
    prev_case: Case,
    prev_non_ascii: u32,
}

impl LatinCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        LatinCandidate {
            data: data,
            score: 0,
            prev: 0,
            prev_case: Case::Space,
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
            let ascii = self.data.is_ascii_class(caseless_class);
            let case = if class == 0 {
                Case::Space
            } else if (class >> 7) == 0 {
                Case::Lower
            } else {
                Case::Upper
            };
            let non_ascii_penalty = match self.prev_non_ascii {
                0 | 1 | 2 => 0,
                3 => -5,
                4 => -20,
                _ => -200,
            };
            self.score += non_ascii_penalty;
            if !((self.prev_non_ascii == 0) && ascii)
                && self.prev_case == Case::Lower
                && case == Case::Upper
            {
                // XXX How bad is this for Irish Gaelic?
                self.score += IMPLAUSIBLE_CASE_TRANSITION_PENALTY;
            }
            if ascii {
                self.prev_non_ascii = 0;
            } else {
                self.prev_non_ascii += 1;
            }
            self.prev_case = case;
            self.score += self.data.score(caseless_class, self.prev);
            self.prev = caseless_class;
        }
        false
    }
}

struct ArabicFrenchCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
    prev_case: Case,
    prev_ascii: bool,
}

impl ArabicFrenchCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        ArabicFrenchCandidate {
            data: data,
            score: 0,
            prev: 0,
            prev_case: Case::Space,
            prev_ascii: true,
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
            let case = if caseless_class != 0x7E {
                Case::Space
            } else if (class >> 7) == 0 {
                Case::Lower
            } else {
                Case::Upper
            };
            if !(self.prev_ascii && ascii) && self.prev_case == Case::Lower && case == Case::Upper {
                self.score += IMPLAUSIBLE_CASE_TRANSITION_PENALTY;
            }
            self.prev_ascii = ascii;
            self.prev_case = case;
            self.score += self.data.score(caseless_class, self.prev);
            self.prev = caseless_class;
        }
        false
    }
}

struct CaselessCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
}

impl CaselessCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        CaselessCandidate {
            data: data,
            score: 0,
            prev: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> bool {
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return true;
            }
            let caseless_class = class & 0x7F;
            self.score += self.data.score(caseless_class, self.prev);
            self.prev = caseless_class;
        }
        false
    }
}

struct LogicalCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
    plausible_punctuation: u64,
}

impl LogicalCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        LogicalCandidate {
            data: data,
            score: 0,
            prev: 0,
            plausible_punctuation: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> bool {
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return true;
            }
            let caseless_class = class & 0x7F;
            if !(self.prev == 0 || self.prev == 0x7E) && caseless_class == 1 {
                self.plausible_punctuation += 1;
            }
            self.score += self.data.score(caseless_class, self.prev);
            self.prev = caseless_class;
        }
        false
    }
}

struct VisualCandidate {
    data: &'static SingleByteData,
    score: i64,
    prev: u8,
    plausible_punctuation: u64,
}

impl VisualCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        VisualCandidate {
            data: data,
            score: 0,
            prev: 0,
            plausible_punctuation: 0,
        }
    }

    fn feed(&mut self, buffer: &[u8]) -> bool {
        for &b in buffer {
            let class = self.data.classify(b);
            if class == 255 {
                return true;
            }
            let caseless_class = class & 0x7F;
            if !(caseless_class == 0 || caseless_class == 0x7E) && self.prev == 1 {
                self.plausible_punctuation += 1;
            }
            self.score += self.data.score(self.prev, caseless_class);
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
                    || (u >= u16::from(b'a') && u <= u16::from(b'z'))
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
                        0xE78D...0xE793
                        | 0xE794...0xE796
                        | 0xE816...0xE818
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
                    || (u >= u16::from(b'a') && u <= u16::from(b'z'))
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
    non_ascii_seen: bool,
    cjk_pairs: u64,
    ascii_cjk_pairs: u64,
    prev: AsciiCjk,
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
                    || (u >= u16::from(b'a') && u <= u16::from(b'z'))
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
    prev: AsciiCjk,
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
                    || (u >= u16::from(b'a') && u <= u16::from(b'z'))
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
                    || (u >= u16::from(b'a') && u <= u16::from(b'z'))
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
                decoder: BIG5.new_decoder_without_bom_handling(),
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

    fn single_byte_score(&self) -> i64 {
        match &self.inner {
            InnerCandidate::Latin(c) => {
                return c.score;
            }
            InnerCandidate::NonLatinCased(c) => {
                return c.score;
            }
            InnerCandidate::Caseless(c) => {
                return c.score;
            }
            InnerCandidate::ArabicFrench(c) => {
                return c.score;
            }
            InnerCandidate::Logical(c) => {
                return c.score;
            }
            InnerCandidate::Visual(c) => {
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
            _ => {
                unreachable!();
            }
        }
    }

    fn increment_score(&mut self) {
        match &mut self.inner {
            InnerCandidate::Latin(c) => {
                c.score += 1;
            }
            InnerCandidate::NonLatinCased(c) => {
                c.score += 1;
            }
            InnerCandidate::Caseless(c) => {
                c.score += 1;
            }
            InnerCandidate::ArabicFrench(c) => {
                c.score += 1;
            }
            InnerCandidate::Logical(c) => {
                c.score += 1;
            }
            InnerCandidate::Visual(c) => {
                c.score += 1;
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
            InnerCandidate::Shift(c) => {
                if c.cjk_pairs > 50 {
                    (c.cjk_pairs, c.ascii_cjk_pairs)
                } else {
                    return false;
                }
            }
            InnerCandidate::EucJp(c) => {
                if c.cjk_pairs > 50 {
                    (c.cjk_pairs, c.ascii_cjk_pairs)
                } else {
                    return false;
                }
            }
            InnerCandidate::EucKr(c) => {
                if c.cjk_pairs > 50 {
                    (c.cjk_pairs, c.ascii_cjk_pairs)
                } else {
                    return false;
                }
            }
            InnerCandidate::Big5(c) => {
                if c.cjk_pairs > 50 {
                    (c.cjk_pairs, c.ascii_cjk_pairs)
                } else {
                    return false;
                }
            }
            InnerCandidate::Gbk(c) => {
                if c.cjk_pairs > 50 {
                    (c.cjk_pairs, c.ascii_cjk_pairs)
                } else {
                    return false;
                }
            }
            _ => {
                unreachable!();
            }
        };
        ascii_cjk_pairs < cjk_pairs / 128 // Arbitrary allowance
    }

    fn pua_ratio_ok(&self) -> bool {
        let (cjk_pairs, pua) = match &self.inner {
            InnerCandidate::Shift(c) => (c.cjk_pairs, c.pua),
            InnerCandidate::Gbk(c) => (c.cjk_pairs, c.pua),
            _ => {
                unreachable!();
            }
        };
        pua < cjk_pairs / 256 // Arbitrary allowance
    }

    fn gbk_euc_ratio_ok(&self) -> bool {
        match &self.inner {
            InnerCandidate::Gbk(c) => {
                // Arbitrary allowance
                c.non_euc_range < c.euc_range / 256
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

        let utf = !self.candidates[Self::UTF_8_INDEX].disqualified;

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
        let mut max = i64::min_value();
        for candidate in (&self.candidates[Self::FIRST_NORMAL_SINGLE_BYTE..]).iter() {
            let score = candidate.single_byte_score();
            println!(
                "{} {} {}",
                candidate.encoding().name(),
                score,
                candidate.disqualified
            );
            if !candidate.disqualified && score > max {
                max = score;
                encoding = candidate.encoding();
            }
        }
        let visual = &self.candidates[Self::VISUAL_INDEX];
        let visual_score = visual.single_byte_score();
        if !visual.disqualified
            && visual_score > max
            && visual.plausible_punctuation()
                > self.candidates[Self::LOGICAL_INDEX].plausible_punctuation()
        {
            max = visual_score;
            encoding = ISO_8859_8;
        }
        if let Some(fallback) = self.fallback {
            if fallback == encoding {
                return (encoding, utf);
            }
            if fallback == WINDOWS_1250 && encoding == ISO_8859_2 {
                return (encoding, utf);
            }
            if fallback == ISO_8859_2 && encoding == WINDOWS_1250 {
                return (encoding, utf);
            }
            if fallback == WINDOWS_1253 && encoding == ISO_8859_7 {
                return (encoding, utf);
            }
            if fallback == ISO_8859_7 && encoding == WINDOWS_1253 {
                return (encoding, utf);
            }
            if fallback == WINDOWS_1255 && encoding == ISO_8859_8 {
                return (encoding, utf);
            }
            if fallback == ISO_8859_8 && encoding == WINDOWS_1255 {
                return (encoding, utf);
            }
            if fallback == WINDOWS_1256 && encoding == ISO_8859_6 {
                return (encoding, utf);
            }
            if fallback == ISO_8859_6 && encoding == WINDOWS_1256 {
                return (encoding, utf);
            }
            if fallback == WINDOWS_1257 && encoding == ISO_8859_4 {
                return (encoding, utf);
            }
            if fallback == ISO_8859_4 && encoding == WINDOWS_1257 {
                return (encoding, utf);
            }
            if (fallback == WINDOWS_1251
                || fallback == KOI8_U
                || fallback == ISO_8859_5
                || fallback == IBM866)
                && (encoding == WINDOWS_1251
                    || encoding == KOI8_U
                    || encoding == ISO_8859_5
                    || encoding == IBM866)
            {
                return (encoding, utf);
            }
        }
        if max < 0 {
            // Single-byte result was garbage and the fallback wasn't
            // the best fit.
            encoding = self.fallback.unwrap_or(WINDOWS_1252);
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
        if !self.candidates[Self::SHIFT_JIS_INDEX].ascii_pair_ratio_ok() {
            euc_jp_ok = false;
        }

        if shift_jis_ok {
            return (SHIFT_JIS, utf);
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
        if other_hangul >= modern_hangul / 256 || hanja >= modern_hangul / 128 {
            euc_kr_ok = false;
        }
        if gbk_ok {
            if euc_kr_ok {
                return (EUC_KR, utf);
            }
            return (GBK, utf);
        } else if euc_jp_ok {
            // Arbitrary allowance for standalone jamo, which overlap
            // the kana range.
            if euc_kr_ok && euc_jp_kana < modern_hangul / 256 {
                return (EUC_KR, utf);
            }
            return (EUC_JP, utf);
        } else if euc_kr_ok {
            return (EUC_KR, utf);
        } else if big5_ok {
            return (BIG5, utf);
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
        for candidate in (&self.candidates[Self::FIRST_SINGLE_BYTE..]).iter() {
            if encoding == candidate.encoding() {
                return (candidate.single_byte_score(), candidate.disqualified);
            }
        }
        unreachable!();
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

    pub fn new_with_fallback(fallback: Option<&'static Encoding>) -> Self {
        let mut det = EncodingDetector {
            candidates: [
                Candidate::new_utf_8(),
                Candidate::new_iso_2022_jp(),
                Candidate::new_shift_jis(),
                Candidate::new_euc_jp(),
                Candidate::new_euc_kr(),
                Candidate::new_big5(),
                Candidate::new_gbk(),
                Candidate::new_visual(&SINGLE_BYTE_DATA[ISO_8859_8_INDEX]),
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1252_INDEX]),
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[WINDOWS_1251_INDEX]),
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1250_INDEX]),
                Candidate::new_latin(&SINGLE_BYTE_DATA[ISO_8859_2_INDEX]),
                Candidate::new_arabic_french(&SINGLE_BYTE_DATA[WINDOWS_1256_INDEX]),
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1254_INDEX]),
                Candidate::new_caseless(&SINGLE_BYTE_DATA[WINDOWS_874_INDEX]),
                Candidate::new_logical(&SINGLE_BYTE_DATA[WINDOWS_1255_INDEX]),
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[WINDOWS_1253_INDEX]),
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[ISO_8859_7_INDEX]),
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1257_INDEX]),
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[KOI8_U_INDEX]),
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[IBM866_INDEX]),
                Candidate::new_caseless(&SINGLE_BYTE_DATA[ISO_8859_6_INDEX]),
                Candidate::new_latin(&SINGLE_BYTE_DATA[WINDOWS_1258_INDEX]),
                Candidate::new_latin(&SINGLE_BYTE_DATA[ISO_8859_4_INDEX]),
                Candidate::new_non_latin_cased(&SINGLE_BYTE_DATA[ISO_8859_5_INDEX]),
            ],
            non_ascii_seen: 0,
            fallback: fallback,
            last_before_non_ascii: None,
            esc_seen: false,
        };
        if let Some(fallback) = fallback {
            for single_byte in det.candidates[Self::FIRST_SINGLE_BYTE..].iter_mut() {
                if single_byte.encoding() == fallback {
                    single_byte.increment_score();
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
        assert!(utf);
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
