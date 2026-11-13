use crate::app::{App, Peek};
use gtk::glib;
use pulseaudio::protocol::{self, ChannelVolume, Volume};
use std::cell::{Cell, RefCell};
use std::ffi::CString;
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

const STEP: f64 = 0.05;

static LIVE: AtomicBool = AtomicBool::new(false);

pub fn live() -> bool {
    LIVE.load(Ordering::Relaxed)
}

#[derive(Clone, Copy)]
pub enum Ask {
    Read,
    Heard,
    Level(f64),
    Mute(bool),
    Step(f64),
    Flip,
    Inputs,
    MicFlip,
    Guard(u32, bool),
}

#[derive(Clone, Debug)]
struct Stream {
    index: u32,
    owner: String,
    name: String,
    binary: String,
    duplex: bool,
    muted: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Show {
    Never,
    Changed,
    Always,
}

impl Ask {
    pub fn parse(action: &str) -> Option<Ask> {
        match action {
            "volume-up" => Some(Ask::Step(STEP)),
            "volume-down" => Some(Ask::Step(-STEP)),
            "mute" => Some(Ask::Flip),
            "mic-mute" => Some(Ask::MicFlip),
            _ => None,
        }
    }

    fn show(self) -> Show {
        match self {
            Ask::Read | Ask::Inputs | Ask::MicFlip | Ask::Guard(..) => Show::Never,
            Ask::Heard | Ask::Level(_) | Ask::Mute(_) => Show::Changed,
            Ask::Step(_) | Ask::Flip => Show::Always,
        }
    }

    fn writes(self) -> bool {
        !matches!(self, Ask::Read | Ask::Heard | Ask::Inputs | Ask::MicFlip | Ask::Guard(..))
    }
}

fn stepped(level: f64, delta: f64) -> f64 {
    let grid = delta.abs().max(0.01);
    let at = level / grid;
    let n = if delta > 0.0 { (at + 0.02).floor() + 1.0 } else { (at - 0.02).ceil() - 1.0 };
    (n * grid).clamp(0.0, level.max(1.0))
}

#[derive(Clone, PartialEq)]
struct Sink {
    level: f64,
    muted: bool,
    name: String,
    desc: String,
}

enum News {
    Changed(bool),
    Inputs,
    Mic(Vec<Stream>, Option<bool>),
    Sink { sink: Sink, seq: u64, show: Show },
}

thread_local! {
    static ASK: RefCell<Option<mpsc::SyncSender<(u64, Ask)>>> = const { RefCell::new(None) };
    static SEQ: Cell<u64> = const { Cell::new(0) };
    static GUESS: Cell<u64> = const { Cell::new(0) };
}

struct Link {
    sock: BufReader<UnixStream>,
    version: u16,
    seq: u32,
}

impl Link {
    fn open(name: &str) -> Option<Self> {
        let path = pulseaudio::socket_path_from_env()?;
        let stream = UnixStream::connect(path).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
        stream.set_write_timeout(Some(Duration::from_secs(3))).ok()?;
        let mut sock = BufReader::new(stream);
        let cookie = pulseaudio::cookie_path_from_env().and_then(|p| std::fs::read(p).ok()).unwrap_or_default();
        let auth = protocol::AuthParams { version: protocol::MAX_VERSION, supports_shm: false, supports_memfd: false, cookie };
        protocol::write_command_message(sock.get_mut(), 0, &protocol::Command::Auth(auth), protocol::MAX_VERSION).ok()?;
        let (_, reply) = protocol::read_reply_message::<protocol::AuthReply>(&mut sock, protocol::MAX_VERSION).ok()?;
        let version = protocol::MAX_VERSION.min(reply.version);
        let mut props = protocol::Props::new();
        props.set(protocol::Prop::ApplicationName, CString::new(name).ok()?);
        protocol::write_command_message(sock.get_mut(), 1, &protocol::Command::SetClientName(props), version).ok()?;
        protocol::read_reply_message::<protocol::SetClientNameReply>(&mut sock, version).ok()?;
        Some(Self { sock, version, seq: 2 })
    }

    fn send(&mut self, command: protocol::Command) -> Option<u32> {
        let seq = self.seq;
        self.seq += 1;
        protocol::write_command_message(self.sock.get_mut(), seq, &command, self.version).ok()?;
        Some(seq)
    }

