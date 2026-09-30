//! IMAP transport: the mockable trait the sync engine talks to, plus the
//! live implementation over `imap` 3.0.0-alpha.15 (rustls).
//!
//! All header/full fetches return parsed `RemoteMail` values (mailparse
//! does the MIME work; decode is lossy and never fails ingest).

use crate::email::{sanitize_imap_error, EmailError};

/// A folder as reported by LIST, with special-use attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteFolder {
    pub name: String,
    /// RFC 6154 special-use markers, e.g. "\\Sent", "\\Trash".
    pub attributes: Vec<String>,
}

/// A parsed message (headers from header sync, body from on-demand fetch).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteMail {
    pub uid: u32,
    pub flags: Vec<String>, // normalized: seen flagged answered draft
    pub size: u64,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub from_name: Option<String>,
    pub from_addr: String,
    pub to: Vec<(Option<String>, String)>,
    pub cc: Vec<(Option<String>, String)>,
    pub subject: String,
    /// RFC 2822 Date header as UTC ISO-8601 (falls back to now on parse failure).
    pub date: String,
    /// Full RFC822 bytes — present only on `fetch_full`.
    pub rfc822: Option<Vec<u8>>,
}

/// Result of selecting a folder: its UIDVALIDITY (resync detector).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FolderState {
    pub uidvalidity: Option<u32>,
}

/// The sync engine's view of an IMAP connection. Mocked in tests.
pub trait ImapTransport {
    fn capabilities(&mut self) -> Result<Vec<String>, EmailError>;
    fn list_folders(&mut self) -> Result<Vec<RemoteFolder>, EmailError>;
    fn select(&mut self, folder: &str) -> Result<FolderState, EmailError>;
    /// Header sync fetch for a UID range (inclusive bounds; `None` end =
    /// open-ended from `uid_start`).
    fn fetch_headers(
        &mut self,
        uid_start: u32,
        uid_end: Option<u32>,
    ) -> Result<Vec<RemoteMail>, EmailError>;
    /// Full-message fetch (BODY.PEEK[]).
    fn fetch_full(&mut self, uid: u32) -> Result<RemoteMail, EmailError>;
    /// All UIDs in the selected folder (for the initial-sync 500 cap).
    fn all_uids(&mut self) -> Result<Vec<u32>, EmailError>;
    /// Replace the flag set of `uid` with `flags` (UID STORE FLAGS.SILENT).
    fn store_flags(&mut self, uid: u32, flags: &[String]) -> Result<(), EmailError>;
    /// Move `uid` to the named folder — UID MOVE with COPY+DELETE fallback
    /// (decided once from server capabilities).
    fn move_mail(&mut self, uid: u32, target: &str) -> Result<(), EmailError>;
    /// APPEND an RFC822 message to a folder (used for Sent copies).
    fn append(&mut self, folder: &str, content: &[u8], flags: &[String]) -> Result<(), EmailError>;
    fn noop(&mut self) -> Result<(), EmailError>;
    fn logout(&mut self) -> Result<(), EmailError>;
}

/// Detect a folder's role from special-use attributes (RFC 6154) with
/// name fallbacks ("Sent Items", "Spam", …). Pure function — unit-tested.
pub fn detect_folder_role(name: &str, attributes: &[String]) -> crate::email::model::FolderRole {
    use crate::email::model::FolderRole as R;
    let has = |needle: &str| {
        attributes
            .iter()
            .any(|a| a.to_lowercase().contains(&needle.to_lowercase()))
    };
    if has("inbox") || name.eq_ignore_ascii_case("inbox") {
        R::Inbox
    } else if has("sent")
        || name.eq_ignore_ascii_case("sent")
        || name.eq_ignore_ascii_case("sent items")
    {
        R::Sent
    } else if has("drafts")
        || name.eq_ignore_ascii_case("drafts")
        || name.eq_ignore_ascii_case("draft")
    {
        R::Drafts
    } else if has("trash")
        || name.eq_ignore_ascii_case("trash")
        || name.eq_ignore_ascii_case("deleted")
        || name.eq_ignore_ascii_case("deleted messages")
    {
        R::Trash
    } else if has("archive")
        || name.eq_ignore_ascii_case("archive")
        || name.eq_ignore_ascii_case("archives")
    {
        R::Archive
    } else if has("junk") || name.eq_ignore_ascii_case("junk") || name.eq_ignore_ascii_case("spam")
    {
        R::Junk
    } else {
        R::Other
    }
}

