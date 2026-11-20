use crate::app::App;
use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::time::Duration;
use std::sync::{Arc, Mutex};
use futures_util::StreamExt;
use zbus::zvariant::{OwnedValue, Value};

const PATH: &str = "/dev/digit/PolkitAgent";
const POLKIT: &str = "org.freedesktop.PolicyKit1";
const SOCKET: &str = "/run/polkit/agent-helper.socket";
const HELPERS: [&str; 2] = ["/usr/libexec/polkit-1/polkit-agent-helper-1", "/usr/lib/polkit-1/polkit-agent-helper-1"];
const TRIES: u32 = 3;

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Cancelled(String),
    Failed(String),
}

enum Outcome {
    Done,
    Cancelled,
    Failed(String),
}

struct Request {
    cookie: String,
    message: String,
    action: String,
    details: HashMap<String, String>,
    users: Vec<(u32, String)>,
    reply: async_channel::Sender<Outcome>,
}

enum Event {
    Begin(Request),
    Cancel(String),
}

struct Agent {
    tx: async_channel::Sender<Event>,
}

async fn from_polkit(conn: &zbus::Connection, header: &zbus::message::Header<'_>) -> bool {
    let Some(sender) = header.sender() else { return false };
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else { return false };
    let Ok(name) = zbus::names::BusName::try_from(POLKIT) else { return false };
    dbus.get_name_owner(name).await.is_ok_and(|owner| owner.as_str() == sender.as_str())
}

fn name_of(uid: u32) -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) };
    if rc != 0 || out.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }.to_string_lossy().to_string();
    (!name.is_empty() && !name.contains(['\n', '\0'])).then_some(name)
}

fn members(gid: u32) -> Vec<u32> {
    let mut buf = vec![0 as libc::c_char; 16384];
    let mut gr: libc::group = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::group = std::ptr::null_mut();
    let rc = unsafe { libc::getgrgid_r(gid, &mut gr, buf.as_mut_ptr(), buf.len(), &mut out) };
    if rc != 0 || out.is_null() || gr.gr_mem.is_null() {
        return Vec::new();
    }
    let mut uids = Vec::new();
    let mut p = gr.gr_mem;
    // This runs on zbus's thread, so only the reentrant lookups: getpwnam's
    // static buffer is shared with getpwuid calls on GTK's thread.
    let mut pwbuf = vec![0 as libc::c_char; 4096];
    unsafe {
        while !(*p).is_null() {
            let mut pw: libc::passwd = std::mem::zeroed();
            let mut found: *mut libc::passwd = std::ptr::null_mut();
            if libc::getpwnam_r(*p, &mut pw, pwbuf.as_mut_ptr(), pwbuf.len(), &mut found) == 0 && !found.is_null() {
                uids.push(pw.pw_uid);
            }
            p = p.add(1);
        }
    }
    uids
}

