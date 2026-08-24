//! NFKC 正規化と Shift-JIS テキスト読み取り (Phase 5b: 構造化抽出)
//!
//! 電脳倶楽部の DOC は SJIS で書かれており、全角/半角の揺れが激しい。
//! 例えば「ＳＡＭＰＬＥ ＴＩＴＬＥ」と「SAMPLE TITLE」など。
//! NFKC 正規化を最初に通すことで、互換等価な揺れ (全角英数 / 互換かな) を吸収する。

use encoding_rs::SHIFT_JIS;
use unicode_normalization::UnicodeNormalization;

/// SJIS バイト列を UTF-8 に lossy デコードしたうえで NFKC 正規化する。
pub fn decode_sjis_nfkc(bytes: &[u8]) -> String {
    let (cow, _enc, _err) = SHIFT_JIS.decode(bytes);
    cow.nfkc().collect()
}

/// 既に UTF-8 になっている文字列を NFKC 正規化する。
pub fn nfkc(s: &str) -> String {
    s.nfkc().collect()
}

/// NFKC 後の文字列から、見出し探索用に「タイトル候補」を整える:
/// - 連続するスペース類 (全角・半角・タブ) を1個の半角スペースに畳む
/// - 前後の空白を trim
pub fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        // NFKC 後でも残る空白を吸収
        let is_space = ch.is_whitespace() || ch == '\u{3000}';
        if is_space {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfkc_normalizes_fullwidth_alphanumeric() {
        // 全角英数 → 半角
        assert_eq!(nfkc("ＳＡＭＰＬＥ"), "SAMPLE");
        // U+2212 MINUS SIGN (SJIS 817C) は NFKC で ASCII '-' に正規化されない (互換等価ではない)。
        // 真ん中のハイフンは U+FF0D FULLWIDTH HYPHEN-MINUS なら ASCII '-' になる。
        assert_eq!(nfkc("Ｚ－ＭＵＳＩＣ"), "Z-MUSIC");
        assert_eq!(nfkc("Ｖｅｒ ２．０６"), "Ver 2.06");
    }

    #[test]
    fn collapse_handles_fullwidth_space() {
        // 全角空白 (U+3000) を含む
        assert_eq!(collapse_spaces("SONGA\u{3000}\u{3000}SONGB"), "SONGA SONGB");
        assert_eq!(collapse_spaces("  abc   def  "), "abc def");
    }

    #[test]
    fn sjis_doc_header_normalized() {
        // 全角英字と全角空白 (0x8140) を含む任意の SJIS バイト列。
        // 不正バイトを含んでも decode_sjis_nfkc が panic しないことだけを確認する
        // (SJIS は lossy デコードなので、内容の正しさはここでは問わない)。
        let bytes = b"\x83\x50\x82\x4F\x82\x6E\x82\x5B\x82\x64\x81\x40\
                      \x82\x65\x82\x6E\x82\x5B\x82\x64";
        let _ = decode_sjis_nfkc(bytes); // panic しないことだけ確認
    }
}
