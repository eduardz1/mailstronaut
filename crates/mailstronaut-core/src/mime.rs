//! Bounded, untrusted MIME projections. HTML requires sanitization before rendering.

use core::ops::Range;

use mail_parser::{AddressList, MessageParser, MessagePart, PartFlags, PartKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Content identity for a complete MIME source or an explicitly extracted attachment.
#[must_use]
pub fn content_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Maximum accepted original message size.
pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
/// Maximum parsed parts, including nested messages.
pub const MAX_PARTS: usize = 256;
/// Maximum retained attachment descriptors.
pub const MAX_ATTACHMENTS: usize = 64;
/// Maximum decoded bytes retained for each body alternative.
pub const MAX_BODY_BYTES: usize = 256 * 1024;
/// Maximum retained bytes in an individual decoded header field.
pub const MAX_HEADER_BYTES: usize = 4096;
/// Maximum retained addresses or ancestry identifiers in one header list.
pub const MAX_HEADER_VALUES: usize = 256;

/// Unsupported inline encryption marker, never a request to decrypt content.
const INLINE_PGP_HEADER: &str = "-----BEGIN PGP MESSAGE-----";

/// A decoded mailbox; group names are not retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Mailbox {
    /// Address, if parsed.
    pub address: Option<String>,
    /// Display name.
    pub name: Option<String>,
}

/// Decoded root-message headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Headers {
    /// Blind-copy recipients if present in the source.
    pub bcc: Vec<Mailbox>,
    /// Copy recipients.
    pub cc: Vec<Mailbox>,
    /// Parsed UTC timestamp in seconds.
    pub date: Option<i64>,
    /// Sender mailboxes.
    pub from: Vec<Mailbox>,
    /// Explicit reply routing, defaulting empty for older cached projections.
    #[serde(default)]
    pub reply_to: Vec<Mailbox>,
    /// Reply identifiers in order.
    pub in_reply_to: Vec<String>,
    /// Message identifier without angle brackets.
    pub message_id: Option<String>,
    /// Reference identifiers in order.
    pub references: Vec<String>,
    /// Decoded subject.
    pub subject: Option<String>,
    /// Primary recipients.
    pub to: Vec<Mailbox>,
}

/// A descriptor into this projection's original raw message, never decoded attachment data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attachment {
    /// Content identifier, scoped to this message.
    pub content_id: Option<String>,
    /// Transfer-decoded size.
    pub decoded_size: usize,
    /// Inline disposition.
    pub inline: bool,
    /// Content type, if present.
    pub media_type: Option<String>,
    /// Filename, if present.
    pub name: Option<String>,
    /// Raw encoded body range in `Projection::original`.
    pub raw_body: Range<usize>,
}

/// Transfer-decoded local PNG/JPEG bytes; pixel validation belongs to the renderer.
/// No filename or sender-provided path is used to read this resource.
#[derive(Clone)]
pub struct InlineRaster {
    /// Exact, message-local MIME Content-ID.
    pub content_id: String,
    /// Declared image/png or image/jpeg type, still requiring a signature check.
    pub media_type: String,
    /// Compressed raster bytes after MIME transfer decoding.
    pub bytes: Vec<u8>,
}

/// Origin of preview text, retained separately from the body revision.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreviewSource {
    /// Deterministically extracted from the original message.
    Extracted,
    /// Reserved provenance for a future generated preview.
    Generated {
        /// Identifier of the model used to generate the text.
        model_revision: String,
    },
}

/// Deterministic source extraction, distinct from any future generated summary.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Preview {
    /// Hash of the complete source message.
    pub body_revision: [u8; 32],
    /// Provenance of the preview text.
    pub source: PreviewSource,
    /// Whitespace-normalized, at most 320 Unicode characters.
    pub text: String,
}

/// Degradation and resource-limit indicators.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent parse conditions may occur together."
)]
pub struct Status {
    /// Header fields exceeded their retained size/count budget.
    #[serde(default)]
    pub headers_truncated: bool,
    /// Attachment descriptors exceeded their limit.
    pub attachments_truncated: bool,
    /// Oversized filename, media type or Content-ID omitted from a retained descriptor.
    #[serde(default)]
    pub attachment_metadata_truncated: bool,
    /// A body alternative exceeded its limit.
    pub body_truncated: bool,
    /// Malformed structure or decoded body.
    pub degraded: bool,
    /// An encrypted MIME part was detected; body extraction is disabled.
    pub encrypted_unsupported: bool,
    /// Parser part or depth limit was reached.
    pub parts_truncated: bool,
}