fn users(identities: &[(String, HashMap<String, OwnedValue>)]) -> Vec<(u32, String)> {
    let me = std::fs::metadata("/proc/self").map_or(u32::MAX, |m| std::os::unix::fs::MetadataExt::uid(&m));
    let mut uids: Vec<u32> = Vec::new();
    for (kind, map) in identities {
        let num = |k: &str| map.get(k).and_then(|v| u32::try_from(v.try_clone().ok()?).ok());
        match kind.as_str() {
            "unix-user" => uids.extend(num("uid")),
            "unix-group" => uids.extend(num("gid").map(members).unwrap_or_default()),
            _ => {}
        }
    }
    uids.sort_by_key(|u| (*u != me, *u != 0, *u));
    uids.dedup();
    uids.into_iter().filter_map(|u| name_of(u).map(|n| (u, n))).collect()
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    #[allow(clippy::too_many_arguments)]
    async fn begin_authentication(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        action_id: String,
        message: String,
        _icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> Result<(), AgentError> {
        if !from_polkit(conn, &header).await {
            return Err(AgentError::Failed("Only polkit may start authentication".into()));
        }
        if cookie.is_empty() || cookie.contains(['\n', '\0']) {
            return Err(AgentError::Failed("Invalid cookie".into()));
        }
        let users = users(&identities);
        if users.is_empty() {
            return Err(AgentError::Failed("No identity can authenticate this action".into()));
        }
        let (tx, rx) = async_channel::bounded(1);
        let request = Request { cookie, message, action: action_id, details, users, reply: tx };
        self.tx.send(Event::Begin(request)).await.map_err(|_| AgentError::Failed("digit is shutting down".into()))?;
        match rx.recv().await {
            Ok(Outcome::Done) => Ok(()),
            Ok(Outcome::Cancelled) | Err(_) => Err(AgentError::Cancelled("Authentication was cancelled".into())),
            Ok(Outcome::Failed(e)) => Err(AgentError::Failed(e)),
        }
    }

    async fn cancel_authentication(&self, #[zbus(header)] header: zbus::message::Header<'_>, #[zbus(connection)] conn: &zbus::Connection, cookie: String) -> Result<(), AgentError> {
        if !from_polkit(conn, &header).await {
            return Err(AgentError::Failed("Only polkit may cancel authentication".into()));
        }
        let _ = self.tx.send(Event::Cancel(cookie)).await;
        Ok(())
    }
}

fn unescape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(d @ '0'..='7') => {
                let mut v = d.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(x) => {
                            v = v * 8 + x;
                            chars.next();
                        }
                        None => break,
                    }
                }
                out.push(char::from_u32(v).unwrap_or('?'));
            }
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

