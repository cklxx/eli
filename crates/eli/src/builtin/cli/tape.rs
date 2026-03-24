//! Tape viewer command.

/// Open the tape viewer web UI.
pub(crate) async fn tape_command(
    port: u16,
    dir: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let tapes_dir = dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".eli")
            .join("tapes")
    });
    crate::builtin::tape_viewer::serve(tapes_dir, port).await
}
