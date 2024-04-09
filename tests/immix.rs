use dlang::immix::blocks::BumpBlock;

#[test]
fn test_bumpblock() {
    let block = BumpBlock::new();
    let mut block = block.unwrap();
    block.meta.mark_range(20, 30);

    for _ in 0..=3 {
        let _ = block.inner_alloc(10000);
    }
    block.meta.print_mark_status();
    println!();
    block.meta.unmark_range(130, 230);
    block.meta.print_mark_status();

    for _ in 0..=3 {
        let _ = block.inner_alloc(10000);
    }

    for idx in 0..=250 {
        println!("idx {idx}");
        let _ = block.inner_alloc(100);
    }
}
