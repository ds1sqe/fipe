use super::blocks::{BLOCK_CAPACITY, LINE_SIZE};

// Object size range
const SMALL_MIN: usize = 1;
const SMALL_MAX: usize = LINE_SIZE;

const MEDIUM_MIN: usize = SMALL_MAX + 1;
const MEDIUM_MAX: usize = BLOCK_CAPACITY;

const LARGE_MIN: usize = MEDIUM_MAX + 1;
const LARGE_MAX: usize = std::usize::MAX;
/// Represent Object's size
/// - Small fit inside a line
/// - Medium more than one line but small than block
/// - Large span multiple blocks
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SizeClass {
    Small,
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SizeClassError {
    TooBig,
}

impl SizeClass {
    pub fn from(size: usize) -> Result<SizeClass, SizeClassError> {
        match size {
            SMALL_MIN..=SMALL_MAX => Ok(SizeClass::Small),
            MEDIUM_MIN..=MEDIUM_MAX => Ok(SizeClass::Small),
            LARGE_MIN..=LARGE_MAX => Ok(SizeClass::Small),
            __ => Err(SizeClassError::TooBig),
        }
    }
}