/// Normalize a user-entered host: trim whitespace and strip ONE trailing
/// ":<digits>" (users paste "imap.example.com:993" into the host field).
/// Bracketed IPv6 is preserved: `[::1]:993` → `[::1]`, bare `[2001:db8::1]`
/// untouched. The port field always wins over an embedded port.
pub fn normalize_host(host: &str) -> String {
    let trimmed = host.trim();
    if trimmed.starts_with('[') {
        // Bracketed IPv6: only strip a port after the closing bracket.
        if let Some(close) = trimmed.find(']') {
            let rest = &trimmed[close + 1..];
            if let Some(port) = rest.strip_prefix(':') {
                if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
                    return trimmed[..=close].to_string();
                }
            }
        }
        return trimmed.to_string();
    }
    match trimmed.rsplit_once(':') {
        Some((h, port))
            if !h.is_empty() && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            h.to_string()
        }
        _ => trimmed.to_string(),
    }
}

/// Live IMAP session over TLS (rustls).
pub struct LiveImap {
    session: imap::Session<imap::Connection>,
    uid_move_supported: bool,
    move_fallback_checked: bool,
}

impl LiveImap {
    /// Connect + LOGIN. Password is held in memory only for the session.
    pub fn connect(
        host: &str,
        port: u16,
        username: &str,
        password: &str,
    ) -> Result<Self, EmailError> {
        let host = normalize_host(host);
        let client = imap::ClientBuilder::new(&host, port)
            .connect()
            .map_err(|e| EmailError::Imap(format!("connect: {}", sanitize_imap_error(&e))))?;
        let session = client
            .login(username, password)
            .map_err(|(e, _client)| EmailError::Imap(sanitize_imap_error(&e)))?;
        Ok(LiveImap {
            session,
            uid_move_supported: false,
            move_fallback_checked: false,
        })
    }

    fn imap<T>(result: imap::Result<T>) -> Result<T, EmailError> {
        result.map_err(|e| EmailError::Imap(sanitize_imap_error(&e)))
    }

    /// Decide UID MOVE vs COPY+DELETE once per connection.
    fn ensure_move_strategy(&mut self) -> Result<(), EmailError> {
        if self.move_fallback_checked {
            return Ok(());
        }
        let caps = self.capabilities()?;
        self.uid_move_supported = caps.iter().any(|c| c.eq_ignore_ascii_case("MOVE"));
        self.move_fallback_checked = true;
        Ok(())
    }
}

impl ImapTransport for LiveImap {
    fn capabilities(&mut self) -> Result<Vec<String>, EmailError> {
        let caps = Self::imap(self.session.capabilities())?;
        Ok(caps.iter().map(|c| format!("{c:?}")).collect())
    }

    fn list_folders(&mut self) -> Result<Vec<RemoteFolder>, EmailError> {
        let names = Self::imap(self.session.list(Some(""), Some("*")))?;
        Ok(names
            .iter()
            .map(|n| RemoteFolder {
                name: n.name().to_string(),
                attributes: n.attributes().iter().map(|a| format!("{a:?}")).collect(),
            })
            .collect())
    }

    fn select(&mut self, folder: &str) -> Result<FolderState, EmailError> {
        let mailbox = Self::imap(self.session.select(folder))?;
        Ok(FolderState {
            uidvalidity: mailbox.uid_validity,
        })
    }

    fn fetch_headers(
        &mut self,
        uid_start: u32,
        uid_end: Option<u32>,
    ) -> Result<Vec<RemoteMail>, EmailError> {
        let query = "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])";
        let set = match uid_end {
            Some(end) => format!("{uid_start}:{end}"),
            None => format!("{uid_start}:*"),
        };
        let fetches = Self::imap(self.session.uid_fetch(set, query))?;
        Ok(fetches
            .iter()
            .filter_map(|f| parse_fetch(f, false))
            .collect())
    }

