use crate::{desktop, prefs, ui};
use gtk::{glib, prelude::*};
use std::cell::RefCell;
use std::time::Duration;

/// One thing search can find: a page, or a row on it.
#[derive(Clone)]
struct Hit {
    page: &'static str,
    section: String,
    title: String,
    sub: String,
    icon: String,
    keys: String,
}

thread_local! {
    static INDEX: RefCell<Option<Vec<Hit>>> = const { RefCell::new(None) };
}

/// Words for pages whose rows depend on hardware or the network, so they aren't indexed.
fn page_words(page: &str) -> &'static str {
    match page {
        "wifi" => "wireless wlan network internet hotspot password ssid",
        "bluetooth" => "pair pairing headphones headset mouse controller",
        "displays" => "monitor screen resolution refresh rate hz scale rotation arrange",
        "apps" => "default startup autostart browser mail",
        "updates" => "upgrade update security packages patch",
        "drivers" => "nvidia amd intel gpu firmware graphics",
        "overview" => "about system hardware memory disk processor version",
        "keyboard" => "shortcuts keybinds hotkeys keys",
        "compositor" => "hyprland niri compositor gaps layout tiling",
        _ => "",
    }
}

fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// The page a section-built or self-contained page gives for indexing; pages that talk to
/// hardware or the network only list their sections (Displays → Night Light).
fn source(page: &str) -> Option<gtk::Widget> {
    match page {
        "compositor" => Some(crate::compositor::build()),
        "keyboard" => Some(crate::keyboard::build()),
        _ if !desktop::sections(page).is_empty() => {
            let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let t = prefs::read();
            for section in desktop::sections(page) {
                section(&body, &t);
            }
            Some(body.upcast())
        }
        _ => None,
    }
}

fn label(w: &gtk::Widget) -> Option<String> {
    w.downcast_ref::<gtk::Label>().map(|l| l.text().to_string())
}

/// Reads a `ui::row`: its tile's icon, then its name and subtitle.
fn read_row(row: &gtk::Widget) -> (String, String, String) {
    let (mut icon, mut title, mut sub) = (String::new(), String::new(), String::new());
    let mut child = row.first_child();
    while let Some(c) = child {
        if c.has_css_class("tile") {
            icon = label(&c).unwrap_or_default();
        } else if c.is::<gtk::Box>() && title.is_empty() {
            let mut l = c.first_child();
            while let Some(x) = l {
                if x.has_css_class("name") {
                    title = label(&x).unwrap_or_default();
                } else if x.has_css_class("sub") {
                    sub = label(&x).unwrap_or_default();
                }
                l = x.next_sibling();
            }
        }
        child = c.next_sibling();
    }
    (icon, title, sub)
}

/// Walks a page in reading order, noting each heading and the rows under it.
fn walk(w: &gtk::Widget, page: &'static str, section: &mut String, out: &mut Vec<Hit>) {
    let mut child = w.first_child();
    while let Some(c) = child {
        if c.has_css_class("heading") {
            *section = label(&c).unwrap_or_default();
        } else if c.has_css_class("row") {
            let (icon, title, sub) = read_row(&c);
            if !title.is_empty() {
                out.push(Hit { page, section: section.clone(), title, sub, icon, keys: String::new() });
            }
        } else {
            walk(&c, page, section, out);
        }
        child = c.next_sibling();
    }
}

fn index() -> Vec<Hit> {
    if let Some(i) = INDEX.with(|i| i.borrow().clone()) {
        return i;
    }
    let mut out = Vec::new();
    for (page, name, icon, _) in crate::PAGES {
        out.push(Hit { page, section: String::new(), title: name.into(), sub: String::new(), icon: icon.into(), keys: page_words(page).into() });
        if let Some(w) = source(page) {
            walk(&w, page, &mut String::new(), &mut out);
        }
    }
    INDEX.with(|i| *i.borrow_mut() = Some(out.clone()));
    out
}

/// Settings change what pages show (setup rows, niri-only rows), so the index is rebuilt
/// on the next search.
pub fn forget() {
    INDEX.with(|i| *i.borrow_mut() = None);
}

fn score(hit: &Hit, words: &[String]) -> Option<i32> {
    let title = fold(&hit.title);
    let rest = fold(&format!("{} {} {} {} {}", hit.section, hit.sub, hit.keys, crate::page_name(hit.page), page_words(hit.page)));
    let mut total = 0;
    for w in words {
        total += if title.starts_with(w.as_str()) {
            300
        } else if title.split(|c: char| !c.is_alphanumeric()).any(|t| t.starts_with(w.as_str())) {
            200
        } else if title.contains(w.as_str()) {
            120
        } else if rest.split(|c: char| !c.is_alphanumeric()).any(|t| t.starts_with(w.as_str())) {
            60
        } else {
            return None;
        };
    }
    // Pages first when they match as well as a row does.
    Some(total + if hit.section.is_empty() { 20 } else { 0 })
}

