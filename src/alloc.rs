//! Large allocations come straight from the operating system and go back to
//! it as soon as they're freed.
//!
//! macOS's allocator keeps freed large blocks for reuse, and they still count
//! as Folio's memory: zooming in and out a few times rebuilds the text atlas
//! (several MB each time) and left 100 MB of freed blocks behind. Blocks of
//! a megabyte or more are rare (the atlas, whole-window buffers), so one
//! mmap each costs nothing noticeable.

use std::alloc::{GlobalAlloc, Layout, System};

pub struct Allocator;

const LARGE: usize = 1 << 20;

fn large(layout: &Layout) -> bool {
    layout.size() >= LARGE && layout.align() <= 4096
}

// SAFETY: small blocks go to the system allocator unchanged; large ones are
// whole anonymous mappings, page aligned (enough for any align up to 4096),
// zero-filled, and unmapped with the size they were mapped with (Rust
// passes back the layout a block was allocated with).
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !large(&layout) {
            return unsafe { System.alloc(layout) };
        }
        let p = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                layout.size(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if p == libc::MAP_FAILED { std::ptr::null_mut() } else { p.cast() }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if large(&layout) {
            // Fresh mappings are already zero.
            unsafe { self.alloc(layout) }
        } else {
            unsafe { System.alloc_zeroed(layout) }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if large(&layout) {
            unsafe { libc::munmap(ptr.cast(), layout.size()) };
        } else {
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: GlobalAlloc::realloc's contract: the new layout is valid.
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        if !large(&layout) && !large(&new_layout) {
            return unsafe { System.realloc(ptr, layout, new_size) };
        }
        let new = unsafe { self.alloc(new_layout) };
        if !new.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
        }
        new
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn large_blocks_work_and_grow() {
        let mut v: Vec<u32> = vec![7; 300_000];
        assert!(v.iter().all(|&x| x == 7));
        v.resize(600_000, 9);
        assert_eq!((v[0], v[599_999]), (7, 9));
        v.truncate(10);
        v.shrink_to_fit();
        assert_eq!(v, vec![7; 10]);
        let z = vec![0u8; 3 << 20];
        assert!(z.iter().all(|&b| b == 0));
    }
}