/// An owned projection of one message. Neither HTML nor attachment contents are trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Projection {
    /// Root-message attachments, in source order.
    pub attachments: Vec<Attachment>,
    /// Root headers.
    pub headers: Headers,
    /// First HTML alternative, if any; unsanitized.
    pub html: Option<String>,
    /// Original bytes, bounded by `MAX_INPUT_BYTES`.
    pub original: Vec<u8>,
    /// Extracted preview, absent for encrypted mail.
    pub preview: Option<Preview>,
    /// SHA-256 of the original, also used as body revision.
    pub revision: [u8; 32],
    /// Parsing and truncation state.
    pub status: Status,
    /// First plain-text alternative, if any.
    pub text: Option<String>,
}

impl Projection {
    /// Decode unambiguous root-message raster parts using the MIME library.
    ///
    /// This is blocking work for the message/render worker. It reparses the
    /// bounded original with the same parser limits and checks its revision.
    /// Non-raster attachments stay unavailable to HTML; image dimensions and
    /// decoded pixel budgets must additionally be checked by the renderer.
    ///
    /// # Errors
    /// Rejects changed/truncated originals, ambiguous IDs, bad transfer encoding,
    /// parts above 4 MiB or aggregate compressed raster data above 16 MiB.
    pub fn inline_rasters(&self) -> Result<Vec<InlineRaster>, Error> {
        if self.status.encrypted_unsupported {
            return Ok(Vec::new());
        }
        if self.original.len() > MAX_INPUT_BYTES
            || self.status.parts_truncated
            || self.status.attachments_truncated
            || content_digest(&self.original) != self.revision
        {
            return Err(Error::InlineResources);
        }
        let message = MessageParser::new()
            .max_parts(MAX_PARTS)
            .max_depth(32)
            .max_encoded_nesting(0)
            .parse(&self.original)
            .ok_or(Error::Unparseable)?;
        let mut images = Vec::new();
        let mut total = 0_usize;
        for part in message.attachments() {
            let Some(cid) = part.content_id() else {
                continue;
            };
            if cid.len() > MAX_HEADER_BYTES {
                continue;
            }
            let media_type = if part.is_content_type("image", "png") {
                "image/png"
            } else if part.is_content_type("image", "jpeg") {
                "image/jpeg"
            } else {
                continue;
            };
            let descriptor = self.attachment_for_cid(cid).ok_or(Error::InlineResources)?;
            if part.decoded_len() > 4 * 1024 * 1024
                || !descriptor
                    .media_type
                    .as_deref()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case(media_type))
                || usize::try_from(part.offset_body()).ok() != Some(descriptor.raw_body.start)
                || usize::try_from(part.offset_end()).ok() != Some(descriptor.raw_body.end)
            {
                return Err(Error::InlineResources);
            }
            total = total
                .checked_add(part.decoded_len())
                .filter(|n| *n <= 16 * 1024 * 1024)
                .ok_or(Error::InlineResources)?;
            let (bytes, problems) = part.decoded_checked();
            if !problems.is_empty() || bytes.len() != part.decoded_len() {
                return Err(Error::InlineResources);
            }
            images.push(InlineRaster {
                content_id: cid.into(),
                media_type: media_type.into(),
                bytes: bytes.into_owned(),
            });
        }
        Ok(images)
    }

    /// Attachments a person can save separately. Only local raster types supported
    /// by the HTML reader can be omitted as inline body images.
    #[must_use]
    pub fn listed_attachments(&self) -> Vec<(usize, &Attachment)> {
        self.attachments
            .iter()
            .enumerate()
            .filter(|(_, part)| {
                let raster = part.media_type.as_deref().is_some_and(|kind| {
                    kind.eq_ignore_ascii_case("image/png")
                        || kind.eq_ignore_ascii_case("image/jpeg")
                });
                !(part.inline && part.content_id.is_some() && raster)
            })
            .collect()
    }

    /// Decode one attachment by its index in [`Self::attachments`], for an explicit
    /// save. This is blocking work; the bytes are untrusted and are never opened here.
    ///
    /// # Errors
    /// Rejects a changed original, an unknown index, a part that no longer matches its
    /// descriptor and invalid transfer encoding.
    pub fn attachment_bytes(&self, index: usize) -> Result<Vec<u8>, Error> {
        let descriptor = self.attachments.get(index).ok_or(Error::InlineResources)?;
        if self.original.len() > MAX_INPUT_BYTES || content_digest(&self.original) != self.revision
        {
            return Err(Error::InlineResources);
        }
        let message = MessageParser::new()
            .max_parts(MAX_PARTS)
            .max_depth(32)
            .max_encoded_nesting(0)
            .parse(&self.original)
            .ok_or(Error::Unparseable)?;
        let part = message
            .attachments()
            .nth(index)
            .ok_or(Error::InlineResources)?;
        if usize::try_from(part.offset_body()).ok() != Some(descriptor.raw_body.start)
            || usize::try_from(part.offset_end()).ok() != Some(descriptor.raw_body.end)
        {
            return Err(Error::InlineResources);
        }
        let (bytes, problems) = part.decoded_checked();
        if !problems.is_empty() {
            return Err(Error::InlineResources);
        }
        Ok(bytes.into_owned())
    }

    /// Native text-reader content. HTML is converted to inert Unicode text by
    /// the existing MIME library; no browser or remote resource is invoked.
    #[must_use]
    pub fn readable_text(&self) -> Option<std::borrow::Cow<'_, str>> {
        self.text
            .as_deref()
            .map(std::borrow::Cow::Borrowed)
            .or_else(|| {
                self.html
                    .as_deref()
                    .map(|html| std::borrow::Cow::Owned(mail_parser::html_to_text(html)))
            })
    }

    /// Resolve a CID only when exactly one root-message attachment has that ID.
    #[must_use = "CID lookup has no effect unless its result is used."]
    #[inline]
    pub fn attachment_for_cid(&self, cid: &str) -> Option<&Attachment> {
        if !valid_cid(cid) {
            return None;
        }
        let mut matches = self
            .attachments
            .iter()
            .filter(|part| part.content_id.as_deref() == Some(cid));
        let first = matches.next()?;
        if matches.next().is_some()
            || self.status.attachments_truncated
            || self.status.parts_truncated
        {
            return None;
        }
        Some(first)
    }
}