fn wipe(secret: &mut [u8]) {
    for b in secret.iter_mut() {
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

fn converse(reader: &mut dyn BufRead, writer: &mut dyn Write, secret: &mut Vec<u8>) -> (bool, Vec<String>) {
    let mut notes = Vec::new();
    let mut answered = false;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = unescape(line.trim_end_matches('\n'));
        if text.starts_with("PAM_PROMPT_ECHO_OFF ") || text.starts_with("PAM_PROMPT_ECHO_ON ") {
            let ok = if answered {
                writer.write_all(b"\n").is_ok()
            } else {
                answered = true;
                let ok = writer.write_all(secret).and_then(|_| writer.write_all(b"\n")).is_ok();
                wipe(secret);
                secret.clear();
                ok
            };
            if !ok || writer.flush().is_err() {
                break;
            }
        } else if let Some(m) = text.strip_prefix("PAM_ERROR_MSG ").or_else(|| text.strip_prefix("PAM_TEXT_INFO ")) {
            notes.push(m.to_string());
        } else if text.starts_with("SUCCESS") {
            wipe(secret);
            return (true, notes);
        } else if text.starts_with("FAILURE") {
            break;
        } else {
            break;
        }
    }
    wipe(secret);
    (false, notes)
}

struct Secret(Vec<u8>);

impl Drop for Secret {
    fn drop(&mut self) { wipe(&mut self.0); }
}

#[derive(Default)]
struct Cancellation {
    cancelled: bool,
    socket: Option<UnixStream>,
    child: Option<std::process::Child>,
}

fn cancel_worker(cancel: &Mutex<Cancellation>) {
    let mut job = cancel.lock().unwrap_or_else(|e| e.into_inner());
    job.cancelled = true;
    if let Some(socket) = job.socket.take() { let _ = socket.shutdown(std::net::Shutdown::Both); }
    if let Some(child) = job.child.as_mut() { let _ = child.kill(); }
}

fn authenticate(user: &str, cookie: &str, mut secret: Secret, cancel: &Mutex<Cancellation>) -> (bool, Vec<String>) {
    if std::path::Path::new(SOCKET).exists() {
        if let Ok(stream) = UnixStream::connect(SOCKET) {
            {
                let mut job = cancel.lock().unwrap_or_else(|e| e.into_inner());
                if job.cancelled { return (false, Vec::new()); }
                let Ok(socket) = stream.try_clone() else { return (false, Vec::new()); };
                job.socket = Some(socket);
            }
            let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
            let Ok(mut writer) = stream.try_clone() else {
                wipe(&mut secret.0);
                return (false, Vec::new());
            };
            if writer.write_all(format!("{user}\n{cookie}\n").as_bytes()).is_err() {
                wipe(&mut secret.0);
                return (false, Vec::new());
            }
            let mut reader = BufReader::new(stream);
            return converse(&mut reader, &mut writer, &mut secret.0);
        }
    }
    let Some(helper) = HELPERS.iter().find(|h| std::path::Path::new(h).exists()) else {
        wipe(&mut secret.0);
        return (false, vec!["polkit's authentication helper is missing".into()]);
    };
    let child = std::process::Command::new(helper)
        .arg(user)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        wipe(&mut secret.0);
        return (false, Vec::new());
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        wipe(&mut secret.0);
        let _ = child.kill();
        let _ = child.wait();
        return (false, Vec::new());
    };
    {
        let mut job = cancel.lock().unwrap_or_else(|e| e.into_inner());
        if job.cancelled { let _ = child.kill(); }
        job.child = Some(child);
    }
    let result = if stdin.write_all(format!("{cookie}\n").as_bytes()).is_ok() {
        let mut reader = BufReader::new(stdout);
        converse(&mut reader, &mut stdin, &mut secret.0)
    } else {
        wipe(&mut secret.0);
        (false, Vec::new())
    };
    drop(stdin);
    let child = cancel.lock().unwrap_or_else(|e| e.into_inner()).child.take();
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

struct Dialog {
    window: gtk::Window,
    entry: gtk::PasswordEntry,
    error: gtk::Label,
    go: gtk::Button,
    pick: Option<gtk::DropDown>,
    tries: u32,
    busy: bool,
}

struct Live {
    id: u64,
    request: Request,
    dialog: Dialog,
    cancel: Arc<Mutex<Cancellation>>,
}

thread_local! {
    static QUEUE: RefCell<VecDeque<Request>> = const { RefCell::new(VecDeque::new()) };
    static NEXT: Cell<u64> = const { Cell::new(0) };
    static LIVE: RefCell<Option<Live>> = const { RefCell::new(None) };
    static APP: RefCell<std::rc::Weak<App>> = const { RefCell::new(std::rc::Weak::new()) };
    static REGISTERED: RefCell<Option<(String, zbus::Connection)>> = const { RefCell::new(None) };
}

fn process_name(pid: &str) -> Option<String> {
    let pid: u32 = pid.parse().ok()?;
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim().to_string())
}

