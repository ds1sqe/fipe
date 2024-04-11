#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Unmarked,
    Allocated,
    Marked,
}
