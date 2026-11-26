use core::mem;

const WORD_SIZE: usize = mem::size_of::<usize>();

/// Copies `len` bytes from `src` to `dest`.
///
/// The memory areas may not overlap.
///
/// This implementation is optimized to copy bytes in chunks of `usize`.
///
/// # Arguments
///
/// * `dest` - The destination buffer.
/// * `src` - The source buffer.
/// * `len` - The number of bytes to copy.
///
/// # Returns
///
/// A pointer to the destination buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    unsafe {
        let mut i = 0_usize;

        // First we copy len / WORD_SIZE chunks...

        let chunks = len / WORD_SIZE;

        while i < chunks * WORD_SIZE {
            dest.add(i)
                .cast::<usize>()
                .write_unaligned(src.add(i).cast::<usize>().read_unaligned());
            i += WORD_SIZE;
        }

        // .. then we copy len % WORD_SIZE bytes
        while i < len {
            dest.add(i).write(src.add(i).read());
            i += 1;
        }

        dest
    }
}

/// Copies `len` bytes from `src` to `dest`.
///
/// The memory areas may overlap.
///
/// This implementation is optimized to copy bytes in chunks of `usize`.
///
/// # Arguments
///
/// * `dest` - The destination buffer.
/// * `src` - The source buffer.
/// * `len` - The number of bytes to copy.
///
/// # Returns
///
/// A pointer to the destination buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    unsafe {
        let chunks = len / WORD_SIZE;

        if src < dest as *const u8 {
            // We have to copy backwards if copying upwards.

            let mut i = len;

            while i != chunks * WORD_SIZE {
                i -= 1;
                dest.add(i).write(src.add(i).read());
            }

            while i > 0 {
                i -= WORD_SIZE;

                dest.add(i)
                    .cast::<usize>()
                    .write_unaligned(src.add(i).cast::<usize>().read_unaligned());
            }
        } else {
            // We have to copy forward if copying downwards.

            let mut i = 0_usize;

            while i < chunks * WORD_SIZE {
                dest.add(i)
                    .cast::<usize>()
                    .write_unaligned(src.add(i).cast::<usize>().read_unaligned());

                i += WORD_SIZE;
            }

            while i < len {
                dest.add(i).write(src.add(i).read());
                i += 1;
            }
        }

        dest
    }
}

/// Fills the first `len` bytes of the memory area pointed to by `dest` with the constant byte `byte`.
///
/// This implementation is optimized to set bytes in chunks of `usize`.
///
/// # Arguments
///
/// * `dest` - The buffer to fill.
/// * `byte` - The byte to fill with.
/// * `len` - The number of bytes to fill.
///
/// # Returns
///
/// A pointer to the destination buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dest: *mut u8, byte: i32, len: usize) -> *mut u8 {
    unsafe {
        let byte = byte as u8;

        let mut i = 0;

        let broadcasted = usize::from_ne_bytes([byte; WORD_SIZE]);
        let chunks = len / WORD_SIZE;

        while i < chunks * WORD_SIZE {
            dest.add(i).cast::<usize>().write_unaligned(broadcasted);
            i += WORD_SIZE;
        }

        while i < len {
            dest.add(i).write(byte);
            i += 1;
        }

        dest
    }
}

/// Compares the first `len` bytes of the memory areas `s1` and `s2`.
///
/// This implementation is optimized to compare bytes in chunks of `usize`.
///
/// # Arguments
///
/// * `s1` - The first buffer to compare.
/// * `s2` - The second buffer to compare.
/// * `len` - The number of bytes to compare.
///
/// # Returns
///
/// An integer less than, equal to, or greater than zero if the first `len` bytes of `s1` is
/// found, respectively, to be less than, to match, or be greater than the first `len` bytes of
/// `s2`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(s1: *const u8, s2: *const u8, len: usize) -> i32 {
    unsafe {
        let mut i = 0_usize;

        // First compare WORD_SIZE chunks...
        let chunks = len / WORD_SIZE;

        while i < chunks * WORD_SIZE {
            let a = s1.add(i).cast::<usize>().read_unaligned();
            let b = s2.add(i).cast::<usize>().read_unaligned();

            if a != b {
                // x86 has had bswap since the 80486, and the compiler will likely use the faster
                // movbe. AArch64 has the REV instruction, which I think is universally available.
                let diff = usize::from_be(a).wrapping_sub(usize::from_be(b)) as isize;

                return diff.signum() as i32;
            }
            i += WORD_SIZE;
        }

        // ... and then compare bytes.
        while i < len {
            let a = s1.add(i).read();
            let b = s2.add(i).read();

            if a != b {
                return i32::from(a) - i32::from(b);
            }
            i += 1;
        }

        0
    }
}
