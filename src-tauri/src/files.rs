//! Choosing and saving files. On a computer this is the system's own open and save dialogs. A phone has no such
//! dialogs to call from here: what the app saves goes into its own Documents folder, which the Files app shows under
//! "On My iPhone › Any Two Keys", and choosing a file to open is not offered there yet.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub use rfd::FileDialog;

#[cfg(any(target_os = "ios", target_os = "android"))]
pub struct FileDialog { name: Option<String> }
#[cfg(any(target_os = "ios", target_os = "android"))]
impl FileDialog {
    pub fn new() -> Self { FileDialog { name: None } }
    pub fn set_title(self, _title: impl Into<String>) -> Self { self }
    pub fn add_filter(self, _name: impl Into<String>, _extensions: &[impl ToString]) -> Self { self }
    pub fn set_file_name(mut self, name: impl Into<String>) -> Self { self.name = Some(name.into()); self }
    fn documents() -> Option<std::path::PathBuf> { let d = crate::profiles::phone_documents()?; std::fs::create_dir_all(&d).ok()?; Some(d) }
    pub fn pick_file(self) -> Option<std::path::PathBuf> { None }
    pub fn pick_files(self) -> Option<Vec<std::path::PathBuf>> { None }
    pub fn pick_folder(self) -> Option<std::path::PathBuf> { Self::documents() }
    pub fn save_file(self) -> Option<std::path::PathBuf> { Some(Self::documents()?.join(self.name.unwrap_or_else(|| "anytwokeys-export".into()))) }
}
