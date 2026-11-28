use futures_util::{Stream, StreamExt};

pub fn drained<S>(stream: S) -> async_channel::Receiver<S::Item>
where
    S: Stream + 'static,
    S::Item: 'static,
{
    let (tx, rx) = async_channel::unbounded();
    gtk::glib::spawn_future_local(async move {
        let mut stream = Box::pin(stream);
        while let Some(item) = stream.next().await {
            if tx.send(item).await.is_err() {
                break;
            }
        }
    });
    rx
}

pub fn pending<T>(rx: &async_channel::Receiver<T>) {
    while rx.try_recv().is_ok() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slow_reader_never_holds_up_the_stream() {
        let ctx = gtk::glib::MainContext::new();
        ctx.block_on(async {
            let (tx, source) = async_channel::bounded::<u32>(2);
            let rx = drained(source);
            for i in 0..100 {
                tx.send(i).await.unwrap();
            }
            drop(tx);
            let mut got = Vec::new();
            while let Ok(i) = rx.recv().await {
                got.push(i);
            }
            assert_eq!(got, (0..100).collect::<Vec<_>>());
        });
    }

    #[test]
    fn dropping_the_reader_lets_the_stream_go() {
        let ctx = gtk::glib::MainContext::new();
        ctx.block_on(async {
            let (tx, source) = async_channel::bounded::<u32>(1);
            drop(drained(source));
            gtk::glib::timeout_future(std::time::Duration::from_millis(10)).await;
            tx.send(1).await.ok();
            gtk::glib::timeout_future(std::time::Duration::from_millis(10)).await;
            assert!(tx.is_closed() || tx.send(2).await.is_err());
        });
    }
}