/// Input cannot be projected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Inline rasters are ambiguous, changed, malformed or beyond their budgets.
    InlineResources,
    /// Original exceeds the hard input limit.
    InputTooLarge,
    /// No message could be parsed.
    Unparseable,
}

/// Flatten parsed address groups.
fn addresses(list: Option<AddressList<'_>>, status: &mut Status) -> Vec<Mailbox> {
    let mut result = Vec::new();
    for mailbox in list.into_iter().flat_map(|items| items.mailboxes()) {
        if result.len() == MAX_HEADER_VALUES {
            status.headers_truncated = true;
            break;
        }
        result.push(Mailbox {
            name: header(mailbox.name(), &mut status.headers_truncated, true),
            // Never invent a different routing address by truncating it.
            address: header(mailbox.address(), &mut status.headers_truncated, false),
        });
    }
    result
}

/// Bound retained headers without altering a routing/identity field's meaning.
fn header(value: Option<&str>, truncated: &mut bool, display: bool) -> Option<String> {
    let value = value?;
    if value.len() <= MAX_HEADER_BYTES {
        return Some(value.to_owned());
    }
    *truncated = true;
    if !display {
        return None;
    }
    let mut end = MAX_HEADER_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    Some(value[..end].to_owned())
}

fn identifiers<'a>(values: impl Iterator<Item = &'a str>, status: &mut Status) -> Vec<String> {
    let mut result = Vec::new();
    for value in values {
        if result.len() == MAX_HEADER_VALUES {
            status.headers_truncated = true;
            break;
        }
        if let Some(value) = header(Some(value), &mut status.headers_truncated, false) {
            result.push(value);
        }
    }
    result
}

/// Accept only a conservative, unambiguous Content-ID lookup key.
fn valid_cid(cid: &str) -> bool {
    !cid.is_empty()
        && cid.len() <= MAX_HEADER_BYTES
        && cid
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'<' | b'>'))
}

/// Keep descriptors addressable by original part index even when their metadata is omitted.
fn attachment(part: MessagePart<'_>, truncated: &mut bool) -> Attachment {
    let name = header(part.attachment_name(), truncated, false);
    let content_id = header(part.content_id(), truncated, false).filter(|cid| valid_cid(cid));
    let media_type = part.content_type().and_then(|kind| {
        let top = kind.ctype();
        let subtype = kind.subtype().unwrap_or("");
        if top.len() + subtype.len() + 1 > MAX_HEADER_BYTES {
            *truncated = true;
            return None;
        }
        Some(format!("{top}/{subtype}"))
    });
    Attachment {
        name,
        content_id,
        media_type,
        raw_body: usize::try_from(part.offset_body()).unwrap_or_default()
            ..usize::try_from(part.offset_end()).unwrap_or_default(),
        decoded_size: part.decoded_len(),
        inline: part.is_inline(),
    }
}

