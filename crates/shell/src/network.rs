//! The network icon in the bar: what the computer is connected through and how strong its
//! Wi-Fi is, read from NetworkManager and again whenever it says something changed.

use crate::app::App;
use gtk::glib;
use std::rc::Rc;
use std::time::Duration;

/// How long NetworkManager's signals may keep coming before the link is read again: joining a
/// network or a scan sends dozens in a row.
const SETTLE: Duration = Duration::from_millis(600);

pub fn start(app: &Rc<App>) {
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::system().await else { return };
        let Some(changes) = kit::nm::changes(&conn).await.map(kit::signals::drained) else { return };
        loop {
            let link = kit::nm::link(&conn).await;
            let Some(app) = weak.upgrade() else { return };
            if app.link.get() != link {
                app.link.set(link);
                app.changed();
            }
            drop(app);
            if changes.recv().await.is_err() {
                return;
            }
            glib::timeout_future(SETTLE).await;
            kit::signals::pending(&changes);
        }
    });
}
