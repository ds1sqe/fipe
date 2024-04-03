use dlang::memory::block::Block;

#[test]
fn test_block_alignment() {
    let size = 64;
    let block = Block::new(size).unwrap();
    // the block address bitwise AND the alignment bits (size - 1) should
    // be a mutually exclusive set of bits
    let mask = size - 1;
    assert!((block.ptr.as_ptr() as usize & mask) ^ mask == mask);
}
