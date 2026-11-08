//! Name hashing shared by content loading and mod scripts.

/// Helper.GetHashCodeCustom, UTF-16 packed little-endian and signed shifts.
pub fn custom_hash(name: &str) -> i32 {
    let mut units: Vec<u16> = name.encode_utf16().collect();
    let len = units.len();
    units.extend([0, 0, 0]);
    let mut a = 352654597i32;
    let mut b = a;
    let mut i = 0;
    let mix = |value: i32, word: i32| {
        value
            .wrapping_shl(5)
            .wrapping_add(value)
            .wrapping_add(value >> 27)
            ^ word
    };
    let word = |j: usize| i32::from(units[j]) | (i32::from(units[j + 1]) << 16);
    while i < len {
        a = mix(a, word(i));
        if len - i <= 2 {
            break;
        }
        b = mix(b, word(i + 2));
        i += 4;
    }
    a.wrapping_add(b.wrapping_mul(1566083941))
}