/// Record parser flags and encrypted containers recursively.
fn inspect(part: MessagePart<'_>, status: &mut Status) {
    status.degraded |= part.has_problems();
    status.parts_truncated |= part.flags().contains(PartFlags::LIMIT_REACHED);
    status.encrypted_unsupported |= part.is_content_type("multipart", "encrypted")
        || part.is_content_type("application", "pkcs7-mime")
        || part.is_content_type("application", "x-pkcs7-mime")
        || part.is_content_type("application", "pgp-encrypted");
    // Inspect all text that could be retained, plus marker-length overlap at its
    // boundary. PartKind includes text/plain implied by a missing Content-Type.
    if matches!(part.kind(), PartKind::Text)
        && part
            .text_prefix_bytes(MAX_BODY_BYTES.saturating_add(INLINE_PGP_HEADER.len()))
            .is_some_and(|text| text.contains(INLINE_PGP_HEADER))
    {
        status.encrypted_unsupported = true;
    }
    for child in part.children() {
        inspect(child, status);
    }
    if let Some(nested) = part.nested() {
        inspect(nested.root_part(), status);
    }
}

/// Decode one bounded text alternative.
fn body(candidate: Option<MessagePart<'_>>, status: &mut Status) -> Option<String> {
    let part = candidate?;
    // Limit work before decoding. Charset conversion can expand the result.
    if part.raw_body().len() > MAX_BODY_BYTES {
        status.body_truncated = true;
        return None;
    }
    let (text, problems) = part.text_checked()?;
    status.degraded |= !problems.is_empty();
    let mut end = text.len().min(MAX_BODY_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    status.body_truncated |= end < text.len();
    Some(text[..end].to_owned())
}

/// Parse a bounded RFC 5322 message without fetching resources or executing HTML.
///
/// # Errors
/// Returns an error if the input exceeds the hard limit or cannot be parsed.
#[inline]
pub fn project(raw: &[u8]) -> Result<Projection, Error> {
    if raw.len() > MAX_INPUT_BYTES {
        return Err(Error::InputTooLarge);
    }
    let message = MessageParser::new()
        .max_parts(MAX_PARTS)
        .max_depth(32)
        .max_encoded_nesting(0)
        .parse(raw)
        .ok_or(Error::Unparseable)?;
    let revision = content_digest(raw);
    let mut status = Status::default();
    let headers = Headers {
        message_id: header(message.message_id(), &mut status.headers_truncated, false),
        references: message
            .references()
            .map_or_else(Vec::new, |values| identifiers(values.iter(), &mut status)),
        in_reply_to: message
            .in_reply_to()
            .map_or_else(Vec::new, |values| identifiers(values.iter(), &mut status)),
        from: addresses(message.from(), &mut status),
        reply_to: addresses(message.reply_to(), &mut status),
        to: addresses(message.to(), &mut status),
        cc: addresses(message.cc(), &mut status),
        bcc: addresses(message.bcc(), &mut status),
        subject: header(message.subject(), &mut status.headers_truncated, true),
        date: message.date().map(|date| date.to_timestamp()),
    };
    inspect(message.root_part(), &mut status);
    let mut attachments = Vec::new();
    for part in message.attachments() {
        if attachments.len() == MAX_ATTACHMENTS {
            status.attachments_truncated = true;
            break;
        }
        attachments.push(attachment(part, &mut status.attachment_metadata_truncated));
    }
    let (text, html, preview) = if status.encrypted_unsupported {
        (None, None, None)
    } else {
        let text = body(
            message
                .text_body()
                .find(|part| matches!(part.kind(), PartKind::Text)),
            &mut status,
        );
        let html = body(
            message
                .html_body()
                .find(|part| matches!(part.kind(), PartKind::Html)),
            &mut status,
        );
        let source = text
            .clone()
            .or_else(|| html.as_deref().map(mail_parser::html_to_text));
        let preview = source.map(|content| Preview {
            source: PreviewSource::Extracted,
            text: content
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(320)
                .collect(),
            body_revision: revision,
        });
        (text, html, preview)
    };
    Ok(Projection {
        original: raw.to_vec(),
        revision,
        headers,
        text,
        html,
        preview,
        attachments,
        status,
    })
}