fn css(app: &App) -> String {
    let pal = app.palette.get();
    let c = crate::draw::css;
    format!(
        r#"
window.digit-auth {{ background: rgba(0,0,0,0.55); }}
.auth-card {{ background: #0b0b0b; border-radius: 28px; padding: 26px 26px 22px 26px; color: {main}; font-family: "{font}", sans-serif; }}
.auth-mark {{ background: {accent}; color: {on}; border-radius: 18px; min-width: 52px; min-height: 52px; font-family: "{icons}"; font-size: 28px; }}
.auth-title {{ font-size: 20px; font-weight: 650; }}
.auth-text {{ color: {meta}; font-size: 14px; }}
.auth-small {{ color: {faint}; font-size: 12px; }}
.auth-mono {{ color: {meta}; font-size: 12px; font-family: "Rubik", monospace; background: alpha({main}, 0.06); border-radius: 10px; padding: 8px 10px; }}
.auth-error {{ color: {risk}; font-size: 13px; }}
.auth-card passwordentry, .auth-card dropdown > button {{ background: #161616; border-radius: 14px; border: none; box-shadow: none; outline: none; padding: 8px 12px; min-height: 26px; color: {main}; }}
.auth-card passwordentry:focus-within {{ background: #1c1c1c; outline: none; box-shadow: inset 0 0 0 1px alpha({accent}, 0.55); }}
.auth-card passwordentry text {{ outline: none; box-shadow: none; }}
.auth-card button.pill {{ background: alpha({main}, 0.10); color: {main}; border-radius: 999px; padding: 7px 18px; border: none; box-shadow: none; font-weight: 520; }}
.auth-card button.pill:hover {{ background: alpha({main}, 0.16); }}
.auth-card button.pill.accent {{ background: {accent}; color: {on}; }}
.auth-card button.pill:disabled {{ opacity: 0.5; }}
"#,
        main = c(pal.ink_main),
        meta = c(pal.ink_meta),
        faint = c(pal.ink_faint),
        accent = c(pal.accent),
        on = c(pal.on_accent),
        risk = c(pal.risk),
        font = app.cfg.font,
        icons = app.cfg.icon_font,
    )
}

fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_max_width_chars(46);
    l.add_css_class(class);
    l
}

fn show(request: &Request, id: u64) -> Option<Dialog> {
    let app = APP.with(|a| a.borrow().upgrade())?;
    let display = gdk::Display::default()?;
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css(&app));
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-auth"));
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.add_css_class("digit-auth");
    let output = crate::compositor::focused_output();
    window.set_monitor(crate::screens::for_output(output.as_deref()).as_ref());
    let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
    card.add_css_class("auth-card");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_size_request(440, -1);
    let mark = gtk::Label::new(Some("lock"));
    mark.add_css_class("auth-mark");
    mark.set_halign(gtk::Align::Start);
    card.append(&mark);
    card.append(&label("Authentication required", "auth-title"));
    card.append(&label(&request.message, "auth-text"));
    let who = request.details.get("polkit.subject-pid").or(request.details.get("polkit.caller-pid")).and_then(|p| process_name(p));
    if let Some(cmd) = request.details.get("command_line").filter(|c| !c.is_empty()) {
        let l = label(cmd, "auth-mono");
        l.set_selectable(false);
        card.append(&l);
    }
    let origin = match &who {
        Some(w) => format!("Requested by {w} · {}", request.action),
        None => request.action.clone(),
    };
    card.append(&label(&origin, "auth-small"));
    let pick = if request.users.len() > 1 {
        let names: Vec<&str> = request.users.iter().map(|u| u.1.as_str()).collect();
        let dd = gtk::DropDown::from_strings(&names);
        dd.set_margin_top(6);
        card.append(&dd);
        Some(dd)
    } else {
        None
    };
    let first = &request.users[0].1;
    let hint = if request.users[0].0 == 0 { "Password for root (administrator)".to_string() } else { format!("Password for {first}") };
    let hint_label = label(&hint, "auth-text");
    hint_label.set_margin_top(8);
    card.append(&hint_label);
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_hexpand(true);
    card.append(&entry);
    let error = label("", "auth-error");
    error.set_visible(false);
    card.append(&error);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(8);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("pill");
    let go = gtk::Button::with_label("Authenticate");
    go.add_css_class("pill");
    go.add_css_class("accent");
    buttons.append(&cancel);
    buttons.append(&go);
    card.append(&buttons);
    if let Some(dd) = &pick {
        let users = request.users.clone();
        let hint_label = hint_label.clone();
        dd.connect_selected_notify(move |d| {
            if let Some((uid, name)) = users.get(d.selected() as usize) {
                hint_label.set_text(&if *uid == 0 { "Password for root (administrator)".to_string() } else { format!("Password for {name}") });
            }
        });
    }
    cancel.connect_clicked(move |_| finish_if(id, Outcome::Cancelled));
    go.connect_clicked(move |_| submit(id));
    entry.connect_activate(move |_| submit(id));
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            finish_if(id, Outcome::Cancelled);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window.set_child(Some(&card));
    window.present();
    entry.grab_focus();
    window.connect_destroy(move |_| {
        if let Some(d) = gdk::Display::default() {
            gtk::style_context_remove_provider_for_display(&d, &provider);
        }
    });
    Some(Dialog { window, entry, error, go, pick, tries: 0, busy: false })
}

fn next() {
    if LIVE.with(|l| l.borrow().is_some()) {
        return;
    }
    let Some(request) = QUEUE.with(|q| q.borrow_mut().pop_front()) else { return };
    let id = NEXT.with(|n| { n.set(n.get() + 1); n.get() });
    match show(&request, id) {
        Some(dialog) => LIVE.with(|l| *l.borrow_mut() = Some(Live { id, request, dialog, cancel: Arc::new(Mutex::new(Cancellation::default())) })),
        None => {
            let _ = request.reply.try_send(Outcome::Failed("The authentication dialog could not be shown".into()));
            next();
        }
    }
}

fn finish(outcome: Outcome) {
    let Some(live) = LIVE.with(|l| l.borrow_mut().take()) else { return };
    cancel_worker(&live.cancel);
    live.dialog.entry.set_text("");
    live.dialog.window.destroy();
    let _ = live.request.reply.try_send(outcome);
    App::trim();
    glib::idle_add_local_once(next);
}

fn current(id: u64, live: Option<u64>) -> bool { live == Some(id) }

#[derive(Debug, PartialEq)]
enum AuthReply { Stale, Done, Retry(u32), Failed }

fn auth_reply(id: u64, live: Option<(u64, u32)>, ok: bool) -> AuthReply {
    let Some((_, tries)) = live.filter(|(live, _)| current(id, Some(*live))) else { return AuthReply::Stale };
    if ok { AuthReply::Done } else if tries + 1 >= TRIES { AuthReply::Failed } else { AuthReply::Retry(tries + 1) }
}

fn finish_if(id: u64, outcome: Outcome) {
    if LIVE.with(|l| current(id, l.borrow().as_ref().map(|l| l.id))) { finish(outcome); }
}

fn submit(id: u64) {
    let job = LIVE.with(|l| {
        let mut l = l.borrow_mut();
        let live = l.as_mut().filter(|l| l.id == id)?;
        if live.dialog.busy { return None; }
        live.dialog.busy = true;
        Some((live.id, live.request.cookie.clone(), live.request.users.clone(), live.dialog.entry.clone(), live.dialog.go.clone(), live.dialog.error.clone(), live.dialog.pick.clone()))
    });
    let Some((id, cookie, users, entry, go, error, pick)) = job else { return };
    let text = entry.text();
    if text.is_empty() || text.contains('\n') {
        LIVE.with(|l| { if let Some(live) = l.borrow_mut().as_mut().filter(|l| l.id == id) { live.dialog.busy = false; } });
        if text.contains('\n') {
            entry.set_text("");
            error.set_text("The password can't contain a line break");
            error.set_visible(true);
        }
        return;
    }
    let secret = Secret(text.as_bytes().to_vec());
    drop(text);
    entry.set_text("");
    entry.set_sensitive(false);
    go.set_sensitive(false);
    go.set_label("Checking…");
    error.set_visible(false);
    let at = pick.as_ref().map_or(0, |p| p.selected() as usize);
    let Some(user) = users.get(at).or(users.first()).map(|u| u.1.clone()) else { return };
    let (tx, rx) = async_channel::bounded(1);
    let cancel = LIVE.with(|l| l.borrow().as_ref().filter(|l| l.id == id).map(|l| l.cancel.clone()));
    let Some(cancel) = cancel else { return };
    let spawned = std::thread::Builder::new().name("digit-auth".into()).spawn(move || {
        let _ = tx.send_blocking(authenticate(&user, &cookie, secret, &cancel));
    });
    if spawned.is_err() {
        finish_if(id, Outcome::Failed("Could not start authentication".into()));
        return;
    }
    glib::spawn_future_local(async move {
        let (ok, notes) = rx.recv().await.unwrap_or((false, Vec::new()));
        let reply = LIVE.with(|l| auth_reply(id, l.borrow().as_ref().map(|l| (l.id, l.dialog.tries)), ok));
        let tries = match reply {
            AuthReply::Stale => return,
            AuthReply::Done => { finish_if(id, Outcome::Done); return; }
            AuthReply::Failed => { finish_if(id, Outcome::Failed("Too many failed attempts".into())); return; }
            AuthReply::Retry(tries) => tries,
        };
        LIVE.with(|l| {
            if let Some(live) = l.borrow_mut().as_mut().filter(|l| l.id == id) {
                live.dialog.tries = tries;
                live.dialog.busy = false;
            }
        });
        let note = notes.into_iter().find(|n| !n.trim().is_empty()).unwrap_or_else(|| "That password didn't work. Try again.".into());
        error.set_text(&note);
        error.set_visible(true);
        entry.set_sensitive(true);
        go.set_sensitive(true);
        go.set_label("Authenticate");
        entry.grab_focus();
    });
}

fn cancel(cookie: &str) {
    QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        while let Some(i) = q.iter().position(|r| r.cookie == cookie) {
            if let Some(r) = q.remove(i) {
                let _ = r.reply.try_send(Outcome::Cancelled);
            }
        }
    });
    let live = LIVE.with(|l| l.borrow().as_ref().is_some_and(|l| l.request.cookie == cookie));
    if live {
        finish(Outcome::Cancelled);
    }
}

async fn session(conn: &zbus::Connection) -> Option<String> {
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(id);
        }
    }
    let mgr = zbus::Proxy::new(conn, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").await.ok()?;
    let path: zbus::zvariant::OwnedObjectPath = mgr.call("GetSession", &("auto",)).await.ok()?;
    let s = zbus::Proxy::new(conn, "org.freedesktop.login1", path.into_inner(), "org.freedesktop.login1.Session").await.ok()?;
    s.get_property::<String>("Id").await.ok()
}

fn subject(id: &str) -> (&'static str, HashMap<&'static str, Value<'_>>) {
    ("unix-session", HashMap::from([("session-id", Value::from(id))]))
}

async fn register(conn: &zbus::Connection, id: &str) -> Result<(), String> {
    let authority = zbus::Proxy::new(conn, POLKIT, "/org/freedesktop/PolicyKit1/Authority", "org.freedesktop.PolicyKit1.Authority").await.map_err(|e| e.to_string())?;
    let locale = std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into());
    authority.call::<_, _, ()>("RegisterAuthenticationAgent", &(subject(id), locale, PATH)).await.map_err(|e| e.to_string())
}