    fn sink(&mut self) -> Option<(protocol::SinkInfo, Sink)> {
        self.send(protocol::Command::GetServerInfo)?;
        let (_, server) = protocol::read_reply_message::<protocol::ServerInfo>(&mut self.sock, self.version).ok()?;
        let name = server.default_sink_name?;
        self.send(protocol::Command::GetSinkInfo(protocol::GetSinkInfo { index: None, name: Some(name) }))?;
        let (_, info) = protocol::read_reply_message::<protocol::SinkInfo>(&mut self.sock, self.version).ok()?;
        let loudest = info.cvolume.channels().iter().map(Volume::as_u32).max().unwrap_or(0);
        let sink = Sink {
            level: loudest as f64 / Volume::NORM.as_u32() as f64,
            muted: info.muted,
            name: info.name.to_string_lossy().into_owned(),
            desc: info.description.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        Some((info, sink))
    }

    fn level_to(&mut self, info: &protocol::SinkInfo, level: f64) -> Option<()> {
        let raw = (level.clamp(0.0, 1.5) * Volume::NORM.as_u32() as f64).round() as u32;
        let mut volume = ChannelVolume::empty();
        for _ in info.cvolume.channels() {
            volume.push(Volume::from_u32_clamped(raw));
        }
        let params = protocol::SetDeviceVolumeParams { device_index: Some(info.index), device_name: None, volume };
        self.send(protocol::Command::SetSinkVolume(params))?;
        protocol::read_ack_message(&mut self.sock).ok().map(drop)
    }

    fn mute_to(&mut self, info: &protocol::SinkInfo, mute: bool) -> Option<()> {
        let params = protocol::SetDeviceMuteParams { device_index: Some(info.index), device_name: None, mute };
        self.send(protocol::Command::SetSinkMute(params))?;
        protocol::read_ack_message(&mut self.sock).ok().map(drop)
    }

    fn mic_flip(&mut self) -> Option<()> {
        self.send(protocol::Command::GetServerInfo)?;
        let (_, server) = protocol::read_reply_message::<protocol::ServerInfo>(&mut self.sock, self.version).ok()?;
        let name = server.default_source_name?;
        self.send(protocol::Command::GetSourceInfo(protocol::GetSourceInfo { index: None, name: Some(name) }))?;
        let (_, info) = protocol::read_reply_message::<protocol::SourceInfo>(&mut self.sock, self.version).ok()?;
        let params = protocol::SetDeviceMuteParams { device_index: Some(info.index), device_name: None, mute: !info.muted };
        self.send(protocol::Command::SetSourceMute(params))?;
        protocol::read_ack_message(&mut self.sock).ok().map(drop)
    }

    fn guard(&mut self, index: u32, mute: bool) -> Option<()> {
        self.send(protocol::Command::SetSourceOutputMute(protocol::SetStreamMuteParams { index, mute }))?;
        protocol::read_ack_message(&mut self.sock).ok().map(drop)
    }

    fn inputs(&mut self) -> Option<(Vec<Stream>, Option<bool>)> {
        self.send(protocol::Command::GetServerInfo)?;
        let (_, server) = protocol::read_reply_message::<protocol::ServerInfo>(&mut self.sock, self.version).ok()?;
        self.send(protocol::Command::GetSourceInfoList)?;
        let (_, sources) = protocol::read_reply_message::<protocol::SourceInfoList>(&mut self.sock, self.version).ok()?;
        self.send(protocol::Command::GetSourceOutputInfoList)?;
        let (_, outputs) = protocol::read_reply_message::<protocol::SourceOutputInfoList>(&mut self.sock, self.version).ok()?;
        self.send(protocol::Command::GetSinkInputInfoList)?;
        let (_, plays) = protocol::read_reply_message::<protocol::SinkInputInfoList>(&mut self.sock, self.version).ok()?;
        let monitors: Vec<u32> = sources.iter().filter(|s| s.monitor_of_sink_index.is_some()).map(|s| s.index).collect();
        let live: Vec<u32> = sources.iter().filter(|s| s.state == protocol::SourceState::Running).map(|s| s.index).collect();
        let owner = |props: &protocol::Props| {
            let get = |k: &std::ffi::CStr| String::from_utf8_lossy(props.get_bytes(k).unwrap_or_default()).trim_end_matches('\0').to_string();
            [c"pipewire.access.portal.app_id", c"application.process.binary", c"application.process.id"].into_iter().map(get).find(|v| !v.is_empty()).unwrap_or_default()
        };
        let playing: Vec<String> = plays.iter().filter(|p| !p.corked).map(|p| owner(&p.props)).filter(|o| !o.is_empty()).collect();
        let mut names: Vec<Stream> = Vec::new();
        let text = |o: &protocol::SourceOutputInfo, p: protocol::Prop| String::from_utf8_lossy(o.props.get(p).unwrap_or_default()).trim_end_matches('\0').to_string();
        for o in outputs.iter().filter(|o| !o.corked && !monitors.contains(&o.source_index) && live.contains(&o.source_index)) {
            let me = owner(&o.props);
            let duplex = !me.is_empty() && playing.contains(&me);
            names.push(Stream { index: o.index, name: text(o, protocol::Prop::ApplicationName), binary: text(o, protocol::Prop::ApplicationProcessBinary), owner: me, duplex, muted: o.muted });
        }
        let muted = server.default_source_name.as_ref().and_then(|d| sources.iter().find(|s| s.name.as_c_str() == d.as_c_str())).map(|s| s.muted);
        Some((names, muted))
    }

    fn apply(&mut self, ask: Ask) -> Option<Sink> {
        let (info, now) = self.sink()?;
        match ask {
            Ask::Read | Ask::Heard | Ask::Inputs | Ask::MicFlip | Ask::Guard(..) => return Some(now),
            Ask::Level(v) => self.level_to(&info, v)?,
            Ask::Mute(m) => self.mute_to(&info, m)?,
            Ask::Step(d) => {
                if d > 0.0 && now.muted {
                    self.mute_to(&info, false)?;
                }
                self.level_to(&info, stepped(now.level, d))?;
            }
            Ask::Flip => self.mute_to(&info, !now.muted)?,
        }
        self.sink().map(|(_, s)| s)
    }
}

/// One row of the mixer: a device or an app stream playing to (or recording from) it.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub index: u32,
    pub name: String,
    pub binary: String,
    pub level: f64,
    pub muted: bool,
    channels: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mix {
    pub device: Option<Channel>,
    pub apps: Vec<Channel>,
}

fn loudest(v: &ChannelVolume) -> f64 {
    v.channels().iter().map(Volume::as_u32).max().unwrap_or(0) as f64 / Volume::NORM.as_u32() as f64
}

fn even(channels: usize, level: f64) -> ChannelVolume {
    let raw = (level.clamp(0.0, 1.5) * Volume::NORM.as_u32() as f64).round() as u32;
    let mut volume = ChannelVolume::empty();
    for _ in 0..channels.max(1) {
        volume.push(Volume::from_u32_clamped(raw));
    }
    volume
}

/// The app behind a stream as (name, binary). Streams no app owns, like loopbacks, have none.
fn owner(props: &protocol::Props) -> Option<(String, String)> {
    let text = |p: protocol::Prop| String::from_utf8_lossy(props.get(p).unwrap_or_default()).trim_end_matches('\0').to_string();
    let name = text(protocol::Prop::ApplicationName);
    let mut binary = text(protocol::Prop::ApplicationProcessBinary);
    if binary.is_empty() {
        let pid = text(protocol::Prop::ApplicationProcessId);
        binary = std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|c| c.trim().to_string()).unwrap_or_default();
    }
    (!name.is_empty() || !binary.is_empty()).then_some((name, binary))
}

impl Link {
    fn mix(&mut self, input: bool) -> Option<Mix> {
        self.send(protocol::Command::GetServerInfo)?;
        let (_, server) = protocol::read_reply_message::<protocol::ServerInfo>(&mut self.sock, self.version).ok()?;
        let mut mix = Mix::default();
        if input {
            self.send(protocol::Command::GetSourceInfoList)?;
            let (_, sources) = protocol::read_reply_message::<protocol::SourceInfoList>(&mut self.sock, self.version).ok()?;
            self.send(protocol::Command::GetSourceOutputInfoList)?;
            let (_, outputs) = protocol::read_reply_message::<protocol::SourceOutputInfoList>(&mut self.sock, self.version).ok()?;
            let monitors: Vec<u32> = sources.iter().filter(|s| s.monitor_of_sink_index.is_some()).map(|s| s.index).collect();
            mix.device = server.default_source_name.as_ref().and_then(|d| sources.iter().find(|s| s.name.as_c_str() == d.as_c_str())).map(|s| Channel {
                index: s.index,
                name: s.description.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|| s.name.to_string_lossy().into_owned()),
                binary: String::new(),
                level: loudest(&s.cvolume),
                muted: s.muted,
                channels: s.cvolume.channels().len(),
            });
            for o in outputs.iter().filter(|o| !monitors.contains(&o.source_index)) {
                if let Some((name, binary)) = owner(&o.props) {
                    mix.apps.push(Channel { index: o.index, name, binary, level: loudest(&o.cvolume), muted: o.muted, channels: o.cvolume.channels().len() });
                }
            }
        } else {
            let (_, sink) = self.sink()?;
            self.send(protocol::Command::GetSinkInfo(protocol::GetSinkInfo { index: None, name: Some(CString::new(sink.name.clone()).ok()?) }))?;
            let (_, info) = protocol::read_reply_message::<protocol::SinkInfo>(&mut self.sock, self.version).ok()?;
            mix.device = Some(Channel { index: info.index, name: if sink.desc.is_empty() { sink.name } else { sink.desc }, binary: String::new(), level: sink.level, muted: sink.muted, channels: info.cvolume.channels().len() });
            self.send(protocol::Command::GetSinkInputInfoList)?;
            let (_, plays) = protocol::read_reply_message::<protocol::SinkInputInfoList>(&mut self.sock, self.version).ok()?;
            for p in plays.iter().filter(|p| p.has_volume) {
                if let Some((name, binary)) = owner(&p.props) {
                    mix.apps.push(Channel { index: p.index, name, binary, level: loudest(&p.cvolume), muted: p.muted, channels: p.cvolume.channels().len() });
                }
            }
        }
        Some(mix)
    }

