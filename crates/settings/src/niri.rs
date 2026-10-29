use kit::niri::{act, windows};
use serde_json::{Value, json};

pub fn retile(free: bool) {
    for w in windows() {
        let Some(id) = w.get("id").and_then(Value::as_u64) else { continue };
        let floating = w.get("is_floating").and_then(Value::as_bool).unwrap_or(false);
        let size = w.get("layout").and_then(|l| l.get("window_size")).and_then(Value::as_array).map(|a| (a.first().and_then(Value::as_u64).unwrap_or(0), a.get(1).and_then(Value::as_u64).unwrap_or(0))).unwrap_or((0, 0));
        if free && !floating {
            act(json!({ "MoveWindowToFloating": { "id": id } }));
        } else if !free && floating && size.0 >= 480 && size.1 >= 360 {
            act(json!({ "MoveWindowToTiling": { "id": id } }));
        }
    }
}
