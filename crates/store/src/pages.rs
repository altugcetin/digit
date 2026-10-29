use crate::catalog::App;
use crate::{Page, Store, store};
use gtk::{gdk, gio, glib, pango, prelude::*};
use kit::ui;
use std::rc::Rc;

pub fn build(s: &Rc<Store>, page: &Page) -> gtk::Widget {
    match page {
        Page::Loading => message("progress_activity", "Loading Flathub…", ""),
        Page::Problem(text) => message("error", "Can't open that", text),
        Page::Discover => discover(s),
        Page::Yours => yours(s),
        Page::Sources => crate::sources::page(s),
        Page::Job(i) => job(s, *i),
        Page::Search(q) => search(s, q),
        Page::App(id) => app_page(s, id),
    }
}

fn scroller(body: &gtk::Box) -> gtk::Widget {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_vexpand(true);
    scroll.set_child(Some(body));
    scroll.upcast()
}

fn body() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.add_css_class("page");
    b
}

fn message(icon: &str, title: &str, text: &str) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Center);
    let t = ui::tile(icon);
    t.add_css_class("big");
    t.set_halign(gtk::Align::Center);
    b.append(&t);
    let l = ui::text(title, "title");
    l.set_halign(gtk::Align::Center);
    b.append(&l);
    if !text.is_empty() {
        let d = ui::text(text, "lead");
        d.set_wrap(true);
        d.set_max_width_chars(60);
        d.set_justify(gtk::Justification::Center);
        d.set_halign(gtk::Align::Center);
        b.append(&d);
    }
    b.set_vexpand(true);
    b.upcast()
}

fn icon(app: &App, px: i32) -> gtk::Image {
    let image = if app.icon.is_empty() { gtk::Image::from_icon_name("application-x-executable") } else { gtk::Image::from_file(&app.icon) };
    image.set_pixel_size(px);
    image.set_valign(gtk::Align::Center);
    image
}

fn action(s: &Rc<Store>, id: &str) -> gtk::Button {
    let (label, kind) = s.action(id);
    let b = ui::button(label, kind);
    b.set_sensitive(label != "Working…" && s.busy.borrow().as_ref().is_none_or(|b| b == id));
    let id = id.to_string();
    b.connect_clicked(move |_| {
        if let Some(s) = store() {
            s.press(&id);
        }
    });
    b
}

fn clickable(w: &impl IsA<gtk::Widget>, id: &str) {
    let click = gtk::GestureClick::new();
    let id = id.to_string();
    click.connect_released(move |_, _, _, _| {
        if let Some(s) = store() {
            s.open(Page::App(id.clone()));
        }
    });
    w.add_controller(click);
    w.set_cursor_from_name(Some("pointer"));
}

fn lockup(s: &Rc<Store>, app: &App) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.add_css_class("lockup");
    row.append(&icon(app, 52));
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_hexpand(true);
    col.set_valign(gtk::Align::Center);
    let n = ui::text(&app.name, "name");
    n.set_ellipsize(pango::EllipsizeMode::End);
    col.append(&n);
    let sub = ui::text(&app.summary, "sub");
    sub.set_ellipsize(pango::EllipsizeMode::End);
    col.append(&sub);
    row.append(&col);
    row.append(&action(s, &app.id));
    clickable(&row, &app.id);
    row.upcast()
}

fn flow(per_line: u32) -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_homogeneous(true);
    f.set_max_children_per_line(per_line);
    f.set_min_children_per_line(1);
    f.set_column_spacing(12);
    f.set_row_spacing(12);
    f
}

fn job_tile(i: usize, job: &crate::Job) -> gtk::Widget {
    let b = gtk::Button::new();
    b.add_css_class("jobtile");
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let t = gtk::Label::new(Some(&job.glyph));
    for c in ["sym", "tile", job.hue.as_str()] {
        t.add_css_class(c);
    }
    t.set_halign(gtk::Align::Start);
    inner.append(&t);
    inner.append(&ui::text(&job.name, "name"));
    b.set_child(Some(&inner));
    b.connect_clicked(move |_| {
        if let Some(s) = store() {
            s.open(Page::Job(i));
        }
    });
    b.upcast()
}

thread_local! {
    static BRANDS: std::cell::RefCell<(Option<gtk::CssProvider>, Vec<String>)> = const { std::cell::RefCell::new((None, Vec::new())) };
}