fn find(query: &str) -> Vec<Hit> {
    let words: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(i32, Hit)> = index().into_iter().filter_map(|h| score(&h, &words).map(|s| (s, h))).collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0));
    // Two shortcuts can share a name; one result takes you to both.
    let mut seen = std::collections::HashSet::new();
    hits.into_iter().map(|(_, h)| h).filter(|h| seen.insert((h.page, h.section.clone(), h.title.clone()))).take(40).collect()
}

/// The results page for a query.
fn results(query: &str) -> (gtk::Widget, Option<Hit>) {
    let (scroll, body) = ui::page("Search", "");
    let hits = find(query);
    if hits.is_empty() {
        let none = ui::text(&format!("No settings match “{}”", query.trim()), "lead");
        none.set_margin_top(12);
        body.append(&none);
        return (scroll.upcast(), None);
    }
    let card = ui::card();
    let list = gtk::ListBox::new();
    list.add_css_class("results");
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_activate_on_single_click(true);
    for h in &hits {
        let page = crate::page_name(h.page);
        let place = match h.section.as_str() {
            "" => "Page".to_string(),
            s if s == page => page.to_string(),
            s => format!("{page} › {s}"),
        };
        let icon = if h.icon.is_empty() { crate::page_icon(h.page) } else { h.icon.as_str() };
        let (row, end) = ui::row(Some(icon), &h.title, &place);
        end.append(&ui::sym("chevron_right"));
        let r = gtk::ListBoxRow::new();
        r.set_child(Some(&row));
        list.append(&r);
    }
    let first = hits.first().cloned();
    list.connect_row_activated(move |_, r| {
        if let Some(h) = hits.get(r.index() as usize) {
            open(h);
        }
    });
    card.append(&list);
    body.append(&card);
    (scroll.upcast(), first)
}

fn open(h: &Hit) {
    crate::clear_search();
    if h.section.is_empty() {
        crate::show(h.page);
    } else {
        crate::show(h.page);
        reveal(h.page, Some(&h.section), &h.title);
    }
}

thread_local! {
    static FIRST: RefCell<Option<Hit>> = const { RefCell::new(None) };
}

/// Shows results for `query` in place of the current page.
pub fn update(stack: &gtk::Stack, query: &str) {
    let (page, first) = results(query);
    FIRST.with(|f| *f.borrow_mut() = first);
    if let Some(old) = stack.child_by_name("search") {
        stack.remove(&old);
    }
    stack.add_named(&page, Some("search"));
    stack.set_transition_type(gtk::StackTransitionType::None);
    stack.set_visible_child_name("search");
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
}

/// Enter in the search field opens the best result.
pub fn activate_first() {
    if let Some(h) = FIRST.with(|f| f.borrow().clone()) {
        open(&h);
    }
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Scrolls the visible page to the row titled `title` (or whose slug is `title`) and flashes it.
pub fn reveal(page: &str, section: Option<&str>, title: &str) {
    let (section, title, page) = (section.map(str::to_string), title.to_string(), page.to_string());
    // Wait a frame or two so the new page has its size.
    glib::timeout_add_local_once(Duration::from_millis(60), move || {
        let Some(scroll) = crate::page_widget(&page).and_then(|w| w.downcast::<gtk::ScrolledWindow>().ok()) else { return };
        let mut rows = Vec::new();
        collect(scroll.upcast_ref(), &mut String::new(), &mut rows);
        let want = slug(&title);
        let found = rows.iter().find(|(sec, t, _)| t == &title && section.as_ref().is_none_or(|s| s == sec)).or_else(|| rows.iter().find(|(_, t, _)| slug(t) == want));
        let Some((_, _, row)) = found else { return };
        let Some(child) = scroll.child() else { return };
        if let Some(p) = row.compute_point(&child, &gtk::graphene::Point::new(0.0, 0.0)) {
            let adj = scroll.vadjustment();
            let target = (p.y() as f64 - 96.0).clamp(adj.lower(), (adj.upper() - adj.page_size()).max(adj.lower()));
            glide(&adj, target);
        }
        row.add_css_class("found");
        let row = row.clone();
        glib::timeout_add_local_once(Duration::from_millis(1400), move || row.remove_css_class("found"));
    });
}

fn collect(w: &gtk::Widget, section: &mut String, out: &mut Vec<(String, String, gtk::Widget)>) {
    let mut child = w.first_child();
    while let Some(c) = child {
        if c.has_css_class("heading") {
            *section = label(&c).unwrap_or_default();
        } else if c.has_css_class("row") {
            out.push((section.clone(), read_row(&c).1, c.clone()));
        } else {
            collect(&c, section, out);
        }
        child = c.next_sibling();
    }
}

/// Moves the page to `target` with the settle-in curve.
fn glide(adj: &gtk::Adjustment, target: f64) {
    let from = adj.value();
    if (target - from).abs() < 1.0 {
        return;
    }
    let start = std::time::Instant::now();
    let curve = kit::motion::Curve::new(0.16, 1.0, 0.3, 1.0);
    let adj = adj.clone();
    glib::timeout_add_local(Duration::from_millis(8), move || {
        let t = (start.elapsed().as_secs_f64() / 0.42).min(1.0);
        adj.set_value(from + (target - from) * curve.at(t));
        if t >= 1.0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}
