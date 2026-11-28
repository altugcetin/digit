//! Power modes as the shell keeps them: the mode chosen for battery and for plugged in, a switch
//! between the two as the charger comes and goes, Power Saver held while the battery is low, and
//! lighter drawing of the shell itself in Power Saver. The power profiles service does the work
//! underneath (the CPU's energy preference, the firmware's platform profile); see `kit::power`.

use crate::app::{App, Peek};
use gtk::glib;
use kit::power::{self, Choice, Profiles, Source};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// How long signals may keep coming before reading again: a charger plugged in sends a burst.
const SETTLE: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Power {
    pub profiles: Option<Profiles>,
    pub source: Option<Source>,
    /// Power Saver is held because the battery is low.
    pub low: bool,
}

impl Power {
    pub fn on_battery(&self) -> bool {
        self.source.is_some_and(|s| s.on_battery)
    }

    pub fn has_battery(&self) -> bool {
        self.source.is_some_and(|s| s.battery)
    }

    /// The modes to choose from, when there are any to choose between.
    pub fn modes(&self) -> Option<&Profiles> {
        self.profiles.as_ref().filter(|p| p.list.len() > 1)
    }

    pub fn saving(&self) -> bool {
        self.profiles.as_ref().is_some_and(|p| p.active == power::SAVER)
    }
}

/// The low battery hold: its number with the service while held, and whether this time on
/// battery has had its hold already (released, or overruled by a mode picked since).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Hold {
    cookie: Option<u32>,
    spent: bool,
}

thread_local! {
    static HOLD: Cell<Hold> = Cell::new(Hold::default());
    static CHOICE: RefCell<Option<Choice>> = const { RefCell::new(None) };
}

#[derive(Debug, PartialEq)]
enum Step {
    Release,
    Set(String),
    Hold,
}

/// What a new reading calls for. `before` is the power source last seen, `None` at start, when
/// nothing is switched: the service keeps the mode it had.
fn plan(before: Option<Source>, now: Source, profiles: &Profiles, choice: &Choice, hold: Hold) -> Vec<Step> {
    let mut steps = Vec::new();
    let held = hold.cookie.is_some();
    if before.is_some_and(|b| b.on_battery != now.on_battery) {
        if held {
            steps.push(Step::Release);
        }
        steps.push(Step::Set(power::usable(choice.for_source(now.on_battery), &profiles.list)));
        return steps;
    }
    let low = now.on_battery && now.battery && now.level <= power::LOW;
    if low && choice.low_saver && !held && !hold.spent && profiles.active != power::SAVER && profiles.list.iter().any(|m| m == power::SAVER) {
        steps.push(Step::Hold);
    }
    if held && !choice.low_saver {
        steps.push(Step::Release);
    }
    steps
}

pub fn start(app: &Rc<App>) {
    CHOICE.with(|c| *c.borrow_mut() = Some(Choice::read(&kit::prefs::read())));
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::system().await else { return };
        let changes = power::changes(&conn).await.map(kit::signals::drained);
        loop {
            let (profiles, source) = futures_util::join!(power::profiles(&conn), power::source(&conn));
            let Some(app) = weak.upgrade() else { return };
            let before = app.power.borrow().source;
            let mut acted = false;
            if let (Some(p), Some(now)) = (&profiles, source) {
                // Let go of by someone else (a mode set elsewhere ends every hold).
                HOLD.with(|h| {
                    let mut hold = h.get();
                    if hold.cookie.is_some() && !p.held {
                        hold = Hold { cookie: None, spent: true };
                    }
                    if !now.on_battery {
                        hold.spent = false;
                    }
                    h.set(hold);
                });
                let choice = CHOICE.with(|c| c.borrow().clone()).unwrap_or_else(|| Choice::read(&kit::prefs::read()));
                for step in plan(before, now, p, &choice, HOLD.with(Cell::get)) {
                    act(&conn, p, step).await;
                    acted = true;
                }
            }
            // What the service is in now that it did as asked.
            let profiles = if acted { power::profiles(&conn).await.or(profiles) } else { profiles };
            show(&app, Power { low: HOLD.with(Cell::get).cookie.is_some(), profiles, source });
            crate::power::refresh(&app);
            drop(app);
            match &changes {
                Some(rx) => {
                    if rx.recv().await.is_err() {
                        return;
                    }
                    glib::timeout_future(SETTLE).await;
                    kit::signals::pending(rx);
                }
                None => glib::timeout_future(Duration::from_secs(30)).await,
            }
        }
    });
}

