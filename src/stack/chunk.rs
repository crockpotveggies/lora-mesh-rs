/// Split without repeatedly copying the unprocessed tail.
pub fn chunk_data(data: Vec<u8>, maxlength: &usize) -> Vec<Vec<u8>> {
    if *maxlength == 0 {
        return Vec::new();
    }
    if data.is_empty() {
        return vec![Vec::new()];
    }
    data.chunks(*maxlength)
        .map(|chunk| chunk.to_vec())
        .collect()
}
