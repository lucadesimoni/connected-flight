// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! `application/x-www-form-urlencoded` decoding, shared by the maintenance
//! page and the ground service. Malformed escapes are kept literally;
//! invalid UTF-8 is replaced. Never panics (robustness-tested).

pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `a=1&b=x+y` -> [("a","1"), ("b","x y")]. Pairs without `=` are skipped.
pub fn parse_form(body: &str) -> Vec<(String, String)> {
    body.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (decode(k), decode(v))).collect()
}

/// First value of `key`, or "".
pub fn get<'a>(form: &'a [(String, String)], key: &str) -> &'a str {
    form.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_forms() {
        let f = parse_form("token=a%2Bb+c&on=1&x=%zz&bad&e=%E2%82%AC&t=%4");
        assert_eq!(get(&f, "token"), "a+b c");
        assert_eq!(get(&f, "on"), "1");
        assert_eq!(get(&f, "x"), "%zz");
        assert_eq!(get(&f, "e"), "€");
        assert_eq!(get(&f, "t"), "%4");
        assert_eq!(get(&f, "missing"), "");
        assert_eq!(decode("%ff%fe"), "\u{fffd}\u{fffd}");
    }
}