pub fn start(app: &Rc<App>) {
    APP.with(|a| *a.borrow_mut() = Rc::downgrade(app));
    let (tx, rx) = async_channel::unbounded::<Event>();
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::system().await else { return };
        if conn.object_server().at(PATH, Agent { tx }).await.is_err() {
            eprintln!("digit: polkit: could not export the agent");
            return;
        }
        let Some(id) = session(&conn).await else {
            eprintln!("digit: polkit: no login session");
            return;
        };
        let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else { return };
        let Ok(mut changes) = dbus.receive_name_owner_changed().await else { return };
        let mut attempts = 0u32;
        let mut said = false;
        loop {
            match register(&conn, &id).await {
                Ok(()) => {
                    REGISTERED.with(|r| *r.borrow_mut() = Some((id.clone(), conn.clone())));
                    eprintln!("digit: polkit agent registered for session {id}");
                    while let Some(change) = changes.next().await {
                        let Ok(args) = change.args() else { continue };
                        if args.name().as_str() == POLKIT {
                            REGISTERED.with(|r| r.borrow_mut().take());
                            if args.new_owner().is_some() { break; }
                        }
                    }
                    attempts = 0;
                    said = false;
                }
                Err(e) => {
                    if !said {
                        eprintln!("digit: polkit: {e}; retrying while another agent holds the session");
                        said = true;
                    }
                    attempts += 1;
                    glib::timeout_future(Duration::from_secs(if attempts < 24 { 5 } else { 30 })).await;
                }
            }
        }
    });
    glib::spawn_future_local(async move {
        while let Ok(event) = rx.recv().await {
            match event {
                Event::Begin(request) => {
                    QUEUE.with(|q| q.borrow_mut().push_back(request));
                    next();
                }
                Event::Cancel(cookie) => cancel(&cookie),
            }
        }
    });
}

