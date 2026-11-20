//! Remembering where each icon was put, by screen, in the state folder.

use super::*;

#[cfg(test)]
pub(super) fn encode(positions: &Positions) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec_pretty(positions)
}
pub(super) fn decode(bytes: &[u8]) -> Result<Positions, serde_json::Error> {
    let entries: BTreeMap<String, serde_json::Value> = serde_json::from_slice(bytes)?;
    Ok(entries
        .into_iter()
        .filter_map(|(name, value)| {
            // Old col/row entries (and malformed entries) have no free position to restore.
            let p: FreePos = serde_json::from_value(value).ok()?;
            (p.x.is_finite() && p.y.is_finite()).then_some((name, p))
        })
        .collect())
}
pub(super) type OutputNames = BTreeMap<String, String>;
pub(super) fn decode_state(bytes: &[u8]) -> Result<(Positions, OutputNames), serde_json::Error> {
    let positions = decode(bytes)?;
    let entries: BTreeMap<String, serde_json::Value> = serde_json::from_slice(bytes)?;
    let outputs = entries
        .into_iter()
        .filter_map(|(name, value)| Some((name, value.get("output")?.as_str()?.to_owned())))
        .collect();
    Ok((positions, outputs))
}
pub(super) fn encode_state(
    positions: &Positions,
    outputs: &OutputNames,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut entries = serde_json::to_value(positions)?;
    for (name, output) in outputs {
        if let Some(entry) = entries
            .get_mut(name)
            .and_then(serde_json::Value::as_object_mut)
        {
            entry.insert("output".into(), output.clone().into());
        }
    }
    serde_json::to_vec_pretty(&entries)
}
pub(super) fn write_state(path: &Path, positions: &Positions, outputs: &OutputNames) -> Result<(), String> {
    write_bytes(
        path,
        &encode_state(positions, outputs).map_err(|e| e.to_string())?,
    )
}
pub(super) fn state_path() -> PathBuf {
    kit::theme::state_dir().join("desktop-icons.json")
}
#[cfg(test)]
pub(super) fn read_positions(path: &Path) -> Result<Positions, String> {
    // Missing, unreadable or corrupt state always starts from the default layout.
    Ok(std::fs::read(path)
        .ok()
        .and_then(|bytes| decode(&bytes).ok())
        .unwrap_or_default())
}
#[cfg(test)]
pub(super) fn write_positions(path: &Path, positions: &Positions) -> Result<(), String> {
    write_bytes(path, &encode(positions).map_err(|e| e.to_string())?)
}
pub(super) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    kit::atomic::write(path, bytes).map_err(|e| e.to_string())
}
