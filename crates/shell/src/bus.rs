use async_lock::OnceCell;

static SESSION: OnceCell<zbus::Connection> = OnceCell::new();
static SYSTEM: OnceCell<zbus::Connection> = OnceCell::new();

pub async fn system() -> zbus::Result<zbus::Connection> {
    SYSTEM.get_or_try_init(zbus::Connection::system).await.cloned()
}

pub async fn session() -> zbus::Result<zbus::Connection> {
    SESSION.get_or_try_init(zbus::Connection::session).await.cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn concurrent_callers_share_pending_initialization() {
        let cell = OnceCell::new();
        let calls = AtomicUsize::new(0);
        let (tx, rx) = async_channel::bounded::<()>(1);
        zbus::block_on(async {
            let init = || async {
                calls.fetch_add(1, Ordering::Relaxed);
                rx.recv().await.unwrap();
                Ok::<_, ()>(42)
            };
            let (a, b, _) = futures_util::join!(cell.get_or_try_init(init), cell.get_or_try_init(init), async {
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                tx.send(()).await.unwrap();
            });
            assert!(std::ptr::eq(a.unwrap(), b.unwrap()));
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        });
    }
}
