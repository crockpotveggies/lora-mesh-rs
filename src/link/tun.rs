//! Small Linux-only TUN descriptor adapter. Interface/route configuration stays in setup code.
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::{
        fs::OpenOptionsExt,
        io::{AsRawFd, RawFd},
    },
};
pub(crate) struct Tun {
    file: File,
    name: String,
}
impl Tun {
    pub(crate) fn open(name: &str) -> io::Result<Self> {
        if name.is_empty()
            || name.len() >= libc::IFNAMSIZ
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(io::Error::other("invalid TUN name"));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open("/dev/net/tun")?;
        // SAFETY: Linux ifreq is an integer/byte POD. The name fits IFNAMSIZ and is NUL terminated.
        let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
        for (out, b) in request.ifr_name.iter_mut().zip(name.bytes()) {
            *out = b as libc::c_char;
        }
        request.ifr_ifru.ifru_flags = (libc::IFF_TUN | libc::IFF_NO_PI | 0x8000) as libc::c_short; // IFF_TUN_EXCL
        // SAFETY: TUNSETIFF copies a valid writable ifreq; the descriptor is exclusively owned.
        if unsafe {
            libc::ioctl(
                file.as_raw_fd(),
                0x400454cau64 as libc::c_ulong,
                &mut request,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            file,
            name: name.to_owned(),
        })
    }
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    pub(crate) fn recv(&self, bytes: &mut [u8]) -> io::Result<usize> {
        (&self.file).read(bytes)
    }
    pub(crate) fn send(&self, bytes: &[u8]) -> io::Result<usize> {
        (&self.file).write(bytes)
    }
}
impl AsRawFd for Tun {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}
