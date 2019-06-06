use encoding_rs::Decoder;
use encoding_rs::DecoderResult;
use encoding_rs::Encoding;
use encoding_rs::BIG5;
use encoding_rs::EUC_JP;
use encoding_rs::EUC_KR;
use encoding_rs::GBK;
use encoding_rs::ISO_2022_JP;
use encoding_rs::SHIFT_JIS;
use encoding_rs::UTF_8;
use encoding_rs::WINDOWS_1251;
use encoding_rs::WINDOWS_1252;
use encoding_rs::WINDOWS_1253;

mod data;
use data::*;

struct SingleByteCandidate {
    data: &'static SingleByteData,
    score: u64,
    prev: u8,
}

impl SingleByteCandidate {
    fn new(data: &'static SingleByteData) -> Self {
        SingleByteCandidate {
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
            self.score += self.data.score(class, self.prev);
            self.prev = class;
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

struct GbkCandidate {
    decoder: Decoder,
}

impl GbkCandidate {
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
            	// Exclude Private Use Areas to make it less likely
            	// that arbitrary byte sequences match as valid.
                if u >= 0xE000 && u <= 0xF8FF {
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
                            return true;
                        }
                    }
                } else if u >= 0xDB80 && u <= 0xDBFF {
                	return true;
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

struct ShiftJisCandidate {
    decoder: Decoder,
    non_ascii_seen: bool,
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
            if !self.non_ascii_seen {
                for &u in dst[..written].iter() {
                    if u >= 0x80 {
                        self.non_ascii_seen = true;
                        if u >= 0xFF61 && u <= 0xFF9F {
                            return true;
                        }
                        break;
                    }
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
                if !self.non_ascii_seen {
                    if u >= 0x80 {
                        self.non_ascii_seen = true;
                        if u >= 0xFF61 && u <= 0xFF9F {
                            return true;
                        }
                    }
                }
                match u {
                    0x3041...0x3093 | 0x30A1...0x30F6 => {
                        self.kana += 1;
                    }
                    0xE400...0xFA2D => {
                        self.kanji += 1;
                    }
                    _ => {}
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

enum InnerCandidate {
    Single(SingleByteCandidate),
    Utf8(Utf8Candidate),
    Iso2022(Iso2022Candidate),
    Shift(ShiftJisCandidate),
    EucJp(EucJpCandidate),
    Gbk(GbkCandidate),
}

impl InnerCandidate {
    fn feed(&mut self, buffer: &[u8], last: bool) -> bool {
        match self {
            InnerCandidate::Single(c) => {
                return c.feed(buffer);
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

    fn new_single_byte(data: &'static SingleByteData) -> Self {
        Candidate {
            inner: InnerCandidate::Single(SingleByteCandidate::new(data)),
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
            }),
            disqualified: false,
        }
    }

    fn new_gbk() -> Self {
        Candidate {
            inner: InnerCandidate::Gbk(GbkCandidate {
                decoder: GBK.new_decoder_without_bom_handling(),
            }),
            disqualified: false,
        }
    }

    fn single_byte_score(&self) -> u64 {
        match &self.inner {
            InnerCandidate::Single(c) => {
                return c.score;
            }
            _ => {
                unreachable!();
            }
        }
    }

    fn encoding(&self) -> &'static Encoding {
        match &self.inner {
            InnerCandidate::Single(c) => {
                return c.data.encoding;
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
    candidates: [Candidate; 23],
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
            }
            start
        } else {
            0
        };
        self.feed_impl(&buffer[start..], last);
    }

    fn fallback_is_multibyte(&self) -> bool {
        if let Some(encoding) = self.fallback {
            !encoding.is_single_byte()
        } else {
            false
        }
    }

    fn guess(&self) -> (&'static Encoding, bool) {
        if self.non_ascii_seen == 0
            && self.esc_seen
            && !self.candidates[Self::ISO_2022_JP_INDEX].disqualified
        {
            return (ISO_2022_JP, false);
        }

        let utf = !self.candidates[Self::UTF_8_INDEX].disqualified;

        if self.fallback_is_multibyte() || self.non_ascii_seen > 50 {
            if let Some(fallback) = self.fallback {
                if fallback == EUC_KR && !self.candidates[Self::EUC_KR_INDEX].disqualified {
                    return (EUC_KR, utf);
                }
                if fallback == BIG5 {
                    if !self.candidates[Self::BIG5_INDEX].disqualified {
                        return (BIG5, utf);
                    }
                    if !self.candidates[Self::GBK_INDEX].disqualified {
                        return (GBK, utf);
                    }
                }
                if fallback == GBK {
                    if !self.candidates[Self::GBK_INDEX].disqualified {
                        return (GBK, utf);
                    }
                    if !self.candidates[Self::BIG5_INDEX].disqualified {
                        return (BIG5, utf);
                    }
                }
                if fallback == SHIFT_JIS {
                    if !self.candidates[Self::SHIFT_JIS_INDEX].disqualified {
                        return (SHIFT_JIS, utf);
                    }
                    if !self.candidates[Self::EUC_JP_INDEX].disqualified {
                        return (EUC_JP, utf);
                    }
                }
                if fallback == EUC_JP {
                    if !self.candidates[Self::EUC_JP_INDEX].disqualified {
                        return (EUC_JP, utf);
                    }
                    if !self.candidates[Self::SHIFT_JIS_INDEX].disqualified {
                        return (SHIFT_JIS, utf);
                    }
                }
            }
            if !self.candidates[Self::SHIFT_JIS_INDEX].disqualified {
                return (SHIFT_JIS, utf);
            }
        }

        let mut encoding = WINDOWS_1252;
        let mut max = 0;
        for candidate in (&self.candidates[Self::FIRST_SINGLE_BYTE..]).iter() {
            let score = candidate.single_byte_score();
            println!("{} {}", candidate.encoding().name(), score);
            if score > max {
                max = score;
                encoding = candidate.encoding();
            }
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

    const UTF_8_INDEX: usize = 0;

    const ISO_2022_JP_INDEX: usize = 1;

    const SHIFT_JIS_INDEX: usize = 2;

    const EUC_JP_INDEX: usize = 3;

    const EUC_KR_INDEX: usize = 4;

    const BIG5_INDEX: usize = 5;

    const GBK_INDEX: usize = 6;

    const FIRST_SINGLE_BYTE: usize = 7;

    pub fn new_with_fallback(fallback: Option<&'static Encoding>) -> Self {
        EncodingDetector {
            candidates: [
                Candidate::new_utf_8(),
                Candidate::new_iso_2022_jp(),
                Candidate::new_shift_jis(),
                Candidate::new_euc_jp(),

                Candidate::new_gbk(),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[WINDOWS_1252_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[WINDOWS_1251_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[WINDOWS_1250_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[ISO_8859_2_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[WINDOWS_1256_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[WINDOWS_1254_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[WINDOWS_874_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[WINDOWS_1255_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[WINDOWS_1253_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[ISO_8859_7_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[WINDOWS_1257_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[KOI8_U_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[IBM866_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[ISO_8859_6_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[WINDOWS_1258_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[ISO_8859_4_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[ISO_8859_5_INDEX]),
                Candidate::new_single_byte(&SINGLE_BYTE_DATA[ISO_8859_8_INDEX]),
            ],
            non_ascii_seen: 0,
            fallback: fallback,
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
        let (enc, _, _) = det.feed(&bytes, true);
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
        check("Määränpää", WINDOWS_1252);
    }

    #[test]
    fn test_ru() {
        check("Русский", WINDOWS_1251);
    }

    #[test]
    fn test_el() {
        check("Ελληνικά", WINDOWS_1253);
    }

}