pub fn stop() {
    let Some((id, conn)) = REGISTERED.with(|r| r.borrow_mut().take()) else { return };
    let _ = zbus::block_on(async {
        let authority = zbus::Proxy::new(&conn, POLKIT, "/org/freedesktop/PolicyKit1/Authority", "org.freedesktop.PolicyKit1.Authority").await?;
        authority.call::<_, _, ()>("UnregisterAuthenticationAgent", &(subject(&id), PATH)).await
    });
}

#[cfg(test)]
mod tests {
    use super::{AuthReply, auth_reply, converse, unescape};

    #[test]
    fn cancelled_results_cannot_finish_the_next_request() {
        for ok in [true, false] {
            assert_eq!(auth_reply(1, None, ok), AuthReply::Stale);
            assert_eq!(auth_reply(1, Some((2, 0)), ok), AuthReply::Stale);
            assert_eq!(auth_reply(1, Some((2, 2)), ok), AuthReply::Stale);
        }
        assert_eq!(auth_reply(2, Some((2, 0)), false), AuthReply::Retry(1));
        assert_eq!(auth_reply(2, Some((2, 2)), false), AuthReply::Failed);
        assert_eq!(auth_reply(2, Some((2, 2)), true), AuthReply::Done);
    }

    #[test]
    fn cancellation_closes_socket_and_is_idempotent() {
        use std::io::Read;
        let (socket, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let cancel = std::sync::Mutex::new(super::Cancellation { socket: Some(socket), ..Default::default() });
        super::cancel_worker(&cancel);
        super::cancel_worker(&cancel);
        assert!(cancel.lock().unwrap().cancelled);
        assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0);
    }