    fn set(&mut self, input: bool, device: bool, ch: &Channel, level: Option<f64>, mute: Option<bool>) -> Option<()> {
        use protocol::{Command as C, SetDeviceMuteParams as DM, SetDeviceVolumeParams as DV, SetStreamMuteParams as SM, SetStreamVolumeParams as SV};
        if let Some(level) = level {
            let volume = even(ch.channels, level);
            let index = ch.index;
            self.send(match (input, device) {
                (false, true) => C::SetSinkVolume(DV { device_index: Some(index), device_name: None, volume }),
                (true, true) => C::SetSourceVolume(DV { device_index: Some(index), device_name: None, volume }),
                (false, false) => C::SetSinkInputVolume(SV { index, volume }),
                (true, false) => C::SetSourceOutputVolume(SV { index, volume }),
            })?;
            protocol::read_ack_message(&mut self.sock).ok()?;
        }
        if let Some(mute) = mute {
            let index = ch.index;
            self.send(match (input, device) {
                (false, true) => C::SetSinkMute(DM { device_index: Some(index), device_name: None, mute }),
                (true, true) => C::SetSourceMute(DM { device_index: Some(index), device_name: None, mute }),
                (false, false) => C::SetSinkInputMute(SM { index, mute }),
                (true, false) => C::SetSourceOutputMute(SM { index, mute }),
            })?;
            protocol::read_ack_message(&mut self.sock).ok()?;
        }
        Some(())
    }
}

