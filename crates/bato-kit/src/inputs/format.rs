//! The pure text logic of the inputs: `applyMask`, `maxLength`, JavaScript
//! number strings, `parseFloat`, and `Intl.NumberFormat` currency output for
//! the locales the template meets (English grouping; the symbol table is
//! Chromium's for those locales).

/// `applyMask(raw, mask)` (MaskedInput.tsx): `#` digit, `A` letter, `*`
/// alphanumeric; other mask characters are literals inserted as the raw
/// text reaches them.
pub fn apply_mask(raw: &str, mask: &str) -> String {
    let raw: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut ri = 0;
    for m in mask.chars() {
        if ri >= raw.len() {
            break;
        }
        let rule: Option<fn(char) -> bool> = match m {
            '#' => Some(|c: char| c.is_ascii_digit()),
            'A' => Some(|c: char| c.is_ascii_alphabetic()),
            '*' => Some(|c: char| c.is_ascii_alphanumeric()),
            _ => None,
        };
        match rule {
            Some(ok) => {
                while ri < raw.len() && !ok(raw[ri]) {
                    ri += 1;
                }
                if ri < raw.len() {
                    out.push(raw[ri]);
                    ri += 1;
                }
            }
            None => {
                out.push(m);
                if raw[ri] == m {
                    ri += 1;
                }
            }
        }
    }
    out
}

/// `maxLength` on an edit from `old` to `new` (characters): typing into a
/// full field is refused, a paste is cut to fit.
pub fn limit_len(old: &str, new: &str, max: usize) -> String {
    let n = new.chars().count();
    if n <= max || n <= old.chars().count() {
        return new.to_string();
    }
    if old.chars().count() >= max {
        return old.to_string();
    }
    new.chars().take(max).collect()
}

/// `String(n)` for the numbers the inputs show (no exponent forms).
pub fn js_number(n: f64) -> String {
    if !n.is_finite() {
        return String::new();
    }
    if n == n.trunc() && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    format!("{n}")
}

/// `parseFloat(s)`: the longest leading decimal number, `None` for NaN.
pub fn parse_float(s: &str) -> Option<f64> {
    let s = s.trim_start();
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        i += 1;
    }
    let digits0 = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut end = i;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i > digits0 + 1 || end > digits0 {
            end = i;
        }
    }
    if end == digits0 || (end == digits0 + 1 && b[digits0] == b'.') {
        return None;
    }
    // Exponent.
    let mut j = end;
    if j < b.len() && (b[j] == b'e' || b[j] == b'E') {
        j += 1;
        if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
            j += 1;
        }
        let e0 = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > e0 {
            end = j;
        }
    }
    s[..end].parse().ok()
}

/// CurrencyInput's `parse`: keep `[0-9.-]`, then `parseFloat`.
pub fn parse_money(s: &str) -> Option<f64> {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    if cleaned.is_empty() || cleaned == "-" {
        return None;
    }
    parse_float(&cleaned).filter(|n| n.is_finite())
}

/// The UI locale Chromium would use: `LC_ALL`, else `LC_MESSAGES`, else
/// `LANG`, as a BCP 47 tag (`en_GB.UTF-8` → `en-GB`); `en-US` without one.
pub fn system_locale() -> String {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .unwrap_or_else(|| "en_US".into());
    let tag = raw.split(['.', '@']).next().unwrap_or("en_US");
    tag.replace('_', "-")
}

/// The currency symbol `Intl.NumberFormat(locale, {style: 'currency'})`
/// prints for `code`.
pub fn currency_symbol(code: &str, locale: &str) -> String {
    let region = locale.split('-').nth(1).unwrap_or("");
    match (code, region) {
        ("USD", "US" | "") => "$".into(),
        ("USD", _) => "US$".into(),
        ("EUR", _) => "€".into(),
        ("GBP", _) => "£".into(),
        ("JPY", "JP") => "￥".into(),
        ("JPY", _) => "JP¥".into(),
        ("CAD", "CA") => "$".into(),
        ("CAD", _) => "CA$".into(),
        ("AUD", "AU") => "$".into(),
        ("AUD", _) => "A$".into(),
        ("INR", _) => "₹".into(),
        ("CNY", _) => "CN¥".into(),
        _ => format!("{code}\u{a0}"),
    }
}

