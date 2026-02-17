/// Parses a price string with 2 decimal places into a fixed-point integer.
/// Example: "123.45" -> 12345 (representing 123.45 as integer ticks)
#[inline(always)]
pub fn parse_px_2dp(s: &str) -> i64 {
    parse_fixed_dp::<2>(s)
}

/// Parses a quantity string with 3 decimal places into a fixed-point integer.
/// Example: "12.345" -> 12345 (representing 12.345 as integer lots)
#[inline(always)]
pub fn parse_qty_3dp(s: &str) -> i64 {
    parse_fixed_dp::<3>(s)
}

/// Computes 10^DP at compile time for efficient fixed-point arithmetic.
/// This avoids runtime exponentiation and allows the compiler to optimize.
/// Examples: pow10::<2>() = 100, pow10::<3>() = 1000
#[inline(always)]
fn pow10<const DP: u32>() -> i64 {
    // compile-time constant evaluation - compiler optimizes this match away
    match DP {
        0 => 1,              // 10^0 = 1
        1 => 10,             // 10^1 = 10
        2 => 100,            // 10^2 = 100
        3 => 1000,           // 10^3 = 1000
        4 => 10_000,         // 10^4 = 10,000
        5 => 100_000,        // 10^5 = 100,000
        6 => 1_000_000,      // 10^6 = 1,000,000
        _ => 10_i64.pow(DP), // fallback (won’t be hit for DP=2/3)
    }
}

/// Parses a decimal string into a fixed-point integer representation.
///
/// Algorithm overview:
/// 1. Handle optional negative sign
/// 2. Parse integer part (digits before decimal point)
/// 3. Parse fractional part (digits after decimal point, up to DP digits)
/// 4. Pad fractional part with zeros if fewer than DP digits were provided
/// 5. Combine: sign * (int_part * 10^DP + frac_part)
///
/// Example: parse_fixed_dp::<2>("-123.45")
///   - sign = -1
///   - int_part = 123
///   - frac = 45
///   - result = -1 * (123 * 100 + 45) = -12345
///
/// Example: parse_fixed_dp::<3>("12.3")
///   - sign = 1
///   - int_part = 12
///   - frac = 3, then padded to 300
///   - result = 1 * (12 * 1000 + 300) = 12300
#[inline(always)]
pub fn parse_fixed_dp<const DP: u32>(s: &str) -> i64 {
    let b = s.as_bytes();
    let len = b.len();
    let mut start = 0usize;

    let mut sign = 1i64;
    if start < len && b[start] == b'-' {
        sign = -1;
        start += 1;
    }

    let dot_idx = unsafe {
        let found = libc::memchr(
            b.as_ptr().add(start) as *const libc::c_void,
            b'.' as i32,
            len - start,
        );
        if found.is_null() {
            len
        } else {
            (found as *const u8).offset_from(b.as_ptr()) as usize
        }
    };

    let mut int_part = 0i64;
    let mut i = start;
    while i < dot_idx {
        let c = unsafe { *b.get_unchecked(i) };
        int_part = int_part * 10 + (c - b'0') as i64;
        i += 1;
    }

    let frac_start = if dot_idx < len { dot_idx + 1 } else { len };
    let frac_end = (frac_start + DP as usize).min(len);

    let mut frac = 0i64;
    let mut got = 0u32;
    i = frac_start;
    while i < frac_end {
        let c = unsafe { *b.get_unchecked(i) };
        if c < b'0' || c > b'9' {
            break;
        }
        frac = frac * 10 + (c - b'0') as i64;
        got += 1;
        i += 1;
    }

    while got < DP {
        frac *= 10;
        got += 1;
    }

    sign * (int_part * pow10::<DP>() + frac)
}