fn brand(w: &impl IsA<gtk::Widget>, hex: &str) {
    let ok = hex.len() == 7 && hex.starts_with('#') && hex[1..].chars().all(|c| c.is_ascii_hexdigit());
    if !ok {
        w.add_css_class("brandless");
        return;
    }
    let class = format!("b{}", &hex[1..]);
    BRANDS.with(|b| {
        let mut b = b.borrow_mut();
        if !b.1.contains(&class) {
            b.1.push(class.clone());
            let rules: String = b.1.iter().map(|c| format!(".{c} {{ background: linear-gradient(135deg, #{h} 0%, mix(#{h}, #000000, 0.45) 100%); }}\n", h = &c[1..])).collect();
            if b.0.is_none() {
                let p = gtk::CssProvider::new();
                if let Some(d) = gdk::Display::default() {
                    gtk::style_context_add_provider_for_display(&d, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
                }
                b.0 = Some(p);
            }
            if let Some(p) = &b.0 {
                p.load_from_string(&rules);
            }
        }
    });
    w.add_css_class(&class);
}

fn pick_apps<'a>(cat: &'a crate::catalog::Catalog, ids: &[String], want: usize, ok: impl Fn(&App) -> bool) -> Vec<&'a App> {
    let mut out: Vec<&App> = Vec::new();
    for id in ids {
        if let Some(a) = cat.get(id) {
            if ok(a) && !out.iter().any(|o| o.id == a.id) {
                out.push(a);
                if out.len() == want {
                    break;
                }
            }
        }
    }
    out
}

fn banner_card(s: &Rc<Store>, app: &App, eyebrow: &str) -> gtk::Widget {
    let card = gtk::Overlay::new();
    card.add_css_class("banner");
    brand(&card, &app.brand);
    card.set_overflow(gtk::Overflow::Hidden);
    card.set_size_request(-1, 300);
    let base = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    card.set_child(Some(&base));
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.set_valign(gtk::Align::Center);
    col.set_halign(gtk::Align::Start);
    col.set_margin_start(40);
    col.set_size_request(400, -1);
    let ic = icon(app, 72);
    ic.set_halign(gtk::Align::Start);
    col.append(&ic);
    col.append(&ui::text(eyebrow, "eyebrow light"));
    col.append(&ui::text(&app.name, "hero"));
    let sum = ui::text(&app.summary, "lead light");
    sum.set_wrap(true);
    sum.set_max_width_chars(42);
    col.append(&sum);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_margin_top(10);
    buttons.append(&action(s, &app.id));
    col.append(&buttons);
    card.add_overlay(&col);
    if let Some(url) = app.shots.first() {
        let pic = shot(url, 456, 256);
        pic.set_halign(gtk::Align::End);
        pic.set_valign(gtk::Align::Center);
        pic.set_margin_end(40);
        pic.add_css_class("lifted");
        card.add_overlay(&pic);
    }
    clickable(&card, &app.id);
    card.upcast()
}

fn banners(s: &Rc<Store>, apps: Vec<App>, eyebrows: Vec<String>) -> gtk::Widget {
    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    stack.set_transition_duration(420);
    let dots = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    dots.set_halign(gtk::Align::Center);
    let n = apps.len();
    for (i, app) in apps.iter().enumerate() {
        stack.add_named(&banner_card(s, app, &eyebrows[i]), Some(&format!("b{i}")));
        let dot = gtk::Button::new();
        dot.add_css_class("dot");
        if i == 0 {
            dot.add_css_class("on");
        }
        let st = stack.clone();
        let d = dots.clone();
        dot.connect_clicked(move |_| show_banner(&st, &d, i));
        dots.append(&dot);
    }
    let frame = gtk::Overlay::new();
    frame.add_css_class("carousel");
    frame.set_child(Some(&stack));
    if n > 1 {
        for (glyph, step, align) in [("chevron_left", n - 1, gtk::Align::Start), ("chevron_right", 1, gtk::Align::End)] {
            let b = gtk::Button::new();
            b.set_child(Some(&ui::sym(glyph)));
            b.add_css_class("arrow");
            b.set_halign(align);
            b.set_valign(gtk::Align::Center);
            b.set_margin_start(14);
            b.set_margin_end(14);
            let st = stack.clone();
            let d = dots.clone();
            b.connect_clicked(move |_| {
                let cur = st.visible_child_name().and_then(|x| x.trim_start_matches('b').parse::<usize>().ok()).unwrap_or(0);
                show_banner(&st, &d, (cur + step) % n);
            });
            frame.add_overlay(&b);
        }
    }
    wrap.append(&frame);
    if n > 1 {
        wrap.append(&dots);
    }
    wrap.upcast()
}

