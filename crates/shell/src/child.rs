use gtk::gio;
use std::ffi::OsStr;

pub fn spawn(argv: &[&OsStr], flags: gio::SubprocessFlags) -> Result<gio::Subprocess, gtk::glib::Error> {
    let launcher = gio::SubprocessLauncher::new(flags);
    // SAFETY: runs between fork and exec and only calls prctl, which is async-signal-safe.
    launcher.set_child_setup(|| unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    });
    launcher.spawn(argv)
}
