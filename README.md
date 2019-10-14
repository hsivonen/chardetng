# chardetng

[![crates.io](https://meritbadge.herokuapp.com/chardetng)](https://crates.io/crates/chardetng)
[![docs.rs](https://docs.rs/chardetng/badge.svg)](https://docs.rs/chardetng/)
[![Apache 2 / MIT dual-licensed](https://img.shields.io/badge/license-Apache%202%20%2F%20MIT-blue.svg)](https://github.com/hsivonen/chardetng/blob/master/COPYRIGHT)

A character encoding detector for legacy Web content.

## Licensing

Please see the file named
[COPYRIGHT](https://github.com/hsivonen/chardetng/blob/master/COPYRIGHT).

## Documentation

Generated [API documentation](https://docs.rs/chardetng/) is available
online.

## Principle of Operation

In general `chardetng` prefers to do negative matching (rule out possibilities from the set of plausible encodings) than to do positive matching. Since negative matching is insufficient, there is positive matching, too.

* UTF-16BE and UTF-16LE are not possible outcomes: Detecting them belongs to the BOM layer.
* Latin single-byte encodings that have never been the default fallback in any locale configuration for any major browser are not possible outcomes.
* x-user-defined is for XHR and is not a possible outcome.
* x-mac-cyrillic is not possible outcome due to IE and Chrome not detecting it.
* KOI8-R is not a possible outcome due to it differing from KOI8-U only in box drawing, so guessing the KOI8 family always as KOI8-U is safer for the purpose of making sure that text is readable.
* ISO-2022-JP is matched if there is at least one ISO-2022-JP escape sequence and the stream as a whole is valid ISO-2022-JP (this implies no non-ASCII bytes).
* UTF-8 match is returned only as a secondary bit of information (to avoid non-interactive use of this information and Web developers depending on things appearing to work without having to take a UI action). UTF-8 is matched if the stream as a whole is valid UTF-8 and has non-ASCII bytes.
* A single encoding error disqualifies an encoding from possible outcomes. (Notably, as the length of the input increases, it becomes increasingly improbable for the input to be valid according to a legacy CJK encoding without being intended as such.)
* A single occurrence of a C1 control character disqualifies an encoding from possible outcomes.
* The first non-ASCII character being a half-width katakana character disqualifies an encoding. (This is _very_ effective for deciding between Shift_JIS and EUC-JP.)
* Having lots of characters whose bytes don't fit the original EUC rectangle is taken as an indication of Big5 relative to the EUC family.
* Having kana is taken as an indication of EUC-JP realative to the other EUC-family encodings.
* Staying within the modern Hangul area is taken as an indication of EUC-KR relative to the other EUC-family encodings.
* For encodings for bicameral scripts, having an upper-case letter follow a lower-case letter is penalized. (This is effective for deciding that something is not Greek or Cyrillic and also may have some residual benefit with characters in Latin encodings that are not in the obvious upper-case/lower-case byte locations.)
* For Latin encodings, having three non-ASCII letters in a row is penalized a little and having four or more is penalized a lot.
* For non-Latin encodings, having a non-Latin letter right next to a Latin letter is penalized.
* For single-byte encodings, having a character pair (excluding pairs where both characters are ASCII) that never occurs in the Wikipedias for the applicable languages is heavily penalized.
* For single-byte encodings, character pairs are given scores according to their relative frequencies in the applicable Wikipedias.
* For Arabic encodings, characters pairs where one character is alif are given an artificially lower score to avoid misdetecting pretty much everything as Arabic. (Pairs that have an alif in Arabic have an unusual relative frequency compared to letter pairs in other scripts.)
* Non-Latin single-byte encodings need to have at least one three-non-ASCII-letter word to qualify. (Otherwise, non-ASCII punctuation in short Latin inputs gets misdetected as non-Latin.)

* Ordinal indicators and digits
* Superscript number
* Inverted punctuation
* œæ

## Release Notes

### 0.1.0

* Initial release.
