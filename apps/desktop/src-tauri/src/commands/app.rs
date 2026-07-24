use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub shell: String,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: "Minos".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        shell: "tauri".into(),
    }
}
