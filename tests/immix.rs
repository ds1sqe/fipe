use dlang::immix::blocks::{self, Block, BumpBlock};

#[test]
fn test_bumpblock() {
    let block = BumpBlock::new();
    let mut block = block.unwrap();
    let foo = block.inner_alloc(100);
    let bar = block.inner_alloc(100);
    let baz = block.inner_alloc(100);
}
