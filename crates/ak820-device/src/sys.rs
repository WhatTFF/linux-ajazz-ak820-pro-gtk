//! The only `unsafe` in this crate: `poll(2)` and `flock(2)`.

use std::io;
use std::os::fd::RawFd;
use std::time::Duration;

/// Wait until `fd` is readable, or the timeout expires.
pub(crate) fn poll_readable(fd: RawFd, timeout: Duration) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: a single valid pollfd is passed with a matching count.
    let rc = unsafe { libc::poll(&mut pfd, 1, millis) };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(err);
    }
    Ok(rc > 0 && pfd.revents & libc::POLLIN != 0)
}

/// Try to take an exclusive, non-blocking lock on `fd`.
pub(crate) fn try_lock_exclusive(fd: RawFd) -> io::Result<bool> {
    // SAFETY: `fd` is owned by the caller and stays open for the call.
    let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EWOULDBLOCK) => Ok(false),
        _ => Err(err),
    }
}
