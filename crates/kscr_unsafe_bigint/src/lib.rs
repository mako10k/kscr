pub type Integer = num_bigint::BigInt;

pub fn int_from_i64(n: i64) -> Integer {
    Integer::from(n)
}

pub fn parse_integer(s: &str) -> Result<Integer, &'static str> {
    Integer::parse_bytes(s.as_bytes(), 10).ok_or("invalid integer")
}

pub fn is_zero(n: &Integer) -> bool {
    n == &Integer::from(0)
}

pub fn in_i32_range(n: &Integer) -> bool {
    let min = Integer::from(i32::MIN);
    let max = Integer::from(i32::MAX);
    n >= &min && n <= &max
}

pub fn in_i64_range(n: &Integer) -> bool {
    let min = Integer::from(i64::MIN);
    let max = Integer::from(i64::MAX);
    n >= &min && n <= &max
}

pub fn to_i32_range_checked(n: Integer) -> i32 {
    n.to_string()
        .parse::<i32>()
        .expect("range-checked BigInt should parse to i32")
}

pub fn to_i64_range_checked(n: Integer) -> i64 {
    n.to_string()
        .parse::<i64>()
        .expect("range-checked BigInt should parse to i64")
}
