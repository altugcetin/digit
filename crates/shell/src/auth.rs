use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::Read;

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const MAX: usize = 4096;

#[repr(C)]
struct Message {
    style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct Response {
    resp: *mut c_char,
    code: c_int,
}

type ConvFn = unsafe extern "C" fn(c_int, *mut *const Message, *mut *mut Response, *mut c_void) -> c_int;

#[repr(C)]
struct Conv {
    conv: ConvFn,
    data: *mut c_void,
}

type Start = unsafe extern "C" fn(*const c_char, *const c_char, *const Conv, *mut *mut c_void) -> c_int;
type Authenticate = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type End = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;

struct Secret {
    text: *const c_char,
    sent: bool,
}

unsafe fn free_all(replies: *mut Response, n: usize) {
    unsafe {
        for j in 0..n {
            let r = replies.add(j);
            if !(*r).resp.is_null() {
                libc::free((*r).resp as *mut c_void);
            }
        }
        libc::free(replies as *mut c_void);
    }
}

unsafe extern "C" fn converse(n: c_int, msgs: *mut *const Message, out: *mut *mut Response, data: *mut c_void) -> c_int {
    unsafe {
        if n <= 0 || msgs.is_null() || out.is_null() || data.is_null() {
            return PAM_CONV_ERR;
        }
        let secret = &mut *(data as *mut Secret);
        let count = n as usize;
        let replies = libc::calloc(count, std::mem::size_of::<Response>()) as *mut Response;
        if replies.is_null() {
            return PAM_BUF_ERR;
        }
        for i in 0..count {
            let m = *msgs.add(i);
            if m.is_null() {
                free_all(replies, i);
                return PAM_CONV_ERR;
            }
            let r = replies.add(i);
            match (*m).style {
                PAM_PROMPT_ECHO_OFF => {
                    if secret.sent {
                        free_all(replies, i);
                        return PAM_CONV_ERR;
                    }
                    (*r).resp = libc::strdup(secret.text);
                    secret.sent = true;
                }
                PAM_PROMPT_ECHO_ON => (*r).resp = libc::strdup(c"".as_ptr()),
                PAM_ERROR_MSG | PAM_TEXT_INFO => {}
                _ => {
                    free_all(replies, i);
                    return PAM_CONV_ERR;
                }
            }
            if matches!((*m).style, PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON) && (*r).resp.is_null() {
                free_all(replies, i + 1);
                return PAM_BUF_ERR;
            }
        }
        *out = replies;
        PAM_SUCCESS
    }
}

fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

fn user() -> Option<CString> {
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_name.is_null() {
            return None;
        }
        Some(CStr::from_ptr((*pw).pw_name).to_owned())
    }
}

pub fn service() -> &'static CStr {
    let has = |n: &str| ["/etc/pam.d/", "/usr/lib/pam.d/", "/usr/etc/pam.d/"].iter().any(|d| std::path::Path::new(&format!("{d}{n}")).exists());
    if has("digit") {
        c"digit"
    } else if has("swaylock") {
        c"swaylock"
    } else {
        c"login"
    }
}

fn check(password: &CStr) -> c_int {
    let Some(user) = user() else { return 2 };
    unsafe {
        let lib = libc::dlopen(c"libpam.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if lib.is_null() {
            return 2;
        }
        let start = libc::dlsym(lib, c"pam_start".as_ptr());
        let auth = libc::dlsym(lib, c"pam_authenticate".as_ptr());
        let end = libc::dlsym(lib, c"pam_end".as_ptr());
        if start.is_null() || auth.is_null() || end.is_null() {
            return 2;
        }
        let start: Start = std::mem::transmute(start);
        let auth: Authenticate = std::mem::transmute(auth);
        let end: End = std::mem::transmute(end);
        let mut secret = Secret { text: password.as_ptr(), sent: false };
        let conv = Conv { conv: converse, data: &mut secret as *mut Secret as *mut c_void };
        let mut handle: *mut c_void = std::ptr::null_mut();
        if start(service().as_ptr(), user.as_ptr(), &conv, &mut handle) != PAM_SUCCESS || handle.is_null() {
            return 2;
        }
        let rc = auth(handle, 0);
        end(handle, rc);
        if rc == PAM_SUCCESS { 0 } else { 1 }
    }
}

pub fn helper() -> ! {
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0);
    }
    let mut buf = [0u8; MAX + 1];
    let mut n = 0;
    let mut input = std::io::stdin().lock();
    let ok = loop {
        if n > MAX - 1 {
            break false;
        }
        match input.read(&mut buf[n..MAX]) {
            Ok(0) => break true,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break false,
        }
    };
    let code = if !ok || buf[..n].contains(&0) {
        2
    } else {
        buf[n] = 0;
        match CStr::from_bytes_with_nul(&buf[..=n]) {
            Ok(p) => check(p),
            Err(_) => 2,
        }
    };
    wipe(&mut buf);
    std::process::exit(code);
}
