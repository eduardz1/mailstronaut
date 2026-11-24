//! Explicit attachment actions for the current original.
use super::*;

impl Workspace {
    pub(super) fn attachment_actions(
        index: usize,
        remote: bool,
        name: &str,
        cx: &Context<Self>,
    ) -> AttachmentActions {
        let (open_id, save_id) = if remote {
            ("open-remote-attachment", "save-remote-attachment")
        } else {
            ("open-cached-attachment", "save-cached-attachment")
        };
        AttachmentActions::new()
            .child(
                Button::new((save_id, index))
                    .ghost()
                    .label("Save…")
                    .tooltip("Save a copy")
                    .accessibility_label(format!("Save {name}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if remote {
                            this.save_remote_attachment(index, window, cx);
                        } else {
                            this.save_attachment(index, window, cx);
                        }
                    })),
            )
            .child(
                Button::new((open_id, index))
                    .ghost()
                    .label("Open…")
                    .tooltip("Choose an application")
                    .accessibility_label(format!("Open {name}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_attachment(index, remote, window, cx);
                    })),
            )
    }

    fn open_attachment(
        &mut self,
        index: usize,
        remote: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.preview_cancel.is_some() || self.view != View::Inbox {
            return;
        }
        let cancel = self.account_lifetime.child_token();
        let previews = self.previews.clone();
        let job = if remote {
            let (Some(row), Some(part), Some(store)) = (
                self.selected.as_ref(),
                self.opened_omitted
                    .as_ref()
                    .and_then(|omitted| omitted.attachments.get(index)),
                self.handle(),
            ) else {
                return;
            };
            let Some(account) = self
                .declaration
                .active()
                .and_then(|config| {
                    config
                        .accounts
                        .iter()
                        .find(|account| account.id == row.account)
                })
                .cloned()
            else {
                return;
            };
            let (access, id, section, token) = (
                self.account_access.clone(),
                row.id,
                part.section.clone(),
                cancel.clone(),
            );
            self.runtime.spawn(async move {
                let bytes = access
                    .fetch_attachment(&store, &account, id, &section, &token)
                    .await?;
                previews.open(bytes, &token).await.map_err(str::to_owned)
            })
        } else {
            let Some(message) = self.opened_message.clone() else {
                return;
            };
            let token = cancel.clone();
            self.runtime.spawn(async move {
                let bytes = tokio::task::spawn_blocking(move || message.attachment_bytes(index))
                    .await
                    .map_err(|_| "Opening the attachment was interrupted.")?
                    .map_err(|_| "This attachment could not be decoded.")?;
                previews.open(bytes, &token).await.map_err(str::to_owned)
            })
        };
        let epoch = self.reading_generation;
        let revision = self.declaration.revision();
        self.preview_cancel = Some(cancel.clone());
        self.reader_notice = "Preparing the attachment for the application chooser…".into();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                if cancel.is_cancelled()
                    || this.reading_generation != epoch
                    || this.declaration.revision() != revision
                {
                    return;
                }
                this.preview_cancel = None;
                this.reader_notice = match result {
                    Ok(Ok(())) => "Attachment sent to the selected application.".into(),
                    Ok(Err(error)) => error,
                    Err(_) => "Opening the attachment was interrupted.".into(),
                };
                cx.notify();
            });
        })
        .detach();
    }
}