fn show_banner(stack: &gtk::Stack, dots: &gtk::Box, i: usize) {
    stack.set_visible_child_name(&format!("b{i}"));
    let mut k = 0;
    let mut c = dots.first_child();
    while let Some(w) = c {
        if k == i {
            w.add_css_class("on");
        } else {
            w.remove_css_class("on");
        }
        k += 1;
        c = w.next_sibling();
    }
}

fn spotlight(s: &Rc<Store>, app: &App) -> gtk::Widget {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("spot");
    brand(&card, &app.brand);
    card.set_size_request(340, -1);
    card.append(&ui::text("APP OF THE DAY", "eyebrow light"));
    match app.shots.first() {
        Some(url) => {
            let pic = shot(url, 292, 164);
            pic.add_css_class("lifted");
            pic.set_margin_top(14);
            pic.set_vexpand(true);
            pic.set_valign(gtk::Align::Center);
            card.append(&pic);
        }
        None => {
            let ic = icon(app, 112);
            ic.set_vexpand(true);
            ic.set_halign(gtk::Align::Center);
            card.append(&ic);
        }
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_top(16);
    row.append(&icon(app, 52));
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_hexpand(true);
    col.set_valign(gtk::Align::Center);
    let n = ui::text(&app.name, "name light");
    n.set_ellipsize(pango::EllipsizeMode::End);
    col.append(&n);
    let sum = ui::text(&app.summary, "sub light");
    sum.set_ellipsize(pango::EllipsizeMode::End);
    col.append(&sum);
    row.append(&col);
    row.append(&action(s, &app.id));
    card.append(&row);
    clickable(&card, &app.id);
    card.upcast()
}

fn tabbed(s: &Rc<Store>, cat: &crate::catalog::Catalog) -> gtk::Widget {
    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 10);
    wrap.set_hexpand(true);
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(160);
    stack.set_vhomogeneous(false);
    let lists = s.lists.borrow().clone();
    let mut updated: Vec<&App> = cat.apps.iter().filter(|a| a.verified && !a.icon.is_empty()).collect();
    updated.sort_by(|a, b| b.released.cmp(&a.released));
    let named = [("Trending", "trending"), ("Popular", "popular"), ("New", "recently-added"), ("Updated", "recently-updated")];
    let mut first = true;
    for (label, key) in named {
        let mut apps = pick_apps(cat, lists.get(key).map(Vec::as_slice).unwrap_or(&[]), 8, |a| !a.icon.is_empty());
        if apps.is_empty() && key == "recently-updated" {
            apps = updated.iter().take(8).copied().collect();
        }
        if apps.is_empty() {
            continue;
        }
        let grid = flow(2);
        grid.set_homogeneous(true);
        for a in apps {
            grid.insert(&lockup(s, a), -1);
        }
        stack.add_named(&grid, Some(key));
        let b = ui::button(label, "tab");
        if first {
            b.add_css_class("on");
            first = false;
        }
        let st = stack.clone();
        let t = tabs.clone();
        let key = key.to_string();
        b.connect_clicked(move |me| {
            st.set_visible_child_name(&key);
            let mut c = t.first_child();
            while let Some(w) = c {
                w.remove_css_class("on");
                c = w.next_sibling();
            }
            me.add_css_class("on");
        });
        tabs.append(&b);
    }
    wrap.append(&tabs);
    let card = ui::card();
    card.append(&stack);
    wrap.append(&card);
    wrap.upcast()
}

fn chip(i: usize, job: &crate::Job) -> gtk::Widget {
    let b = gtk::Button::new();
    b.add_css_class("chip");
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let t = gtk::Label::new(Some(&job.glyph));
    for c in ["sym", "tile", "small", job.hue.as_str()] {
        t.add_css_class(c);
    }
    inner.append(&t);
    inner.append(&ui::text(&job.name, "name"));
    b.set_child(Some(&inner));
    b.connect_clicked(move |_| {
        if let Some(s) = store() {
            s.open(Page::Job(i));
        }
    });
    b.upcast()
}

