//! Read-only file handoff through the desktop application chooser.
use std::{
    fs::File, io::Write as _, os::unix::fs::PermissionsExt as _, path::Path, sync::Arc,
    time::Duration,
};

use tempfile::TempPath;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const UNAVAILABLE: &str = "Private attachment storage is unavailable. You can still save a copy.";
const INTERRUPTED: &str = "Opening the attachment was interrupted.";
const MAX_BYTES: usize = 128 * 1024 * 1024;

struct Preview {
    digest: [u8; 32],
    path: TempPath,
    size: usize,
}

/// A successful launch may read the file after the portal replies. Keep private copies
/// until the application exits, including while its last window is closed.
#[derive(Clone, Default)]
pub struct Previews(Arc<Mutex<Vec<Preview>>>);

fn stage(directory: &Path, bytes: &[u8]) -> std::io::Result<TempPath> {
    let mut file = tempfile::Builder::new()
        .prefix("mailstronaut-attachment-")
        .tempfile_in(directory)?;
    file.write_all(bytes)?;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o400))?;
    Ok(file.into_temp_path())
}

impl Previews {
    pub async fn open(
        &self,
        bytes: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<(), &'static str> {
        cancel
            .run_until_cancelled(self.choose(bytes))
            .await
            .ok_or(INTERRUPTED)?
    }

    async fn choose(&self, bytes: Vec<u8>) -> Result<(), &'static str> {
        let size = bytes.len();
        let (digest, bytes) = tokio::task::spawn_blocking(move || {
            let digest = mailstronaut_core::mime::content_digest(&bytes);
            (digest, bytes)
        })
        .await
        .map_err(|_| INTERRUPTED)?;
        let mut previews = self.0.lock().await;
        let staged;
        let path = if let Some(previous) = previews.iter().find(|entry| entry.digest == digest) {
            staged = None;
            &previous.path
        } else {
            if previews.len() >= 64
                || previews.iter().map(|entry| entry.size).sum::<usize>() + size > MAX_BYTES
            {
                return Err("The attachment preview limit was reached. Save a copy to open it.");
            }
            let base = directories::BaseDirs::new().ok_or(UNAVAILABLE)?;
            let directory = base.runtime_dir().ok_or(UNAVAILABLE)?.to_owned();
            staged = Some(
                tokio::task::spawn_blocking(move || stage(&directory, &bytes))
                    .await
                    .map_err(|_| INTERRUPTED)?
                    .map_err(|_| UNAVAILABLE)?,
            );
            staged.as_ref().ok_or(UNAVAILABLE)?
        };
        let file = File::open(path).map_err(|_| UNAVAILABLE)?;
        let request = tokio::time::timeout(
            Duration::from_secs(60),
            ashpd::desktop::open_uri::OpenFileRequest::default()
                .ask(true)
                .send_file(&file),
        )
        .await
        .map_err(|_| "The application chooser timed out.")?
        .map_err(|_| "The application chooser is unavailable or refused the request.")?;
        request
            .response()
            .map_err(|_| "Opening the attachment was cancelled or refused.")?;
        if let Some(path) = staged {
            previews.push(Preview { digest, path, size });
        }
        drop(previews);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_only_the_bytes_and_unlinks_on_drop() {
        let directory = tempfile::tempdir().expect("runtime directory");
        let path = stage(directory.path(), b"synthetic attachment").expect("stage");
        assert_eq!(
            std::fs::read(&path).expect("private fixture"),
            b"synthetic attachment"
        );
        assert_eq!(
            std::fs::metadata(&path)
                .expect("private fixture")
                .permissions()
                .mode()
                & 0o777,
            0o400
        );
        let copy = path.to_path_buf();
        drop(path);
        assert!(!copy.exists());
    }
}