    fn fetch_full(&mut self, uid: u32) -> Result<RemoteMail, EmailError> {
        let query = "(UID FLAGS RFC822.SIZE BODY.PEEK[])";
        let fetches = Self::imap(self.session.uid_fetch(uid.to_string(), query))?;
        let fetch = fetches
            .iter()
            .next()
            .ok_or_else(|| EmailError::Imap("empty fetch".to_string()))?;
        parse_fetch(fetch, true).ok_or_else(|| EmailError::Imap("parse failed".to_string()))
    }

    fn all_uids(&mut self) -> Result<Vec<u32>, EmailError> {
        let fetches = Self::imap(self.session.uid_fetch("1:*", "UID"))?;
        Ok(fetches.iter().filter_map(|f| f.uid).collect())
    }

    fn store_flags(&mut self, uid: u32, flags: &[String]) -> Result<(), EmailError> {
        let query = format!("FLAGS.SILENT ({})", imap_flag_names(flags).join(" "));
        Self::imap(self.session.uid_store(uid.to_string(), query))?;
        Ok(())
    }

    fn move_mail(&mut self, uid: u32, target: &str) -> Result<(), EmailError> {
        self.ensure_move_strategy()?;
        let set = uid.to_string();
        if self.uid_move_supported {
            // imap 3.0.0-alpha.15 names the MOVE command `uid_mv`.
            if self
                .session
                .uid_mv(&set, target)
                .map_err(|e| crate::email::EmailError::Imap(sanitize_imap_error(&e)))
                .is_ok()
            {
                return Ok(());
            }
        }
        Self::imap(self.session.uid_copy(&set, target))?;
        let _ = Self::imap(self.session.uid_store(&set, "+FLAGS.SILENT (\\Deleted)"));
        Self::imap(self.session.expunge())?;
        Ok(())
    }

    fn append(&mut self, folder: &str, content: &[u8], flags: &[String]) -> Result<(), EmailError> {
        let mut cmd = self.session.append(folder, content);
        use imap::types::Flag;
        for f in flags {
            match f.as_str() {
                "seen" => {
                    cmd.flag(Flag::Seen);
                }
                "flagged" => {
                    cmd.flag(Flag::Flagged);
                }
                "answered" => {
                    cmd.flag(Flag::Answered);
                }
                "draft" => {
                    cmd.flag(Flag::Draft);
                }
                _ => {}
            }
        }
        Self::imap(cmd.finish())?;
        Ok(())
    }

    fn noop(&mut self) -> Result<(), EmailError> {
        Self::imap(self.session.noop())
    }

    fn logout(&mut self) -> Result<(), EmailError> {
        let _ = Self::imap(self.session.logout());
        Ok(())
    }
}

/// Map normalized flag names ("seen"…) to IMAP system-flag spellings.
fn imap_flag_names(flags: &[String]) -> Vec<String> {
    flags
        .iter()
        .map(|f| match f.as_str() {
            "seen" => "\\Seen".to_string(),
            "flagged" => "\\Flagged".to_string(),
            "answered" => "\\Answered".to_string(),
            "draft" => "\\Draft".to_string(),
            other => other.to_string(),
        })
        .collect()
}