    #[test]
    fn unescapes_like_glib() {
        assert_eq!(unescape(r"PAM_PROMPT_ECHO_OFF Password: "), "PAM_PROMPT_ECHO_OFF Password: ");
        assert_eq!(unescape(r"a\tb\\c\101"), "a\tb\\cA");
    }

    #[test]
    fn answers_once_and_wipes_the_secret() {
        let mut input: &[u8] = b"PAM_TEXT_INFO hello\nPAM_PROMPT_ECHO_OFF Password: \nSUCCESS\n";
        let mut written = Vec::new();
        let mut secret = b"hunter2".to_vec();
        let (ok, notes) = converse(&mut input, &mut written, &mut secret);
        assert!(ok);
        assert_eq!(notes, vec!["hello".to_string()]);
        assert_eq!(written, b"hunter2\n");
        assert!(secret.iter().all(|b| *b == 0));
    }

    #[test]
    fn reports_failure() {
        let mut input: &[u8] = b"PAM_PROMPT_ECHO_OFF Password: \nPAM_ERROR_MSG Authentication failure\nFAILURE\n";
        let mut written = Vec::new();
        let mut secret = b"x".to_vec();
        let (ok, notes) = converse(&mut input, &mut written, &mut secret);
        assert!(!ok);
        assert_eq!(notes, vec!["Authentication failure".to_string()]);
    }
}
