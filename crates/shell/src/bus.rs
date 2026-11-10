use async_lock::Mutex;

static SESSION: Mutex<Option<zbus::Connection>> = Mutex::new(None);
static SYSTEM: Mutex<Option<zbus::Connection>> = Mutex::new(None);

pub async fn system() -> zbus::Result<zbus::Connection> {
    cached(&SYSTEM, |c| !c.is_closed(), zbus::Connection::system).await
}

pub async fn session() -> zbus::Result<zbus::Connection> {
    cached(&SESSION, |c| !c.is_closed(), zbus::Connection::session).await
}

async fn cached<T: Clone, E, F, Fut>(cache: &Mutex<Option<T>>, valid: impl Fn(&T) -> bool, connect: F) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut slot = cache.lock().await;
    if slot.as_ref().is_none_or(|c| !valid(c)) {
        *slot = None;
        *slot = Some(connect().await?);
    }
    Ok(slot.as_ref().unwrap().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn concurrent_callers_share_initialization_and_replace_invalid_values() {
        let cache = Mutex::new(None);
        let calls = AtomicUsize::new(0);
        let (tx, rx) = async_channel::bounded::<()>(1);
        zbus::block_on(async {
            let init = || async {
                calls.fetch_add(1, Ordering::Relaxed);
                rx.recv().await.unwrap();
                Ok::<_, ()>(42)
            };
            let (a, b, _) = futures_util::join!(cached(&cache, |_| true, init), cached(&cache, |_| true, init), async {
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                tx.send(()).await.unwrap();
            });
            assert_eq!((a, b), (Ok(42), Ok(42)));
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert_eq!(cached(&cache, |_| false, || async { Ok::<_, ()>(43) }).await, Ok(43));
            assert_eq!(cached(&cache, |_| false, || async { Err::<i32, _>(()) }).await, Err(()));
            assert!(cache.lock().await.is_none());
        });
    }
}