fn story(s: &Rc<Store>, app: &App) -> gtk::Widget {
    let card = gtk::Overlay::new();
    card.add_css_class("story");
    card.set_overflow(gtk::Overflow::Hidden);
    card.set_size_request(-1, 280);
    match app.shots.first() {
        Some(url) => {
            let pic = shot(url, 560, 280);
            pic.remove_css_class("shot");
            card.set_child(Some(&pic));
        }
        None => {
            let base = gtk::Box::new(gtk::Orientation::Vertical, 0);
            brand(&base, &app.brand);
            card.set_child(Some(&base));
        }
    }
    let scrim = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    scrim.add_css_class("scrim");
    scrim.set_valign(gtk::Align::End);
    scrim.append(&icon(app, 52));
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_hexpand(true);
    col.set_valign(gtk::Align::Center);
    col.append(&ui::text(&app.name, "name light"));
    let sum = ui::text(&app.summary, "sub light");
    sum.set_ellipsize(pango::EllipsizeMode::End);
    col.append(&sum);
    scrim.append(&col);
    scrim.append(&action(s, &app.id));
    card.add_overlay(&scrim);
    clickable(&card, &app.id);
    card.upcast()
}

fn lane(s: &Rc<Store>, title: &str, apps: Vec<&App>) -> Option<gtk::Widget> {
    if apps.len() < 3 {
        return None;
    }
    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 6);
    wrap.append(&ui::heading(title));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    for chunk in apps.chunks(3) {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        col.set_size_request(360, -1);
        for a in chunk {
            col.append(&lockup(s, a));
        }
        row.append(&col);
    }
    let sc = gtk::ScrolledWindow::new();
    sc.set_vscrollbar_policy(gtk::PolicyType::Never);
    sc.set_child(Some(&row));
    let card = ui::card();
    card.append(&sc);
    wrap.append(&card);
    Some(wrap.upcast())
}

fn discover(s: &Rc<Store>) -> gtk::Widget {
    let b = body();
    let Some(cat) = s.catalog.borrow().clone() else { return message("error", "No catalogue", "") };
    let lists = s.lists.borrow().clone();
    let get = |k: &str| lists.get(k).cloned().unwrap_or_default();
    let mut ranked: Vec<String> = get("trending");
    ranked.extend(get("popular"));
    let showy = |a: &App| !a.brand.is_empty() && !a.shots.is_empty() && !a.icon.is_empty() && a.verified;
    let mut heroes: Vec<App> = Vec::new();
    let mut eyebrows: Vec<String> = Vec::new();
    if let Some(f) = cat.get(&s.editorial.feature.id) {
        heroes.push(f.clone());
        eyebrows.push("FEATURED".into());
    }
    for a in pick_apps(&cat, &ranked, 6, showy) {
        if heroes.len() < 5 && !heroes.iter().any(|h| h.id == a.id) {
            heroes.push(a.clone());
            eyebrows.push(if get("trending").contains(&a.id) { "TRENDING".into() } else { "POPULAR".into() });
        }
    }
    if heroes.len() < 3 {
        for a in cat.apps.iter().filter(|a| showy(a)).take(5) {
            if heroes.len() < 5 && !heroes.iter().any(|h| h.id == a.id) {
                heroes.push(a.clone());
                eyebrows.push("DISCOVER".into());
            }
        }
    }
    if !heroes.is_empty() {
        b.append(&banners(s, heroes.clone(), eyebrows));
    }
    let chips = flow(6);
    chips.set_homogeneous(false);
    chips.set_margin_top(14);
    for (i, j) in s.editorial.jobs.iter().enumerate() {
        chips.insert(&chip(i, j), -1);
    }
    b.append(&ui::heading("What do you want to do?"));
    b.append(&chips);
    let day = glib::DateTime::now_local().map(|d| d.day_of_year() as usize + d.year() as usize * 7).unwrap_or(0);
    let pool: Vec<&App> = cat.apps.iter().filter(|a| showy(a) && !heroes.iter().any(|h| h.id == a.id)).collect();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.set_margin_top(18);
    if !pool.is_empty() {
        row.append(&spotlight(s, pool[day % pool.len()]));
    }
    row.append(&tabbed(s, &cat));
    b.append(&row);
    let picks: Vec<&App> = s.editorial.picks.iter().filter_map(|id| cat.get(id)).collect();
    if !picks.is_empty() {
        b.append(&ui::heading("Editor's picks"));
        let grid = flow(2);
        for a in picks {
            grid.insert(&story(s, a), -1);
        }
        b.append(&grid);
    }
    let in_cat = |a: &App, c: &[&str]| a.categories.iter().any(|x| c.contains(&x.as_str()));
    let lanes: [(&str, &[&str], &[&str], &[&str]); 4] = [
        ("Games", &["category/Game"], &["Game"], &[]),
        ("Create", &["category/Graphics", "category/AudioVideo"], &["Graphics", "AudioVideoEditing", "Photography", "Midi", "3DGraphics", "Publishing"], &["Player", "Viewer", "Game"]),
        ("Watch and listen", &["category/AudioVideo"], &["Player", "TV", "Music"], &["Game", "AudioVideoEditing"]),
        ("Work and code", &["category/Office", "category/Development"], &["Office", "Development", "IDE", "TextEditor"], &["Game"]),
    ];
    let mut fresh: Vec<&App> = cat.apps.iter().filter(|a| a.verified && !a.icon.is_empty() && !a.shots.is_empty()).collect();
    fresh.sort_by(|a, b| b.released.cmp(&a.released));
    for (title, sources, yes, no) in lanes {
        let ok = |a: &App| in_cat(a, yes) && !in_cat(a, no) && !a.icon.is_empty();
        let mut ids: Vec<String> = Vec::new();
        let mut columns: Vec<std::vec::IntoIter<String>> = sources.iter().map(|k| get(k).into_iter()).collect();
        loop {
            let mut any = false;
            for c in columns.iter_mut() {
                if let Some(id) = c.next() {
                    ids.push(id);
                    any = true;
                }
            }
            if !any {
                break;
            }
        }
        ids.extend(ranked.iter().cloned());
        let mut apps: Vec<&App> = pick_apps(&cat, &ids, 12, ok);
        if apps.len() < 12 {
            let have: Vec<String> = apps.iter().map(|g| g.id.clone()).collect();
            let room = 12 - apps.len();
            apps.extend(fresh.iter().copied().filter(|a| ok(a) && !have.contains(&a.id)).take(room));
        }
        if let Some(l) = lane(s, title, apps) {
            b.append(&l);
        }
    }
    scroller(&b)
}