async fn act(conn: &zbus::Connection, p: &Profiles, step: Step) {
    match step {
        Step::Release => {
            if let Some(cookie) = HOLD.with(|h| h.replace(Hold { cookie: None, spent: h.get().spent }).cookie) {
                let _ = power::release(conn, p, cookie).await;
            }
        }
        Step::Set(mode) => {
            if mode != p.active
                && let Err(e) = power::set(conn, p, &mode).await
            {
                eprintln!("digit: power mode: {e}");
            }
        }
        Step::Hold => match power::hold(conn, p, power::SAVER, "The battery is low").await {
            // The low battery warning (`power`) says so already; the bar shows the leaf.
            Ok(cookie) => HOLD.with(|h| h.set(Hold { cookie: Some(cookie), spent: true })),
            Err(e) => {
                HOLD.with(|h| h.set(Hold { cookie: None, spent: true }));
                eprintln!("digit: power saver for a low battery: {e}");
            }
        },
    }
}

/// Takes in a new reading: the shell's own drawing in Power Saver, the bar and Control Center.
fn show(app: &Rc<App>, now: Power) {
    if *app.power.borrow() == now {
        return;
    }
    let before = app.power.borrow().source;
    let source = app.power.borrow().on_battery() != now.on_battery();
    let charger = before.is_some_and(|b| now.source.is_some_and(|n| b.on_battery != n.on_battery));
    let unplugged = now.on_battery();
    *app.power.borrow_mut() = now;
    if source {
        crate::idle::apply(app);
    }
    if charger {
        crate::brightness::battery_dim(unplugged);
    }
    lighten(app);
    app.changed();
    if let Some(c) = app.control.borrow().clone() {
        c.changed();
    }
}

/// Draws lighter in Power Saver when that's wanted.
fn lighten(app: &Rc<App>) {
    let light = CHOICE.with(|c| c.borrow().as_ref().is_none_or(|c| c.light));
    if kit::effects::set_saving(light && app.power.borrow().saving()) {
        app.effects_moved();
    }
}

/// The modes chosen for battery and plugged in, as last read from the settings.
pub fn choice() -> Choice {
    CHOICE.with(|c| c.borrow().clone()).unwrap_or_else(|| Choice::read(&kit::prefs::read()))
}

/// A mode set by hand ends the low battery's hold (the service lets go of every hold when a mode
/// is set), and it isn't taken again until the next time on battery.
fn overrule() {
    HOLD.with(|h| {
        let hold = h.get();
        h.set(Hold { cookie: None, spent: hold.spent || hold.cookie.is_some() });
    });
}

/// Steps to the next mode, and keeps it for the power source in use.
pub fn step(app: &Rc<App>) {
    let p = app.power.borrow().clone();
    let Some(mode) = p.modes().and_then(|m| power::next(&m.active, &m.list)) else { return };
    choose(app, p.on_battery(), &mode);
}

/// Keeps `mode` for battery or for plugged in, and switches to it now when that's the source in
/// use. A mode picked by hand ends a low battery's hold until the next time on battery.
pub fn choose(app: &Rc<App>, on_battery: bool, mode: &str) {
    if let Err(e) = kit::prefs::set(Choice::key(on_battery), mode.into()) {
        app.request(Peek::Note { text: format!("Couldn't keep the power mode: {e}") });
    }
    CHOICE.with(|c| *c.borrow_mut() = Some(Choice::read(&kit::prefs::read())));
    let p = app.power.borrow().clone();
    let Some(profiles) = p.profiles.clone() else { return };
    if p.on_battery() != on_battery || profiles.active == mode {
        return;
    }
    overrule();
    let mut now = p.clone();
    if let Some(pr) = now.profiles.as_mut() {
        pr.active = mode.to_owned();
        pr.held = false;
    }
    now.low = false;
    show(app, now);
    let (weak, mode) = (Rc::downgrade(app), mode.to_owned());
    glib::spawn_future_local(async move {
        let result = match crate::bus::system().await {
            Ok(conn) => power::set(&conn, &profiles, &mode).await,
            Err(e) => Err(e.to_string()),
        };
        if let (Err(e), Some(app)) = (result, weak.upgrade()) {
            app.request(Peek::Note { text: format!("Couldn't switch to {}: {e}", power::label(&mode)) });
        }
    });
}