/// Minor digits of `code` (ISO 4217).
fn minor_digits(code: &str) -> usize {
    match code {
        "JPY" | "KRW" | "VND" | "CLP" | "ISK" => 0,
        _ => 2,
    }
}

/// `new Intl.NumberFormat(locale, {style: 'currency', currency}).format(v)`
/// with English grouping: `US$1,999.00`, `-$5.50`.
pub fn format_currency(v: f64, code: &str, locale: &str) -> String {
    let digits = minor_digits(code);
    let scale = 10f64.powi(digits as i32);
    // Half away from zero, like ICU's default for currency.
    let cents = (v.abs() * scale + 1e-9).round();
    let int = (cents / scale).trunc() as u64;
    let frac = (cents - int as f64 * scale).round() as u64;
    let s = int.to_string();
    let mut grouped = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let mut out = String::new();
    if v < 0.0 && cents > 0.0 {
        out.push('-');
    }
    out.push_str(&currency_symbol(code, locale));
    out.push_str(&grouped);
    if digits > 0 {
        out.push('.');
        out.push_str(&format!("{frac:0digits$}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_like_apply_mask() {
        let m = "(###) ###-####";
        assert_eq!(apply_mask("", m), "");
        assert_eq!(apply_mask("5", m), "(5");
        assert_eq!(apply_mask("5551234567", m), "(555) 123-4567");
        assert_eq!(apply_mask("(555) 123-4567", m), "(555) 123-4567");
        assert_eq!(apply_mask("555a1", m), "(555) 1");
        assert_eq!(apply_mask("55512345678999", m), "(555) 123-4567");
        // Backspacing over a literal leaves it (React behaves the same).
        assert_eq!(apply_mask("(555", m), "(555");
        assert_eq!(apply_mask("(555) ", m), "(555) ");
        assert_eq!(apply_mask("12/3", "##/##/####"), "12/3");
        assert_eq!(apply_mask("ab12", "AA-##"), "ab-12");
        assert_eq!(apply_mask("a!b", "**"), "ab");
    }

    #[test]
    fn max_length() {
        assert_eq!(limit_len("abc", "abcd", 4), "abcd");
        assert_eq!(
            limit_len("abcd", "abcde", 4),
            "abcd",
            "typing into a full field"
        );
        assert_eq!(limit_len("ab", "abxyz", 4), "abxy", "a paste is cut");
        assert_eq!(limit_len("abcdef", "abcde", 4), "abcde", "deleting is fine");
    }

    #[test]
    fn numbers() {
        assert_eq!(js_number(3.0), "3");
        assert_eq!(js_number(-2.5), "-2.5");
        assert_eq!(js_number(f64::NAN), "");
        assert_eq!(parse_float("12abc"), Some(12.0));
        assert_eq!(parse_float("-1.5"), Some(-1.5));
        assert_eq!(parse_float(".5"), Some(0.5));
        assert_eq!(parse_float("1."), Some(1.0));
        assert_eq!(parse_float("1e3"), Some(1000.0));
        assert_eq!(parse_float("-"), None);
        assert_eq!(parse_float(""), None);
        assert_eq!(parse_float("."), None);
        assert_eq!(parse_money("US$1,999.00"), Some(1999.0));
        assert_eq!(parse_money("-"), None);
        assert_eq!(parse_money("abc"), None);
    }

    #[test]
    fn currency_like_intl() {
        assert_eq!(format_currency(1999.0, "USD", "en-GB"), "US$1,999.00");
        assert_eq!(format_currency(1999.0, "USD", "en-US"), "$1,999.00");
        assert_eq!(format_currency(-5.5, "USD", "en-US"), "-$5.50");
        assert_eq!(format_currency(0.005, "EUR", "en-GB"), "€0.01");
        assert_eq!(
            format_currency(1234567.891, "GBP", "en-GB"),
            "£1,234,567.89"
        );
        assert_eq!(format_currency(1500.0, "JPY", "en-US"), "JP¥1,500");
    }
}
