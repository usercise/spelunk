/// Clamps a value between min and max.
pub fn clamp(val: i32, min: i32, max: i32) -> i32 {
    if val < min {
        min
    } else if val > max {
        max
    } else {
        val
    }
}

/// Converts a slice of i32 to their sum.
pub fn sum_slice(values: &[i32]) -> i32 {
    values.iter().sum()
}
