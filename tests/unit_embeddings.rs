//! Unit tests for embedding helpers (vec_to_blob / blob_to_vec roundtrip).

use spelunk::embeddings::{blob_to_vec, vec_to_blob};

#[test]
fn roundtrip_empty_vec() {
    let v: Vec<f32> = vec![];
    assert_eq!(blob_to_vec(&vec_to_blob(&v)), v);
}

#[test]
fn roundtrip_single_value() {
    let v = vec![1.0_f32];
    assert_eq!(blob_to_vec(&vec_to_blob(&v)), v);
}

#[test]
fn roundtrip_multi_value() {
    let v: Vec<f32> = vec![0.0, 1.0, -1.0, f32::MAX, f32::MIN_POSITIVE];
    let result = blob_to_vec(&vec_to_blob(&v));
    for (a, b) in v.iter().zip(result.iter()) {
        assert_eq!(a.to_bits(), b.to_bits(), "bit-exact roundtrip failed");
    }
}

#[test]
fn blob_length_is_four_bytes_per_float() {
    let v: Vec<f32> = vec![1.0, 2.0, 3.0];
    assert_eq!(vec_to_blob(&v).len(), 12);
}

#[test]
fn blob_to_vec_ignores_trailing_incomplete_chunk() {
    // 13 bytes → 3 complete f32s (12 bytes) + 1 leftover byte (ignored)
    let mut blob = vec_to_blob(&[1.0_f32, 2.0, 3.0]);
    blob.push(0xFF);
    let result = blob_to_vec(&blob);
    assert_eq!(result.len(), 3);
}