fn job(s: &Rc<Store>, i: usize) -> gtk::Widget {
    let Some(job) = s.editorial.jobs.get(i).cloned() else { return message("error", "Not found", "") };
    let Some(cat) = s.catalog.borrow().clone() else { return message("error", "No catalogue", "") };
    let (scroll, b) = ui::page(&job.name, &job.advice);
    let card = ui::card();
    let mut listed: Vec<String> = Vec::new();
    for id in &job.apps {
        if let Some(app) = cat.get(id) {
            card.append(&lockup(s, app));
            listed.push(id.clone());
        }
    }
    b.append(&card);
    let mut more: Vec<&App> = Vec::new();
    for w in &job.words {
        for app in cat.search(w, 6) {
            if !listed.contains(&app.id) && !more.iter().any(|m| m.id == app.id) && more.len() < 10 {
                more.push(app);
            }
        }
    }
    if !more.is_empty() {
        b.append(&ui::heading("More to try"));
        let grid = flow(2);
        for app in more {
            grid.insert(&lockup(s, app), -1);
        }
        let c = ui::card();
        c.append(&grid);
        b.append(&c);
    }
    scroll.upcast()
}

fn search(s: &Rc<Store>, q: &str) -> gtk::Widget {
    let Some(cat) = s.catalog.borrow().clone() else { return message("error", "No catalogue", "") };
    let (scroll, b) = ui::page("Results", &format!("for “{}”", q.trim()));
    let folded = kit::fuzzy::fold(q.trim());
    let jobs = flow(6);
    let mut any_job = false;
    for (i, j) in s.editorial.jobs.iter().enumerate() {
        if folded.len() > 1 && (kit::fuzzy::fold(&j.name).contains(&folded) || j.words.iter().any(|w| w.starts_with(&folded) || folded.contains(w.as_str()))) {
            jobs.insert(&job_tile(i, j), -1);
            any_job = true;
        }
    }
    if any_job {
        b.append(&jobs);
    }
    let hits = cat.search(q, 40);
    if hits.is_empty() && !any_job {
        b.append(&message("search_off", "No apps match", "Try a word for what you want to do, like “edit photos” or “record”."));
    } else if !hits.is_empty() {
        let card = ui::card();
        for app in hits {
            card.append(&lockup(s, app));
        }
        b.append(&card);
    }
    scroll.upcast()
}