/// After the settings changed: a new mode for the source in use applies now, and the low battery
/// hold and lighter drawing follow their switches.
pub fn prefs_changed(app: &Rc<App>) {
    let choice = Choice::read(&kit::prefs::read());
    let Some(before) = CHOICE.with(|c| c.borrow_mut().replace(choice.clone())) else { return };
    if before == choice {
        return;
    }
    lighten(app);
    let p = app.power.borrow().clone();
    let (Some(profiles), Some(source)) = (p.profiles.clone(), p.source) else { return };
    let mode = power::usable(choice.for_source(source.on_battery), &profiles.list);
    let switch = before.for_source(source.on_battery) != choice.for_source(source.on_battery) && mode != profiles.active;
    let release = before.low_saver && !choice.low_saver && HOLD.with(Cell::get).cookie.is_some();
    if !switch && !release {
        return;
    }
    // The service's signals bring the new mode back to the watch in `start`.
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::system().await else { return };
        if release {
            act(&conn, &profiles, Step::Release).await;
        }
        if switch {
            overrule();
            act(&conn, &profiles, Step::Set(mode)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profiles(active: &str) -> Profiles {
        Profiles { active: active.into(), list: power::MODES.iter().map(|m| m.to_string()).collect(), ..Default::default() }
    }

    fn source(on_battery: bool, level: f64) -> Source {
        Source { on_battery, battery: true, level }
    }

    fn choice(battery: &str, plugged: &str) -> Choice {
        Choice { battery: battery.into(), plugged: plugged.into(), low_saver: true, light: true }
    }

    #[test]
    fn the_charger_coming_and_going_switches_to_its_mode() {
        let c = choice(power::SAVER, power::PERFORMANCE);
        let unplugged = plan(Some(source(false, 80.0)), source(true, 80.0), &profiles(power::PERFORMANCE), &c, Hold::default());
        assert_eq!(unplugged, [Step::Set(power::SAVER.into())]);
        let plugged = plan(Some(source(true, 80.0)), source(false, 80.0), &profiles(power::SAVER), &c, Hold::default());
        assert_eq!(plugged, [Step::Set(power::PERFORMANCE.into())]);
        // A mode the computer lacks becomes Balanced.
        let two = Profiles { list: vec![power::SAVER.into(), power::BALANCED.into()], ..profiles(power::SAVER) };
        assert_eq!(plan(Some(source(true, 80.0)), source(false, 80.0), &two, &c, Hold::default()), [Step::Set(power::BALANCED.into())]);
    }

    #[test]
    fn starting_and_steady_readings_leave_the_mode_alone() {
        let c = choice(power::SAVER, power::PERFORMANCE);
        assert!(plan(None, source(true, 80.0), &profiles(power::PERFORMANCE), &c, Hold::default()).is_empty());
        assert!(plan(Some(source(true, 81.0)), source(true, 80.0), &profiles(power::BALANCED), &c, Hold::default()).is_empty());
    }

    #[test]
    fn a_low_battery_holds_power_saver_once_until_plugged_in() {
        let c = choice(power::BALANCED, power::BALANCED);
        let p = profiles(power::BALANCED);
        assert_eq!(plan(Some(source(true, 21.0)), source(true, 20.0), &p, &c, Hold::default()), [Step::Hold]);
        // Not again while held, nor once this time on battery has had its hold.
        assert!(plan(Some(source(true, 20.0)), source(true, 19.0), &p, &c, Hold { cookie: Some(7), spent: true }).is_empty());
        assert!(plan(Some(source(true, 20.0)), source(true, 19.0), &p, &c, Hold { cookie: None, spent: true }).is_empty());
        // Already saving, or turned off: nothing to hold.
        assert!(plan(None, source(true, 10.0), &profiles(power::SAVER), &c, Hold::default()).is_empty());
        let off = Choice { low_saver: false, ..c.clone() };
        assert!(plan(None, source(true, 10.0), &p, &off, Hold::default()).is_empty());
        // Plugging in lets go of it and goes to the plugged-in mode.
        assert_eq!(plan(Some(source(true, 15.0)), source(false, 15.0), &profiles(power::SAVER), &c, Hold { cookie: Some(7), spent: true }), [Step::Release, Step::Set(power::BALANCED.into())]);
        // Turned off while held: let go.
        assert_eq!(plan(Some(source(true, 15.0)), source(true, 14.0), &profiles(power::SAVER), &off, Hold { cookie: Some(7), spent: true }), [Step::Release]);
    }

    #[test]
    fn a_computer_without_a_battery_is_never_low() {
        let c = choice(power::BALANCED, power::BALANCED);
        let desktop = Source { on_battery: true, battery: false, level: 0.0 };
        assert!(plan(None, desktop, &profiles(power::BALANCED), &c, Hold::default()).is_empty());
    }
}
