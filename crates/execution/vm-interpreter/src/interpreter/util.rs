pub fn decode_single(x: usize) -> Option<usize> {
    if x <= 90 || x >= 128 {
        Some((x + 145) % 256)
    } else {
        None
    }
}

pub fn decode_pair(x: usize) -> Option<(usize, usize)> {
    if x > 81 && x < 128 {
        return None;
    }
    let k = x ^ 143;
    let q = k / 16;
    let r = k % 16;
    if q < r {
        Some((q + 1, r + 1))
    } else {
        Some((r + 1, 29 - q))
    }
}