/// An output or input device to choose between.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// The server's name for it, used to make it the default.
    pub name: String,
    pub desc: String,
    pub default: bool,
    /// A Material Symbols glyph for it: speaker, headphones, mic, a screen.
    pub glyph: &'static str,
}

fn glyph_for(input: bool, props: &protocol::Props, desc: &str) -> &'static str {
    let text = |p: protocol::Prop| String::from_utf8_lossy(props.get(p).unwrap_or_default()).trim_end_matches('\0').to_lowercase();
    let form = text(protocol::Prop::DeviceFormFactor);
    let bus = text(protocol::Prop::DeviceBus);
    let desc = desc.to_lowercase();
    let worn = matches!(form.as_str(), "headphone" | "headset" | "hands-free") || bus == "bluetooth" || ["headphone", "headset", "buds", "airpods"].iter().any(|w| desc.contains(w));
    match (input, worn) {
        (_, true) => "headphones",
        (true, false) => "mic",
        (false, false) if ["hdmi", "displayport"].iter().any(|w| desc.contains(w)) => "tv",
        (false, false) => "speaker",
    }
}

impl Link {
    fn devices(&mut self, input: bool) -> Option<Vec<Device>> {
        self.send(protocol::Command::GetServerInfo)?;
        let (_, server) = protocol::read_reply_message::<protocol::ServerInfo>(&mut self.sock, self.version).ok()?;
        let named = |name: &std::ffi::CStr, desc: &Option<CString>| (name.to_string_lossy().into_owned(), desc.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string_lossy().into_owned()));
        let mut out = Vec::new();
        if input {
            self.send(protocol::Command::GetSourceInfoList)?;
            let (_, sources) = protocol::read_reply_message::<protocol::SourceInfoList>(&mut self.sock, self.version).ok()?;
            let default = server.default_source_name.as_ref().map(|d| d.to_string_lossy().into_owned());
            // A sink's monitor isn't a microphone.
            for s in sources.iter().filter(|s| s.monitor_of_sink_index.is_none()) {
                let (name, desc) = named(&s.name, &s.description);
                out.push(Device { default: default.as_deref() == Some(name.as_str()), glyph: glyph_for(true, &s.props, &desc), name, desc });
            }
        } else {
            self.send(protocol::Command::GetSinkInfoList)?;
            let (_, sinks) = protocol::read_reply_message::<protocol::SinkInfoList>(&mut self.sock, self.version).ok()?;
            let default = server.default_sink_name.as_ref().map(|d| d.to_string_lossy().into_owned());
            for s in sinks.iter() {
                let (name, desc) = named(&s.name, &s.description);
                out.push(Device { default: default.as_deref() == Some(name.as_str()), glyph: glyph_for(false, &s.props, &desc), name, desc });
            }
        }
        Some(out)
    }

    fn set_default(&mut self, input: bool, name: &str) -> Option<()> {
        let name = CString::new(name).ok()?;
        self.send(if input { protocol::Command::SetDefaultSource(name) } else { protocol::Command::SetDefaultSink(name) })?;
        protocol::read_ack_message(&mut self.sock).ok().map(|_| ())
    }
}