/// Map an imap Fetch (headers or full) into a RemoteMail via mailparse.
fn parse_fetch(fetch: &imap::types::Fetch, want_body: bool) -> Option<RemoteMail> {
    use imap::types::Flag;
    use mailparse::MailHeaderMap;
    let uid = fetch.uid?;
    let flags: Vec<String> = fetch
        .flags()
        .iter()
        .filter_map(|f| match f {
            Flag::Seen => Some("seen"),
            Flag::Flagged => Some("flagged"),
            Flag::Answered => Some("answered"),
            Flag::Draft => Some("draft"),
            _ => None,
        })
        .map(str::to_string)
        .collect();
    let size = u64::from(fetch.size.unwrap_or(0));

    let bytes: &[u8] = if want_body {
        fetch.body()?
    } else {
        fetch.header()?
    };
    let parsed = mailparse::parse_mail(bytes).ok()?;
    let headers = parsed.get_headers();

    let decode = |h: &str| -> Option<String> {
        headers
            .get_first_header(h)
            .map(|hdr| hdr.get_value().trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let decode_lossy = |h: &str| decode(h).unwrap_or_default();

    let date = decode("Date")
        .and_then(|d| parse_rfc2822_date(&d))
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    let (from_name, from_addr) = headers
        .get_first_header("From")
        .map(|h| h.get_value())
        .map(|v| parse_address_list(&v))
        .and_then(|mut a| {
            if a.is_empty() {
                None
            } else {
                Some(a.remove(0))
            }
        })
        .unwrap_or((None, String::new()));

    let to = headers
        .get_first_header("To")
        .map(|h| h.get_value())
        .map(|v| parse_address_list(&v))
        .unwrap_or_default();
    let cc = headers
        .get_first_header("Cc")
        .map(|h| h.get_value())
        .map(|v| parse_address_list(&v))
        .unwrap_or_default();

    let references = decode("References")
        .map(|r| {
            r.split_whitespace()
                .map(str::to_string)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    Some(RemoteMail {
        uid,
        flags,
        size,
        message_id: decode("Message-ID"),
        in_reply_to: decode("In-Reply-To"),
        references,
        from_name,
        from_addr,
        to,
        cc,
        subject: decode_lossy("Subject"),
        date,
        rfc822: if want_body {
            Some(bytes.to_vec())
        } else {
            None
        },
    })
}

/// Parse an address header value "Name <addr>, addr2" into (name, addr) pairs.
fn parse_address_list(value: &str) -> Vec<(Option<String>, String)> {
    use mailparse::MailAddr;
    mailparse::addrparse(value)
        .map(|addrs| {
            addrs
                .iter()
                .filter_map(|a| match a {
                    MailAddr::Single(s) => Some((s.display_name.clone(), s.addr.clone())),
                    // Groups (rare) contribute their members.
                    MailAddr::Group(_) => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_rfc2822_date(value: &str) -> Option<String> {
    // mailparse's date parsing is lossy; chrono's RFC2822 handles the common
    // forms. Broken dates fall back to "now" at the caller.
    chrono::DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc).to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::normalize_host;

    #[test]
    fn normalize_host_strips_embedded_port() {
        assert_eq!(
            normalize_host("imap.purelymail.com:993"),
            "imap.purelymail.com"
        );
        assert_eq!(
            normalize_host("smtp.purelymail.com:465"),
            "smtp.purelymail.com"
        );
        assert_eq!(
            normalize_host("imap.purelymail.com:143"),
            "imap.purelymail.com"
        );
    }

    #[test]
    fn normalize_host_bare_and_whitespace() {
        assert_eq!(normalize_host("imap.purelymail.com"), "imap.purelymail.com");
        assert_eq!(
            normalize_host("  imap.purelymail.com  "),
            "imap.purelymail.com"
        );
        assert_eq!(
            normalize_host("\timap.purelymail.com\t"),
            "imap.purelymail.com"
        );
    }

    #[test]
    fn normalize_host_ipv6_brackets_preserved() {
        // Bracketed IPv6 with a port: strip the port, keep the brackets.
        assert_eq!(normalize_host("[::1]:993"), "[::1]");
        assert_eq!(normalize_host("[2001:db8::1]:465"), "[2001:db8::1]");
        // Bare bracketed IPv6 has no trailing port to strip.
        assert_eq!(normalize_host("[2001:db8::1]"), "[2001:db8::1]");
        assert_eq!(normalize_host("[::1]"), "[::1]");
    }

    #[test]
    fn normalize_host_non_numeric_ports_left_alone() {
        assert_eq!(
            normalize_host("imap.example.com:abc"),
            "imap.example.com:abc"
        );
        // Only ONE trailing ":digits" is stripped.
        assert_eq!(normalize_host("host:993:993"), "host:993");
        // Empty host/port parts are not stripped.
        assert_eq!(normalize_host(":993"), ":993");
        assert_eq!(normalize_host("host:"), "host:");
    }

    #[test]
    fn normalize_host_empty() {
        assert_eq!(normalize_host(""), "");
        assert_eq!(normalize_host("   "), "");
    }
}
