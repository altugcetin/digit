//! Secrets in memory: a password is wiped as soon as it has been used.

/// Overwrites `buf` with zeros in a way the compiler can't drop as a dead store, and keeps later
/// code from moving above it, so the secret is gone before the buffer is freed or reused.
pub fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: `b` is a valid, aligned, exclusive reference into `buf`.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_wiped_buffer_is_all_zeros() {
        let mut secret = *b"hunter2";
        super::wipe(&mut secret);
        assert_eq!(secret, [0; 7]);
    }
}
