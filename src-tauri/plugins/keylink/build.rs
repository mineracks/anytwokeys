fn main() {
    tauri_plugin::Builder::new(&[]).ios_path("ios").android_path("android").build();
}