/// The output (or input) devices, the default one marked. Blocks; run it off the main thread.
pub fn devices(input: bool) -> Vec<Device> {
    Link::open("digit-mixer").and_then(|mut l| l.devices(input)).unwrap_or_default()
}

/// Makes a device the default output (or input); apps following the default move with it.
/// Blocks; run it off the main thread.
pub fn set_default(input: bool, name: &str) -> bool {
    Link::open("digit-mixer").and_then(|mut l| l.set_default(input, name)).is_some()
}

/// Reads the output (or input) device and the app streams on it. Blocks; run it off the main thread.
pub fn mix(input: bool) -> Option<Mix> {
    Link::open("digit-mixer")?.mix(input)
}

/// Sets one mixer row's level and/or mute. Blocks; run it off the main thread.
pub fn set_channel(input: bool, device: bool, ch: &Channel, level: Option<f64>, mute: Option<bool>) -> bool {
    Link::open("digit-mixer").and_then(|mut l| l.set(input, device, ch, level, mute)).is_some()
}

/// The name the rest of the shell shows for an app stream.
pub fn who(app: &App, ch: &Channel) -> String {
    speaker(app, &ch.name, &ch.binary)
}

fn wpctl(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("wpctl").args(args).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn fallback(ask: Ask) -> Option<Sink> {
    let sink = "@DEFAULT_AUDIO_SINK@";
    let read = || {
        let out = wpctl(&["get-volume", sink])?;
        let level = out.split_whitespace().nth(1)?.parse::<f64>().ok()?;
        Some(Sink { level, muted: out.contains("[MUTED]"), name: String::new(), desc: String::new() })
    };
    let now = read()?;
    match ask {
        Ask::Read | Ask::Heard | Ask::Inputs | Ask::Guard(..) => return Some(now),
        Ask::MicFlip => {
            wpctl(&["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])?;
        }
        Ask::Level(v) => {
            wpctl(&["set-volume", sink, &format!("{:.3}", v.clamp(0.0, 1.5))])?;
        }
        Ask::Mute(m) => {
            wpctl(&["set-mute", sink, if m { "1" } else { "0" }])?;
        }
        Ask::Step(d) => {
            if d > 0.0 && now.muted {
                wpctl(&["set-mute", sink, "0"])?;
            }
            wpctl(&["set-volume", sink, &format!("{:.3}", stepped(now.level, d))])?;
        }
        Ask::Flip => {
            wpctl(&["set-mute", sink, "toggle"])?;
        }
    }
    read()
}

fn run(link: &mut Option<Link>, ask: Ask) -> Option<Sink> {
    for _ in 0..2 {
        if link.is_none() {
            *link = Link::open("digit");
        }
        let Some(l) = link.as_mut() else { return fallback(ask) };
        match l.apply(ask) {
            Some(sink) => return Some(sink),
            None => *link = None,
        }
    }
    None
}

pub fn once(ask: Ask) -> bool {
    run(&mut None, ask).is_some()
}

fn listen(news: async_channel::Sender<News>) {
    let mut wait = 2;
    loop {
        if let Some(mut link) = Link::open("digit") {
            let mask = protocol::SubscriptionMask::SINK | protocol::SubscriptionMask::SERVER | protocol::SubscriptionMask::SOURCE | protocol::SubscriptionMask::SOURCE_OUTPUT | protocol::SubscriptionMask::SINK_INPUT;
            if link.send(protocol::Command::Subscribe(mask)).is_some() && protocol::read_ack_message(&mut link.sock).is_ok() {
                let _ = link.sock.get_mut().set_read_timeout(None);
                wait = 2;
                LIVE.store(true, Ordering::Relaxed);
                if news.send_blocking(News::Changed(false)).is_err() || news.send_blocking(News::Inputs).is_err() {
                    return;
                }
                while let Ok((_, command)) = protocol::read_command_message(&mut link.sock, link.version) {
                    let protocol::Command::SubscribeEvent(event) = command else { continue };
                    let item = match event.event_facility {
                        protocol::SubscriptionEventFacility::Source | protocol::SubscriptionEventFacility::SourceOutput | protocol::SubscriptionEventFacility::SinkInput => News::Inputs,
                        _ => News::Changed(true),
                    };
                    if news.send_blocking(item).is_err() {
                        return;
                    }
                }
                LIVE.store(false, Ordering::Relaxed);
            }
        }
        if news.is_closed() {
            return;
        }
        std::thread::sleep(Duration::from_secs(wait));
        wait = (wait * 2).min(30);
    }
}

fn control(asks: mpsc::Receiver<(u64, Ask)>, news: async_channel::Sender<News>) {
    let mut link: Option<Link> = None;
    while let Ok(first) = asks.recv() {
        let mut batch = vec![first];
        batch.extend(asks.try_iter());
        if batch.iter().any(|b| matches!(b.1, Ask::Inputs | Ask::MicFlip | Ask::Guard(..))) {
            let flip = batch.iter().any(|b| matches!(b.1, Ask::MicFlip));
            let guards: Vec<(u32, bool)> = batch.iter().filter_map(|b| if let Ask::Guard(i, m) = b.1 { Some((i, m)) } else { None }).collect();
            batch.retain(|b| !matches!(b.1, Ask::Inputs | Ask::MicFlip | Ask::Guard(..)));
            for (index, mute) in guards {
                if link.is_none() {
                    link = Link::open("digit");
                }
                if link.as_mut().and_then(|l| l.guard(index, mute)).is_none() {
                    link = None;
                }
            }
            if flip {
                if link.is_none() {
                    link = Link::open("digit");
                }
                if link.as_mut().and_then(Link::mic_flip).is_none() {
                    link = None;
                    let _ = fallback(Ask::MicFlip);
                }
            }
            let names = (0..2).find_map(|_| {
                if link.is_none() {
                    link = Link::open("digit");
                }
                let found = link.as_mut()?.inputs();
                if found.is_none() {
                    link = None;
                }
                found
            });
            if let Some((names, muted)) = names {
                if news.send_blocking(News::Mic(names, muted)).is_err() {
                    return;
                }
            }
            if batch.is_empty() {
                continue;
            }
        }
        let seq = batch.iter().map(|b| b.0).max().unwrap_or(0);
        let show = batch.iter().map(|b| b.1.show()).max().unwrap_or(Show::Never);
        let mut plan: Vec<Ask> = Vec::new();
        for (_, ask) in batch.into_iter().filter(|b| b.1.writes()) {
            let merge = matches!((plan.last(), ask), (Some(Ask::Level(_)), Ask::Level(_)) | (Some(Ask::Mute(_)), Ask::Mute(_)));
            match (merge, plan.last_mut()) {
                (true, Some(last)) => *last = ask,
                _ => plan.push(ask),
            }
        }
        if plan.is_empty() {
            plan.push(Ask::Read);
        }
        let mut last = None;
        for ask in plan {
            last = run(&mut link, ask).or(last);
        }
        if let Some(sink) = last {
            if news.send_blocking(News::Sink { sink, seq, show }).is_err() {
                return;
            }
        }
    }
}

const GRACE: i64 = 2_500_000;

thread_local! {
    static SEEN: RefCell<std::collections::HashMap<u32, i64>> = RefCell::new(std::collections::HashMap::new());
    static GUARDED: RefCell<Option<Vec<(u32, String)>>> = const { RefCell::new(None) };
}

fn guard_file() -> std::path::PathBuf {
    glib::user_runtime_dir().join("digit-mic-guard")
}

fn guarded_load() -> Vec<(u32, String)> {
    std::fs::read_to_string(guard_file())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (i, o) = l.split_once(' ')?;
            Some((i.parse().ok()?, o.to_string()))
        })
        .collect()
}

fn guarded_save(list: &[(u32, String)]) {
    let text: String = list.iter().map(|(i, o)| format!("{i} {o}\n")).collect();
    if list.is_empty() {
        let _ = std::fs::remove_file(guard_file());
    } else {
        let _ = std::fs::write(guard_file(), text);
    }
}

fn guard(app: &Rc<App>, streams: &[Stream]) -> Vec<u32> {
    let on = app.prefs.borrow().mic_guard;
    let now = App::now();
    let mut list = GUARDED.with(|g| g.borrow_mut().get_or_insert_with(guarded_load).clone());
    let before = list.clone();
    list.retain(|(i, o)| streams.iter().any(|s| s.index == *i && s.owner == *o));
    let mut recheck = false;
    for s in streams {
        let who = speaker(app, &s.name, &s.binary);
        let voice = app.prefs.borrow().voice(&who) || app.prefs.borrow().voice(&s.binary);
        let ours = list.iter().any(|(i, _)| *i == s.index);
        if ours && (s.duplex || !on) {
            ask(Ask::Guard(s.index, false));
            list.retain(|(i, _)| *i != s.index);
            continue;
        }
        if s.duplex || !voice || !on || s.muted || ours {
            SEEN.with(|m| m.borrow_mut().remove(&s.index));
            continue;
        }
        let first = SEEN.with(|m| *m.borrow_mut().entry(s.index).or_insert(now));
        if now - first >= GRACE {
            ask(Ask::Guard(s.index, true));
            list.push((s.index, s.owner.clone()));
            app.request(crate::app::Peek::Osd { icon: "mic_off", text: format!("{who} blocked"), level: None });
        } else {
            recheck = true;
        }
    }
    SEEN.with(|m| m.borrow_mut().retain(|i, _| streams.iter().any(|s| s.index == *i)));
    if recheck {
        glib::timeout_add_local_once(Duration::from_millis((GRACE / 1000 + 200) as u64), || ask(Ask::Inputs));
    }
    if list != before {
        guarded_save(&list);
    }
    let ids = list.iter().map(|(i, _)| *i).collect();
    GUARDED.with(|g| *g.borrow_mut() = Some(list));
    ids
}

fn speaker(app: &App, name: &str, binary: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let generic = name.is_empty() || ["webrtc", "chromium", "voiceengine", "pulseaudio", "alsa", "pipewire"].iter().any(|g| lower.contains(g));
    if !binary.is_empty() && (generic || app.apps.find(binary).is_some()) {
        app.apps.display_name(binary)
    } else if !generic {
        name.to_string()
    } else {
        "An app".into()
    }
}

pub fn press(app: &Rc<App>, action: &str) -> bool {
    let Some(want) = Ask::parse(action) else { return false };
    if matches!(want, Ask::MicFlip) {
        ask(Ask::MicFlip);
        return true;
    }
    let Some((level, muted)) = app.volume.get() else {
        ask(want);
        return true;
    };
    let (next, mute) = match want {
        Ask::Step(d) => (stepped(level, d), muted && d <= 0.0),
        _ => (level, !muted),
    };
    app.volume.set(Some((next, mute)));
    if (next - level).abs() > 1e-9 {
        ask(Ask::Level(next));
    }
    if mute != muted {
        ask(Ask::Mute(mute));
    }
    app.history_changed();
    app.request(Peek::Volume { level: next, muted: mute });
    true
}

pub fn ask(a: Ask) {
    let seq = if matches!(a, Ask::Inputs) {
        0
    } else {
        SEQ.with(|s| {
            s.set(s.get() + 1);
            s.get()
        })
    };
    ASK.with(|s| {
        if let Some(tx) = s.borrow().as_ref() {
            if tx.try_send((seq, a)).is_ok() && matches!(a, Ask::Level(_) | Ask::Mute(_)) {
                GUESS.with(|g| g.set(seq));
            }
        }
    });
}

pub fn start(app: &Rc<App>) {
    let (news_tx, news_rx) = async_channel::unbounded::<News>();
    let (ask_tx, ask_rx) = mpsc::sync_channel::<(u64, Ask)>(64);
    ASK.with(|s| *s.borrow_mut() = Some(ask_tx));
    let events = news_tx.clone();
    let _ = std::thread::Builder::new().name("digit-audio-events".into()).stack_size(64 * 1024).spawn(move || listen(events));
    let _ = std::thread::Builder::new().name("digit-audio".into()).stack_size(64 * 1024).spawn(move || control(ask_rx, news_tx));
    ask(Ask::Read);
    let weak = Rc::downgrade(app);
    let heard: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    let owed = Cell::new(Show::Never);
    let probing = Rc::new(Cell::new(false));
    let known: RefCell<Option<String>> = RefCell::new(None);
    glib::spawn_future_local(async move {
        while let Ok(news) = news_rx.recv().await {
            match news {
                News::Changed(loud) => {
                    if heard.replace(Some(loud || heard.get().unwrap_or(false))).is_none() {
                        let heard = heard.clone();
                        glib::timeout_add_local_once(Duration::from_millis(25), move || {
                            let loud = heard.take().unwrap_or(false);
                            ask(if loud { Ask::Heard } else { Ask::Read });
                        });
                    }
                }
                News::Inputs => {
                    if !probing.replace(true) {
                        let probing = probing.clone();
                        glib::timeout_add_local_once(Duration::from_millis(150), move || {
                            probing.set(false);
                            ask(Ask::Inputs);
                        });
                    }
                }
                News::Mic(pairs, muted) => {
                    let Some(app) = weak.upgrade() else { break };
                    if app.mic_muted.replace(muted) != muted {
                        app.changed();
                    }
                    let mut names: Vec<String> = Vec::new();
                    let mut talk: Vec<String> = Vec::new();
                    let guarded = guard(&app, &pairs);
                    for s in &pairs {
                        let who = speaker(&app, &s.name, &s.binary);
                        if s.duplex && !talk.contains(&who) {
                            talk.push(who.clone());
                        }
                        if !s.muted && !guarded.contains(&s.index) && !names.contains(&who) {
                            names.push(who);
                        }
                    }
                    app.set_lights(|l| {
                        l.mic = names;
                        l.talk = talk;
                    });
                }
                News::Sink { sink, seq, show } => {
                    let Some(app) = weak.upgrade() else { break };
                    let show = owed.replace(Show::Never).max(show);
                    if seq < GUESS.with(Cell::get) {
                        owed.set(show);
                        continue;
                    }
                    if !sink.name.is_empty() {
                        let before = known.replace(Some(sink.name.clone()));
                        if before.is_some_and(|b| b != sink.name) {
                            let lower = format!("{} {}", sink.name, sink.desc).to_lowercase();
                            let icon = if ["bluez", "headphone", "headset", "buds", "airpods", "earphone", "barracuda"].iter().any(|k| lower.contains(k)) { "headphones" } else { "speaker" };
                            let name = if sink.desc.is_empty() { sink.name.clone() } else { sink.desc.clone() };
                            app.request(Peek::Output { name, icon });
                        }
                    }
                    let next = Some((sink.level, sink.muted));
                    let moved = app.volume.replace(next) != next;
                    if moved {
                        app.history_changed();
                    }
                    if show == Show::Always || (show == Show::Changed && moved) {
                        app.request(Peek::Volume { level: sink.level, muted: sink.muted });
                    } else if moved {
                        app.changed();
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::stepped;

    #[test]
    fn steps_snap_to_the_grid() {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(near(stepped(0.47, 0.05), 0.50));
        assert!(near(stepped(0.47, -0.05), 0.45));
        assert!(near(stepped(0.45, 0.05), 0.50));
        assert!(near(stepped(0.45, -0.05), 0.40));
        assert!(near(stepped(1.0, 0.05), 1.0));
        assert!(near(stepped(1.2, 0.05), 1.2));
        assert!(near(stepped(1.2, -0.05), 1.15));
        assert!(near(stepped(0.0, -0.05), 0.0));
        assert!(near(stepped(0.8000030517578125, -0.05), 0.75));
        assert!(near(stepped(0.8000030517578125, 0.05), 0.85));
        assert!(near(stepped(0.7999969482421875, -0.05), 0.75));
        assert!(near(stepped(0.7999969482421875, 0.05), 0.85));
    }
}
