use std::ptr;

pub unsafe fn write<T>(dest: *const u8, object: T) {
    ptr::write(dest as *mut T, object);
}
