use dlang::immix::blocks::{BlockList, BumpBlock};

#[test]
fn test_bumpblock() {
    let block = BumpBlock::new();
    let mut block = block.unwrap();
    block.meta.mark_range(20, 30);

    for _ in 0..=3 {
        let _ = block.inner_alloc(10000);
    }
    block.meta.unmark_range(130, 230);

    for _ in 0..=3 {
        let _ = block.inner_alloc(10000);
    }

    for idx in 0..=250 {
        println!("idx {idx}");
        let _ = block.inner_alloc(100);
    }
}

#[test]
fn test_block_list() {
    let mut bls = BlockList::new();
    dbg!(&bls);
    let big_size: usize = (1 << 18) - 1000;
    let r = bls.alloc(big_size);
    dbg!(&r, &bls);
    let r = bls.alloc(1 << 10);
    dbg!(&r, &bls);
    let r = bls.alloc(1 << 20);
    dbg!(&r, &bls);
    for _ in 0..128 {
        let _ = bls.alloc(1 << 10);
    }
    for _ in 0..128 {
        let _ = bls.alloc(1 << 13);
    }
    dbg!(&bls);
}
