use fipe::immix::{
    blocks::{BlockList, BLOCK_CAPACITY, BLOCK_SIZE},
    mark::Mark,
};

#[test]
fn large_allocation_metadata_tracks_mark_state() {
    let mut blocks = BlockList::new();
    let mut object = blocks.alloc(BLOCK_SIZE).unwrap();
    blocks.unmark_all();
    assert!(object.is_unmarked());
    object.set_mark(&Mark::Marked);
    assert!(!object.is_unmarked());
}

#[test]
fn small_allocation_metadata_survives_block_rollover() {
    let mut blocks = BlockList::new();
    let mut first = blocks.alloc(1).unwrap();
    blocks.alloc(BLOCK_CAPACITY).unwrap();
    blocks.unmark_all();
    first.set_mark(&Mark::Marked);
    assert!(blocks.rest.values().any(|block| block.meta.is_marked(0)));
}

#[test]
fn zero_sized_values_keep_their_required_alignment() {
    #[repr(align(512))]
    struct Aligned;
    let mut heap = fipe::immix::Immix::<usize, Aligned>::new();
    for key in 0..3 {
        let ptr = heap.alloc(key, Aligned).unwrap();
        assert_eq!(ptr.data.addr() % std::mem::align_of::<Aligned>(), 0);
    }
}
