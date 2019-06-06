use encoding_rs::Decoder;
use encoding_rs::DecoderResult;
use encoding_rs::Encoding;
use encoding_rs::WINDOWS_1252;

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
    Shift(ShiftJisCandidate),
    Iso2022(Iso2022Candidate),
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
            InnerCandidate::Shift(c) => {
                return c.feed(buffer, last);
            }
            InnerCandidate::Iso2022(c) => {
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

    fn single_byte_score(&self) -> u64 {
        match self.inner {
            InnerCandidate::Single(c) => {
                return c.score;
            }
            _ => {
                unreachable!();
            }
        }
    }

    fn encoding(&self) -> &'static Encoding {
        match self.inner {
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
    candidates: [Candidate; 18],
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

    pub fn new() -> Self {
        EncodingDetector {
            candidates: [
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
            last_before_non_ascii: None,
            esc_seen: false,
        }
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

    fn guess(&self) -> (&'static Encoding, bool) {
        let mut encoding = WINDOWS_1252;
        let mut max = 0;
        for candidate in self.candidates.iter() {
            let score = candidate.single_byte_score();
            if score > max {
                max = score;
                encoding = candidate.encoding();
            }
        }
        (encoding, false)
    }

    pub fn feed_without_guessing(&mut self, buffer: &[u8]) {
        self.feed_without_guessing_impl(buffer, false);
    }

    pub fn feed(&mut self, buffer: &[u8], last: bool) -> (&'static Encoding, bool, u64) {
        self.feed_without_guessing_impl(buffer, last);
        let (enc, utf) = self.guess();
        (enc, utf, self.non_ascii_seen)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