fn date(ts: u64) -> String {
    glib::DateTime::from_unix_local(ts as i64).ok().and_then(|d| d.format("%-d %b %Y").ok()).map(|s| s.to_string()).unwrap_or_default()
}

fn shot(url: &str, w: i32, h: i32) -> gtk::Widget {
    let pic = gtk::Picture::new();
    pic.set_content_fit(gtk::ContentFit::Cover);
    pic.set_size_request(w, h);
    pic.add_css_class("shot");
    pic.set_overflow(gtk::Overflow::Hidden);
    if !url.starts_with("https://dl.flathub.org/") {
        return pic.upcast();
    }
    let dir = glib::user_cache_dir().join("digit/store/shots");
    let name = glib::compute_checksum_for_string(glib::ChecksumType::Md5, url).map(|s| s.to_string()).unwrap_or_default();
    let path = dir.join(format!("{name}.img"));
    let url = url.to_string();
    let p = pic.clone();
    let sf = pic.scale_factor().max(1);
    let (dw, dh) = (w * sf, h * sf);
    glib::spawn_future_local(async move {
        if !path.exists() {
            let _ = std::fs::create_dir_all(&dir);
            let tmp = path.with_extension("part");
            let argv: Vec<&std::ffi::OsStr> = vec!["curl".as_ref(), "-fsSL".as_ref(), "--max-time".as_ref(), "25".as_ref(), "--max-filesize".as_ref(), "12000000".as_ref(), "-o".as_ref(), tmp.as_os_str(), url.as_ref()];
            let Ok(proc_) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE) else { return };
            if proc_.wait_check_future().await.is_err() || std::fs::rename(&tmp, &path).is_err() {
                let _ = std::fs::remove_file(&tmp);
                return;
            }
        }
        let decoded = gio::spawn_blocking(move || {
            let (_, iw, ih) = gtk::gdk_pixbuf::Pixbuf::file_info(&path)?;
            let k = (dw as f64 / iw.max(1) as f64).max(dh as f64 / ih.max(1) as f64).min(1.0);
            let (sw, sh) = (((iw as f64 * k).round() as i32).max(1), ((ih as f64 * k).round() as i32).max(1));
            let full = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&path, sw, sh, false).ok()?;
            let (cw, ch) = (dw.min(full.width()), dh.min(full.height()));
            let pb = full.new_subpixbuf((full.width() - cw) / 2, (full.height() - ch) / 2, cw, ch).copy()?;
            let format = if pb.has_alpha() { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
            Some((pb.width(), pb.height(), format, pb.read_pixel_bytes(), pb.rowstride() as usize))
        })
        .await
        .ok()
        .flatten();
        if let Some((w, h, format, bytes, stride)) = decoded {
            let tex = gdk::MemoryTexture::new(w, h, format, &bytes, stride);
            p.set_paintable(Some(&tex));
        }
    });
    pic.upcast()
}

