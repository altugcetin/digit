use std::cell::RefCell;

thread_local! {
    static SESSION: RefCell<Option<zbus::Connection>> = const { RefCell::new(None) };
    static SYSTEM: RefCell<Option<zbus::Connection>> = const { RefCell::new(None) };
}

pub async fn system() -> zbus::Result<zbus::Connection> {
    if let Some(conn) = SYSTEM.with(|s| s.borrow().clone()) {
        return Ok(conn);
    }
    let conn = zbus::Connection::system().await?;
    SYSTEM.with(|s| *s.borrow_mut() = Some(conn.clone()));
    Ok(conn)
}

pub async fn session() -> zbus::Result<zbus::Connection> {
    if let Some(conn) = SESSION.with(|s| s.borrow().clone()) {
        return Ok(conn);
    }
    let conn = zbus::Connection::session().await?;
    SESSION.with(|s| *s.borrow_mut() = Some(conn.clone()));
    Ok(conn)
}
