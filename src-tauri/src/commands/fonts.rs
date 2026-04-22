use std::collections::BTreeSet;

/// Return a sorted, deduplicated list of font family names installed on the system.
#[tauri::command]
pub fn get_system_fonts() -> Vec<String> {
    let source = font_kit::source::SystemSource::new();
    let mut families = BTreeSet::new();
    if let Ok(all) = source.all_families() {
        for name in all {
            if !name.starts_with('.') {
                families.insert(name);
            }
        }
    }
    families.into_iter().collect()
}