fn app_page(s: &Rc<Store>, id: &str) -> gtk::Widget {
    let Some(cat) = s.catalog.borrow().clone() else { return message("error", "No catalogue", "") };
    let Some(app) = cat.get(id).cloned() else {
        return message("search_off", "Not on Flathub", &format!("{id} isn't in Flathub's catalogue."));
    };
    let b = body();
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 26);
    head.append(&icon(&app, 128));
    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    col.set_valign(gtk::Align::Center);
    col.set_hexpand(true);
    col.append(&ui::text(&app.name, "hero"));
    let by = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    by.append(&ui::text(if app.developer.is_empty() { "Unknown developer" } else { &app.developer }, "sub"));
    if app.verified {
        let v = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let tick = ui::sym("verified");
        tick.add_css_class("verified");
        v.append(&tick);
        v.append(&ui::text("Verified", "sub verified"));
        by.append(&v);
    }
    col.append(&by);
    let sum = ui::text(&app.summary, "lead");
    sum.set_wrap(true);
    col.append(&sum);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_margin_top(10);
    buttons.append(&action(s, &app.id));
    if s.installed.borrow().contains_key(&app.id) && s.busy.borrow().is_none() {
        let rm = ui::button("Remove", "danger");
        let id = app.id.clone();
        rm.connect_clicked(move |_| {
            if let Some(s) = store() {
                s.remove(&id);
            }
        });
        buttons.append(&rm);
    }
    col.append(&buttons);
    head.append(&col);
    b.append(&head);
    if !app.shots.is_empty() {
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        for url in &app.shots {
            strip.append(&shot(url, 480, 270));
        }
        let sc = gtk::ScrolledWindow::new();
        sc.set_vscrollbar_policy(gtk::PolicyType::Never);
        sc.set_child(Some(&strip));
        sc.set_margin_top(18);
        b.append(&sc);
    }
    if !app.about.is_empty() {
        b.append(&ui::heading("About"));
        let card = ui::card();
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 10);
        inner.set_margin_top(12);
        inner.set_margin_bottom(12);
        inner.set_margin_start(16);
        inner.set_margin_end(16);
        for p in &app.about {
            let l = ui::text(p, "about");
            l.set_wrap(true);
            l.set_selectable(true);
            inner.append(&l);
        }
        card.append(&inner);
        b.append(&card);
    }
    b.append(&ui::heading("Information"));
    let info = ui::card();
    if !app.version.is_empty() {
        let when = if app.released > 0 { format!(" · {}", date(app.released)) } else { String::new() };
        info.append(&ui::row(Some("new_releases"), "Version", &format!("{}{when}", app.version)).0);
    }
    if !app.license.is_empty() {
        info.append(&ui::row(Some("gavel"), "License", &app.license.replace(" AND ", ", ")).0);
    }
    if !app.categories.is_empty() {
        info.append(&ui::row(Some("category"), "Category", &app.categories.iter().filter(|c| !c.starts_with("X-")).take(3).cloned().collect::<Vec<_>>().join(", ")).0);
    }
    if let Some(inst) = s.installed.borrow().get(&app.id) {
        info.append(&ui::row(Some("check_circle"), "Installed", &format!("{} · {}", if inst.version.is_empty() { "yes" } else { &inst.version }, if inst.system { "for everyone" } else { "for you" })).0);
    }
    if app.homepage.starts_with("https://") || app.homepage.starts_with("http://") {
        let (row, end) = ui::row(Some("language"), "Website", &app.homepage);
        let open = ui::button("Open", "");
        let url = app.homepage.clone();
        open.connect_clicked(move |_| {
            let _ = gio::AppInfo::launch_default_for_uri(&url, gio::AppLaunchContext::NONE);
        });
        end.append(&open);
        info.append(&row);
    }
    info.append(&ui::row(Some("deployed_code"), "App ID", &app.id).0);
    b.append(&info);
    b.append(&ui::heading("Get it from"));
    let from = ui::card();
    crate::sources::app_sources(s, &app.id, &app.name, &from);
    b.append(&from);
    scroller(&b)
}

fn yours(s: &Rc<Store>) -> gtk::Widget {
    let (scroll, b) = ui::page("Yours", "Apps you got from Flatpak. Apps installed for you need no password.");
    let cat = s.catalog.borrow().clone();
    let installed = s.installed.borrow().clone();
    let updates = s.updates.borrow().clone();
    let entry = |id: &str| -> App {
        cat.as_ref().and_then(|c| c.get(id).cloned()).unwrap_or_else(|| App { id: id.to_string(), name: installed.get(id).map(|i| i.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| id.to_string()), summary: installed.get(id).map(|i| i.version.clone()).unwrap_or_default(), ..Default::default() })
    };
    let mut ids: Vec<&String> = installed.keys().collect();
    ids.sort_by_key(|id| entry(id).name.to_lowercase());
    let pending: Vec<&String> = ids.iter().copied().filter(|id| updates.contains(*id)).collect();
    if !pending.is_empty() {
        b.append(&ui::heading(&format!("Updates · {}", pending.len())));
        let card = ui::card();
        for id in &pending {
            card.append(&lockup(s, &entry(id)));
        }
        b.append(&card);
    }
    b.append(&ui::heading(&format!("Installed · {}", ids.len())));
    let card = ui::card();
    if ids.is_empty() {
        card.append(&ui::row(Some("apps"), "Nothing yet", "Apps you get from the store show up here").0);
    }
    for id in ids.iter().filter(|id| !updates.contains(**id)) {
        card.append(&lockup(s, &entry(id)));
    }
    b.append(&card);
    scroll.upcast()
}
